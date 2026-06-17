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

// --- Data Structures ---

#[derive(Deserialize)]
pub struct GenerateKeyRequest {
    pub user_id: Uuid, // The UUID of the user from the `users` table
}

#[derive(Serialize)]
pub struct GenerateKeyResponse {
    pub user_id: Uuid,
    pub api_key: String, // The cleartext key (Only shown this one time)
    pub message: String,
}

// --- IAM Endpoints ---

/// Generates a secure API key, hashes it, stores the hash, and returns the cleartext.
pub async fn generate_api_key(
    State(state): State<Arc<GatewayState>>,
    Json(payload): Json<GenerateKeyRequest>,
) -> Result<Json<GenerateKeyResponse>, (StatusCode, String)> {
    // 1. Cryptographic Generation: Create 32 random bytes
    let secret: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(32)
        .map(char::from)
        .collect();

    // 2. Format the Key: Prepend our environment identifier
    let cleartext_key = format!("sk_live_{}", secret);

    // 3. Extract the UX Prefix: For the dashboard (e.g., "sk_live_abcde...")
    let key_prefix: String = cleartext_key.chars().take(15).collect();

    // 4. Secure Hashing: We only store the SHA-256 hash in Postgres
    let mut hasher = Sha256::new();
    hasher.update(cleartext_key.as_bytes());
    let key_hash = hex::encode(hasher.finalize());

    // 5. Execute the Insertion
    let result =
        sqlx::query("INSERT INTO api_keys (user_id, key_hash, key_prefix) VALUES ($1, $2, $3)")
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
            tracing::error!("Failed to generate API Key: {}", e);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to provision credentials".to_string(),
            ))
        }
    }
}
