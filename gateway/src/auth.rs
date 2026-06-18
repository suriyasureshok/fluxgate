//! ---------------------------------------------------------------------------
//! Fluxgate Enterprise API Gateway
//! Module: Identity & Access Management (IAM) Guard
//! ---------------------------------------------------------------------------

use axum::{
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::Row;
use std::sync::Arc;
use std::time::Duration;

use crate::infrastructure::GatewayState;

/// The Identity package. Now derives Serialize/Deserialize to allow Redis caching.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuthenticatedIdentity {
    pub key_prefix: String,
    pub key_hash: String,
    pub capacity: f64,
    pub refill_rate: f64,
}

pub async fn auth_guard(
    State(state): State<Arc<GatewayState>>,
    headers: HeaderMap,
    mut request: Request,
    next: Next,
) -> Response {
    let auth_header = match headers.get("Authorization") {
        Some(header) => header.to_str().unwrap_or(""),
        None => return (StatusCode::UNAUTHORIZED, "Missing API Key").into_response(),
    };

    if !auth_header.starts_with("Bearer ") {
        return (StatusCode::UNAUTHORIZED, "Malformed Bearer Token").into_response();
    }

    let token = auth_header.trim_start_matches("Bearer ").trim();

    // 1. Cryptographic Hashing
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    let key_hash = hex::encode(hasher.finalize());

    // 2. Redis Fast-Path Cache Lookup (Protects Managed Postgres)
    let cache_key = format!("fluxgate:auth:{}", key_hash);

    // Acquire Redis connection with a strict 50ms timeout
    let mut redis_conn =
        match tokio::time::timeout(Duration::from_millis(50), state.redis_pool.get()).await {
            Ok(Ok(conn)) => conn,
            _ => {
                tracing::error!("Auth Guard: Redis pool timeout/exhaustion.");
                return (StatusCode::SERVICE_UNAVAILABLE, "Auth subsystem degraded")
                    .into_response();
            }
        };

    let cached_auth: Option<String> = redis_conn.get(&cache_key).await.unwrap_or(None);

    let identity = if let Some(cached_json) = cached_auth {
        // Cache Hit
        serde_json::from_str::<AuthenticatedIdentity>(&cached_json).unwrap()
    } else {
        // 3. Cache Miss: Query Managed Postgres (Cold Path)
        let record = sqlx::query(
            r#"
            SELECT k.key_prefix, k.status, t.token_capacity, t.refill_rate
            FROM api_keys k
            JOIN users u ON k.user_id = u.id
            JOIN tiers t ON u.tier_id = t.id
            WHERE k.key_hash = $1
            "#,
        )
        .bind(&key_hash)
        .fetch_optional(&state.db_pool)
        .await;

        match record {
            Ok(Some(row)) => {
                if row.get::<Option<String>, _>("status").as_deref() != Some("active") {
                    return (StatusCode::FORBIDDEN, "API Key Revoked").into_response();
                }

                let new_identity = AuthenticatedIdentity {
                    key_prefix: row.get("key_prefix"),
                    key_hash: key_hash.clone(),
                    capacity: row.get("token_capacity"),
                    refill_rate: row.get("refill_rate"),
                };

                // Warm the cache for subsequent requests (5 minute TTL)
                let serialized = serde_json::to_string(&new_identity).unwrap();
                let _: () = redis_conn
                    .set_ex(&cache_key, serialized, 300)
                    .await
                    .unwrap_or(());

                new_identity
            }
            Ok(None) => return (StatusCode::UNAUTHORIZED, "Invalid API Key").into_response(),
            Err(e) => {
                tracing::error!("PostgreSQL Auth Query Failed: {}", e);
                return (StatusCode::INTERNAL_SERVER_ERROR).into_response();
            }
        }
    };

    // 4. Telemetry Offloading (Update Redis instead of Postgres)
    let telemetry_key = format!("fluxgate:telemetry:last_used:{}", identity.key_hash);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    // Fire-and-forget to Redis. (Your control plane can ingest this later)
    tokio::spawn(async move {
        let _: () = redis_conn
            .set_ex(telemetry_key, now, 86400)
            .await
            .unwrap_or(());
    });

    request.extensions_mut().insert(identity);
    next.run(request).await
}
