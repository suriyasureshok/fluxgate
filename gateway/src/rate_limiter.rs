//! Semantic Token Bucket Rate Limiter
//!
//! This module implements a thread-safe, IP-based rate limiter that calculates
//! payload limits dynamically based on estimated LLM token consumption rather
//! than simple request counting.

use axum::{
    body::Body,
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;

/// The base cost of a transaction routing through the gateway (in tokens).
const BASE_TRANSACTION_COST: f64 = 50.0;

/// The rough character-to-token ratio used for semantic payload estimation.
const CHARS_PER_TOKEN: f64 = 4.0;

/// Represents an incoming OpenAI-compatible chat completion request.
#[derive(Deserialize)]
struct ChatCompletionRequest {
    messages: Vec<ChatMessage>,
}

/// Represents an individual message within the chat completion request.
#[derive(Deserialize)]
struct ChatMessage {
    content: String,
}

/// Represents an individual token bucket for a specific client.
struct TokenBucket {
    capacity: f64,
    refill_rate: f64,
    tokens: f64,
    last_refill: Instant,
}

impl TokenBucket {
    /// Creates a new TokenBucket initialized to maximum capacity.
    fn new(capacity: f64, refill_rate: f64) -> Self {
        Self {
            capacity,
            refill_rate,
            tokens: capacity,
            last_refill: Instant::now(),
        }
    }

    /// Evaluates the elapsed time, replenishes tokens mathematically,
    /// and attempts to consume the requested amount.
    ///
    /// Returns `true` if tokens were successfully consumed, `false` if the limit is breached.
    fn try_consume(&mut self, amount: f64) -> bool {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.last_refill = now;

        // Apply replenishment formula: T_current = min(Capacity, T_previous + Rate * Time)
        self.tokens = (self.capacity).min(self.tokens + (self.refill_rate * elapsed));

        if self.tokens >= amount {
            self.tokens -= amount;
            true
        } else {
            false
        }
    }
}

/// Global, thread-safe state manager tracking all active IP buckets.
pub struct RateLimiterState {
    buckets: Mutex<HashMap<IpAddr, TokenBucket>>,
    default_capacity: f64,
    default_refill_rate: f64,
}

impl RateLimiterState {
    /// Initializes a new rate limiter state with global default thresholds.
    pub fn new(capacity: f64, refill_rate: f64) -> Self {
        Self {
            buckets: Mutex::new(HashMap::new()),
            default_capacity: capacity,
            default_refill_rate: refill_rate,
        }
    }

    /// Dynamically overwrites the rate limit rules for a specific IP address.
    ///
    /// Designed to be invoked by the Control Plane (MCP) during anomaly detection.
    pub async fn update_rules(&self, ip: IpAddr, capacity: f64, refill_rate: f64) {
        let mut buckets = self.buckets.lock().await;
        buckets.insert(ip, TokenBucket::new(capacity, refill_rate));
        tracing::info!(
            "Updated limits for IP {}: Cap {}, Rate {}",
            ip,
            capacity,
            refill_rate
        );
    }
}

/// Helper function to estimate the token footprint of an incoming JSON payload.
fn estimate_payload_tokens(bytes: &[u8]) -> f64 {
    let mut estimated_tokens = BASE_TRANSACTION_COST;

    if let Ok(payload) = serde_json::from_slice::<ChatCompletionRequest>(bytes) {
        let total_chars: usize = payload.messages.iter().map(|m| m.content.len()).sum();
        estimated_tokens += (total_chars as f64) / CHARS_PER_TOKEN;
    }

    estimated_tokens
}

/// Axum middleware that reads the request, evaluates the semantic weight,
/// enforces the token bucket threshold, and transparently reconstructs the stream.
pub async fn semantic_rate_limiter(
    State(state): State<Arc<RateLimiterState>>,
    client_ip: IpAddr,
    request: Request,
    next: Next,
) -> Response {
    let (parts, body) = request.into_parts();

    // Buffer the raw bytes to prevent consuming the stream for downstream handlers
    let bytes = match axum::body::to_bytes(body, usize::MAX).await {
        Ok(b) => b,
        Err(e) => {
            tracing::error!("Failed to buffer request body: {}", e);
            return (StatusCode::BAD_REQUEST).into_response();
        }
    };

    let estimated_tokens = estimate_payload_tokens(&bytes);

    // CRITICAL SECTION: Lock the map, evaluate, and drop the lock immediately
    {
        let mut buckets = state.buckets.lock().await;
        let bucket = buckets
            .entry(client_ip)
            .or_insert_with(|| TokenBucket::new(state.default_capacity, state.default_refill_rate));

        if !bucket.try_consume(estimated_tokens) {
            tracing::warn!(
                "Rate limit breached for IP: {}. Deficit attempt: {} tokens.",
                client_ip,
                estimated_tokens
            );
            return (StatusCode::TOO_MANY_REQUESTS).into_response();
        }
    }

    // Transparently rebuild the request for the proxy layer
    let mut reconstructed_request = Request::from_parts(parts, Body::from(bytes));
    reconstructed_request
        .extensions_mut()
        .insert(estimated_tokens);

    next.run(reconstructed_request).await.into_response()
}
