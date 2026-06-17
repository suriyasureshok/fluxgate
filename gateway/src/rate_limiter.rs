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
use serde::Deserialize;
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
/// atomically against Redis via Lua, enforces distributed throttling, and reconstructs the byte stream.
pub async fn semantic_rate_limiter(
    State(state): State<Arc<GatewayState>>,
    axum::extract::Extension(identity): axum::extract::Extension<AuthenticatedIdentity>,
    request: Request,
    next: Next,
) -> Response {
    let (parts, body) = request.into_parts();

    // 10 MiB limit - adjust based on max expected prompt size to prevent memory exhaustion
    const MAX_BODY_SIZE: usize = 10 * 1024 * 1024;
    let bytes = match axum::body::to_bytes(body, MAX_BODY_SIZE).await {
        Ok(b) => b,
        Err(_) => return (StatusCode::PAYLOAD_TOO_LARGE).into_response(),
    };

    let estimated_tokens = estimate_payload_tokens(&bytes);

    // --- ATOMIC CACHE-ASIDE REDIS LOGIC ---
    let mut con = match state.redis_client.get_async_connection().await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(
                "Redis connection failure: {}. Failsafe: Denying traffic.",
                e
            );
            return (StatusCode::SERVICE_UNAVAILABLE, "Rate Limiter Offline").into_response();
        }
    };

    let bucket_key = format!("fluxgate:rate_limit:{}", identity.key_hash);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();

    // Define the atomic Lua script for true concurrency safety
    const RATE_LIMIT_LUA: &str = r#"
        local tokens_needed = tonumber(ARGV[1])
        local now = tonumber(ARGV[2])
        local capacity = tonumber(ARGV[3])
        local refill_rate = tonumber(ARGV[4])

        local tokens = tonumber(redis.call('HGET', KEYS[1], 'tokens') or capacity)
        local last_refill = tonumber(redis.call('HGET', KEYS[1], 'last_refill') or now)

        local elapsed = math.max(0, now - last_refill)
        tokens = math.min(capacity, tokens + (refill_rate * elapsed))

        if tokens >= tokens_needed then
            tokens = tokens - tokens_needed
            redis.call('HSET', KEYS[1], 'tokens', tokens, 'last_refill', now, 'capacity', capacity, 'refill_rate', refill_rate)
            redis.call('EXPIRE', KEYS[1], 86400)
            return 1
        else
            redis.call('HSET', KEYS[1], 'tokens', tokens, 'last_refill', now, 'capacity', capacity, 'refill_rate', refill_rate)
            redis.call('EXPIRE', KEYS[1], 86400)
            return 0
        end
    "#;

    let script = redis::Script::new(RATE_LIMIT_LUA);

    // Invoke the script using the values injected by the PostgreSQL Auth layer
    let permitted: i32 = script
        .key(&bucket_key)
        .arg(estimated_tokens)
        .arg(now)
        .arg(identity.capacity)
        .arg(identity.refill_rate)
        .invoke_async(&mut con)
        .await
        .unwrap_or(0); // Fail-closed: Reject if Redis fails during script execution

    if permitted == 1 {
        // Request is allowed - Reconstruct the body and pass it to the proxy
        let mut reconstructed_request = Request::from_parts(parts, Body::from(bytes));
        reconstructed_request
            .extensions_mut()
            .insert(estimated_tokens); // Pass the cost down to telemetry if needed

        next.run(reconstructed_request).await.into_response()
    } else {
        // Request exceeded limits
        tracing::warn!(
            "Rate limit breached for key prefix: {}. Required: {}, Available limits enforced.",
            identity.key_prefix,
            estimated_tokens
        );
        (StatusCode::TOO_MANY_REQUESTS).into_response()
    }
}
