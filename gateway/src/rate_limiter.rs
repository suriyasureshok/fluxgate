//! ---------------------------------------------------------------------------
//! Fluxgate Enterprise API Gateway
//! Module: Semantic Token Bucket Rate Limiter (Distributed via Redis)
//! ---------------------------------------------------------------------------
//!
//! This module implements a distributed rate limiter that calculates
//! payload limits dynamically based on estimated LLM token consumption rather
//! than simple HTTP request counting.
//!
//! Trust Boundary: Shifts from IP-based blocking to Application-Layer (API Key)
//! blocking, natively solving the enterprise NAT/Shared IP problem.
//!
//! By utilizing Redis as a centralized state store, this implementation supports
//! horizontal scaling of Gateway nodes while maintaining a single source of truth.

use axum::{
    body::Body,
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use redis::AsyncCommands;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::auth::AuthenticatedIdentity;
use crate::infrastructure::GatewayState;

/// The base cost of a transaction routing through the gateway (in tokens).
/// Accounts for HTTP overhead and base prompt framing.
const BASE_TRANSACTION_COST: f64 = 50.0;

/// The semantic heuristic ratio: rough character-to-token ratio used for payload estimation.
const CHARS_PER_TOKEN: f64 = 4.0;

/// Represents the structure of an incoming OpenAI-compatible chat completion request.
#[derive(Deserialize, Debug)]
struct ChatCompletionRequest {
    messages: Vec<ChatMessage>,
}

/// Represents an individual message within the chat completion request.
#[derive(Deserialize, Debug)]
struct ChatMessage {
    content: String,
}

/// Centralized state manager for the Rate Limiter, utilizing Redis for distributed memory.
pub struct RateLimiterState {
    pub redis_client: redis::Client,
    pub default_capacity: f64,
    pub default_refill_rate: f64,
}

impl RateLimiterState {
    /// Initializes a new Redis-backed rate limiter state.
    ///
    /// # Arguments
    /// * `redis_url` - The connection string for the Redis cluster.
    /// * `default_capacity` - The baseline token capacity for a new API key.
    /// * `default_refill_rate` - Tokens replenished per second.
    pub fn new(redis_url: &str, default_capacity: f64, default_refill_rate: f64) -> Self {
        let client = redis::Client::open(redis_url)
            .expect("CRITICAL: Failed to connect to Redis cache during initialization.");

        Self {
            redis_client: client,
            default_capacity,
            default_refill_rate,
        }
    }

    /// Evaluates token consumption against the distributed Redis state.
    /// Uses a mathematical replenishment formula based on elapsed time since the last request.
    ///
    /// # Arguments
    /// * `identifier` - The extracted API Key or JWT subject.
    /// * `tokens_needed` - The estimated token weight of the inbound payload.
    pub async fn check_and_consume(&self, identifier: &str, tokens_needed: f64) -> bool {
        let mut con = match self.redis_client.get_async_connection().await {
            Ok(c) => c,
            Err(e) => {
                // Failsafe mechanism: If Redis drops, we must deny traffic to protect the hardware
                tracing::error!(
                    "Redis connection failure: {}. Failsafe: Denying traffic.",
                    e
                );
                return false;
            }
        };

        // Namespace the key in Redis to avoid collisions
        let bucket_key = format!("fluxgate:rate_limit:{}", identifier);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();

        // 1. Fetch current state from Redis. If missing, initialize with defaults.
        // We store data as a Redis Hash: { tokens, last_refill, capacity, refill_rate }
        let (mut tokens, last_refill, capacity, refill_rate): (f64, f64, f64, f64) = redis::pipe()
            .hget(&bucket_key, "tokens")
            .hget(&bucket_key, "last_refill")
            .hget(&bucket_key, "capacity")
            .hget(&bucket_key, "refill_rate")
            .query_async(&mut con)
            .await
            .unwrap_or((
                self.default_capacity,
                now,
                self.default_capacity,
                self.default_refill_rate,
            ));

        // 2. Time-based replenishment math
        let elapsed = f64::max(0.0, now - last_refill);
        tokens = f64::min(capacity, tokens + (refill_rate * elapsed));

        // 3. Evaluate if the payload fits in the replenished bucket
        if tokens >= tokens_needed {
            let new_balance = tokens - tokens_needed;

            // Atomically update the Redis Hash with the new balance and timestamp
            let _: () = redis::pipe()
                .atomic()
                .hset(&bucket_key, "tokens", new_balance)
                .hset(&bucket_key, "last_refill", now)
                .hset(&bucket_key, "capacity", capacity)
                .hset(&bucket_key, "refill_rate", refill_rate)
                .expire(&bucket_key, 86400) // TTL: 24 hours to prevent memory leaks in Redis
                .query_async(&mut con)
                .await
                .unwrap_or(());

            true
        } else {
            // Write the replenished tokens back so the user doesn't lose their earned time,
            // but reject the transaction because it exceeds current physical capacity.
            let _: () = redis::pipe()
                .atomic()
                .hset(&bucket_key, "tokens", tokens)
                .hset(&bucket_key, "last_refill", now)
                .hset(&bucket_key, "capacity", capacity)
                .hset(&bucket_key, "refill_rate", refill_rate)
                .expire(&bucket_key, 86400)
                .query_async(&mut con)
                .await
                .unwrap_or(());

            false
        }
    }

    /// Dynamically overwrites the rate limit rules for a specific API Key in Redis.
    /// Designed to be invoked by the Python Control Plane (MCP) during anomaly response.
    pub async fn update_rules(&self, identifier: &str, new_capacity: f64, new_refill_rate: f64) {
        let mut con = match self.redis_client.get_async_connection().await {
            Ok(c) => c,
            Err(_) => return,
        };

        let bucket_key = format!("fluxgate:rate_limit:{}", identifier);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();

        // Enforce the new limits immediately, dropping current tokens to the new capacity
        let _: () = redis::pipe()
            .atomic()
            .hset(&bucket_key, "tokens", new_capacity)
            .hset(&bucket_key, "last_refill", now)
            .hset(&bucket_key, "capacity", new_capacity)
            .hset(&bucket_key, "refill_rate", new_refill_rate)
            .query_async(&mut con)
            .await
            .unwrap_or(());

        tracing::warn!(
            "Control Plane Override: Identity {} adjusted to Cap: {}, Rate: {}",
            identifier,
            new_capacity,
            new_refill_rate
        );
    }

    /// TELEMETRY: Safely snapshots the current token capacity of all active API Keys.
    /// Scans the Redis keyspace for active sessions to feed the Python algorithmic guard.
    pub async fn get_metrics_snapshot(&self) -> HashMap<String, f64> {
        let mut snapshot = HashMap::new();
        let mut con = match self.redis_client.get_async_connection().await {
            Ok(c) => c,
            Err(_) => return snapshot,
        };

        // Fetches all rate limit keys to provide a holistic view to the Control Plane.
        if let Ok(keys) = con.keys::<&str, Vec<String>>("fluxgate:rate_limit:*").await {
            for key in keys {
                if let Ok(tokens) = con.hget::<&str, &str, f64>(&key, "tokens").await {
                    let identifier = key.replace("fluxgate:rate_limit:", "");
                    snapshot.insert(identifier, tokens);
                }
            }
        }

        snapshot
    }
}

/// Helper function to estimate the token footprint of an incoming JSON payload.
/// Parses the OpenAI-compatible schema and applies the semantic heuristic ratio.
fn estimate_payload_tokens(bytes: &[u8]) -> f64 {
    let mut estimated_tokens = BASE_TRANSACTION_COST;

    if let Ok(payload) = serde_json::from_slice::<ChatCompletionRequest>(bytes) {
        let total_chars: usize = payload.messages.iter().map(|m| m.content.len()).sum();
        estimated_tokens += (total_chars as f64) / CHARS_PER_TOKEN;
    }

    estimated_tokens
}

/// Core Axum Middleware: The Semantic Limiter
///
/// Intercepts requests, extracts the authenticated identity, evaluates semantic weight
/// against Redis, enforces distributed throttling, and reconstructs the byte stream.
pub async fn semantic_rate_limiter(
    State(state): State<Arc<GatewayState>>,
    axum::extract::Extension(identity): axum::extract::Extension<AuthenticatedIdentity>,
    request: Request,
    next: Next,
) -> Response {
    let (parts, body) = request.into_parts();

    let bytes = match axum::body::to_bytes(body, usize::MAX).await {
        Ok(b) => b,
        Err(_) => return (StatusCode::BAD_REQUEST).into_response(),
    };

    let estimated_tokens = estimate_payload_tokens(&bytes);

    // --- CACHE-ASIDE REDIS LOGIC USING THE RULES FROM POSTGRES ---
    let mut con = match state.redis_client.get_async_connection().await {
        Ok(c) => c,
        Err(_) => return (StatusCode::SERVICE_UNAVAILABLE, "Rate Limiter Offline").into_response(),
    };

    let bucket_key = format!("fluxgate:rate_limit:{}", identity.key_hash);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();

    // Fetch current tokens. If missing, use the capacity we got from Postgres via Auth Guard!
    let (mut tokens, last_refill): (f64, f64) = redis::pipe()
        .hget(&bucket_key, "tokens")
        .hget(&bucket_key, "last_refill")
        .query_async(&mut con)
        .await
        .unwrap_or((identity.capacity, now));

    let elapsed = f64::max(0.0, now - last_refill);
    tokens = f64::min(identity.capacity, tokens + (identity.refill_rate * elapsed));

    if tokens >= estimated_tokens {
        let new_balance = tokens - estimated_tokens;

        let _: () = redis::pipe()
            .atomic()
            .hset(&bucket_key, "tokens", new_balance)
            .hset(&bucket_key, "last_refill", now)
            .expire(&bucket_key, 86400)
            .query_async(&mut con)
            .await
            .unwrap_or(());

        let mut reconstructed_request = Request::from_parts(parts, Body::from(bytes));
        reconstructed_request
            .extensions_mut()
            .insert(estimated_tokens);
        next.run(reconstructed_request).await.into_response()
    } else {
        tracing::warn!(
            "Rate limit breached for key prefix: {}",
            identity.key_prefix
        );
        (StatusCode::TOO_MANY_REQUESTS).into_response()
    }
}
