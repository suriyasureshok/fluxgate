//! ---------------------------------------------------------------------------
//! Fluxgate Enterprise API Gateway
//! Module: Internal Control Plane (IAM & Telemetry)
//! ---------------------------------------------------------------------------

use axum::{Json, extract::State, http::StatusCode};
use rand::{Rng, distributions::Alphanumeric};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use uuid::Uuid;

use crate::infrastructure::GatewayState;

#[derive(Deserialize)]
pub struct GenerateKeyRequest {
    pub user_id: Uuid,
}

#[derive(Serialize)]
pub struct GenerateKeyResponse {
    pub user_id: Uuid,
    pub api_key: String,
    pub message: String,
}

/// Generates a secure API key, hashes it, stores the hash, and returns the cleartext.
pub async fn generate_api_key(
    State(state): State<Arc<GatewayState>>,
    Json(payload): Json<GenerateKeyRequest>,
) -> Result<Json<GenerateKeyResponse>, (StatusCode, String)> {
    // 1. Cryptographic Generation: 32 bytes of high-entropy randomness
    let secret: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(32)
        .map(char::from)
        .collect();

    let cleartext_key = format!("sk_live_{}", secret);
    let key_prefix: String = cleartext_key.chars().take(15).collect();

    // 2. Secure Hashing
    // ARCHITECTURE NOTE: We use SHA-256 here instead of Argon2 because the input
    // entropy (190+ bits) is mathematically immune to brute-force/rainbow table attacks.
    // This allows us to keep authentication ultra-fast without sacrificing security.
    let mut hasher = Sha256::new();
    hasher.update(cleartext_key.as_bytes());
    let key_hash = hex::encode(hasher.finalize());

    // 3. Execute the Insertion into Managed Postgres
    let result =
        sqlx::query("INSERT INTO api_keys (user_id, key_hash, key_prefix, status) VALUES ($1, $2, $3, 'active')")
            .bind(payload.user_id)
            .bind(&key_hash)
            .bind(&key_prefix)
            .execute(&state.db_pool)
            .await;

    match result {
        Ok(_) => Ok(Json(GenerateKeyResponse {
            user_id: payload.user_id,
            api_key: cleartext_key,
            message: "CRITICAL: Store this API key immediately. It cannot be recovered."
                .to_string(),
        })),
        Err(e) => {
            tracing::error!("Failed to provision credentials: {}", e);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to provision credentials".to_string(),
            ))
        }
    }
}
