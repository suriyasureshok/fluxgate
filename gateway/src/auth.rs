//! ---------------------------------------------------------------------------
//! Fluxgate Enterprise API Gateway
//! Module: Identity & Access Management (IAM) Guard
//! ---------------------------------------------------------------------------
//!
//! Intercepts incoming requests to the protected perimeter. Extracts the Bearer
//! token, hashes it securely using SHA-256, and cross-references it against
//! the PostgreSQL Single Source of Truth to validate tier limits and status.

use axum::{
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use sha2::{Digest, Sha256};
use sqlx::Row;
use std::sync::Arc;

use crate::infrastructure::GatewayState;

/// The Identity and Rule package injected into the request extension.
/// Contains pre-computed tier limits so the rate limiter does not need to query the database.
#[derive(Clone, Debug)]
pub struct AuthenticatedIdentity {
    pub key_prefix: String,
    pub key_hash: String, // Used as the Redis unique identifier
    pub capacity: f64,
    pub refill_rate: f64,
}

/// Core Axum Middleware: The perimeter security checkpoint.
pub async fn auth_guard(
    State(state): State<Arc<GatewayState>>,
    headers: HeaderMap,
    mut request: Request,
    next: Next,
) -> Response {
    // 1. Extract the Authorization Header
    let auth_header = match headers.get("Authorization") {
        Some(header) => header.to_str().unwrap_or(""),
        None => return (StatusCode::UNAUTHORIZED, "Missing API Key").into_response(),
    };

    if !auth_header.starts_with("Bearer ") {
        return (StatusCode::UNAUTHORIZED, "Malformed Bearer Token format").into_response();
    }

    let token = auth_header.trim_start_matches("Bearer ").trim();

    // 2. Cryptographic Hashing (Zero-Knowledge Validation)
    // SECURITY: We never pass plain-text keys to the database to prevent exposure via SQL logs.
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    let key_hash = hex::encode(hasher.finalize());

    // 3. PostgreSQL Identity Verification
    // RUNTIME QUERY: Avoids sqlx macro cache locks for dynamic CI/CD builds.
    let record = sqlx::query(
        r#"
        SELECT 
            k.key_prefix, 
            k.status, 
            t.token_capacity, 
            t.refill_rate
        FROM api_keys k
        JOIN users u ON k.user_id = u.id
        JOIN tiers t ON u.tier_id = t.id
        WHERE k.key_hash = $1
        "#,
    )
    .bind(&key_hash)
    .fetch_optional(&state.db_pool)
    .await;

    // 4. Authorization & State Injection
    match record {
        Ok(Some(row)) => {
            let status: Option<String> = row.get("status");

            // Check for suspension or revocation
            if status.as_deref() != Some("active") {
                tracing::warn!(
                    "Blocked access attempt using inactive API key prefix: {}",
                    row.get::<String, _>("key_prefix")
                );
                return (StatusCode::FORBIDDEN, "API Key Revoked or Suspended").into_response();
            }

            // Construct the secure context
            let identity = AuthenticatedIdentity {
                key_prefix: row.get("key_prefix"),
                key_hash,
                capacity: row.get("token_capacity"),
                refill_rate: row.get("refill_rate"),
            };

            let pool_clone = state.db_pool.clone();
            let hash_clone = identity.key_hash.clone();

            // Inject identity into the Axum request lifecycle
            request.extensions_mut().insert(identity);

            // 5. Asynchronous Telemetry
            // Fire-and-forget update for last active timestamp (does not block the request)
            tokio::spawn(async move {
                let _ = sqlx::query("UPDATE api_keys SET last_used_at = NOW() WHERE key_hash = $1")
                    .bind(hash_clone)
                    .execute(&pool_clone)
                    .await;
            });

            // Pass execution to the next layer (Rate Limiter)
            next.run(request).await
        }
        Ok(None) => (StatusCode::UNAUTHORIZED, "Invalid API Key").into_response(),
        Err(e) => {
            tracing::error!("CRITICAL: PostgreSQL Auth Query Failed: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR).into_response()
        }
    }
}
