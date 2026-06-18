//! ---------------------------------------------------------------------------
//! Fluxgate Enterprise API Gateway
//! Module: Semantic Token Bucket Rate Limiter
//! ---------------------------------------------------------------------------

use axum::{
    body::Body,
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::auth::AuthenticatedIdentity;
use crate::infrastructure::GatewayState;

const BASE_TRANSACTION_COST: f64 = 50.0;
const CHARS_PER_TOKEN: f64 = 4.0;

#[derive(Deserialize, Debug)]
struct ChatCompletionRequest {
    messages: Vec<ChatMessage>,
}

#[derive(Deserialize, Debug)]
struct ChatMessage {
    content: String,
}

fn estimate_payload_tokens(bytes: &[u8]) -> f64 {
    let mut estimated_tokens = BASE_TRANSACTION_COST;
    if let Ok(payload) = serde_json::from_slice::<ChatCompletionRequest>(bytes) {
        let total_chars: usize = payload.messages.iter().map(|m| m.content.len()).sum();
        estimated_tokens += (total_chars as f64) / CHARS_PER_TOKEN;
    }
    estimated_tokens
}

pub async fn semantic_rate_limiter(
    State(state): State<Arc<GatewayState>>,
    axum::extract::Extension(identity): axum::extract::Extension<AuthenticatedIdentity>,
    request: Request,
    next: Next,
) -> Response {
    let (parts, body) = request.into_parts();

    const MAX_BODY_SIZE: usize = 10 * 1024 * 1024;
    let bytes = match axum::body::to_bytes(body, MAX_BODY_SIZE).await {
        Ok(b) => b,
        Err(_) => return (StatusCode::PAYLOAD_TOO_LARGE).into_response(),
    };

    let estimated_tokens = estimate_payload_tokens(&bytes);

    // Strict 50ms acquisition timeout for the connection pool
    let mut con =
        match tokio::time::timeout(Duration::from_millis(50), state.redis_pool.get()).await {
            Ok(Ok(c)) => c,
            _ => {
                tracing::error!("Rate Limiter: Redis pool timeout. Denying traffic.");
                return (StatusCode::SERVICE_UNAVAILABLE, "Rate Limiter Offline").into_response();
            }
        };

    let bucket_key = format!("fluxgate:rate_limit:{}", identity.key_hash);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();

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
    let permitted: i32 = script
        .key(&bucket_key)
        .arg(estimated_tokens)
        .arg(now)
        .arg(identity.capacity)
        .arg(identity.refill_rate)
        .invoke_async(&mut con)
        .await
        .unwrap_or(0);

    if permitted == 1 {
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
