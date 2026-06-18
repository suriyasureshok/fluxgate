//! ---------------------------------------------------------------------------
//! Fluxgate Enterprise API Gateway
//! Module: Developer Portal Authentication (Frontend Auth)
//! ---------------------------------------------------------------------------

use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};
use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

use crate::infrastructure::GatewayState;

#[derive(Deserialize)]
pub struct RegisterRequest {
    pub email: String,
    pub password: String,
}

#[derive(Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub message: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Claims {
    pub sub: Uuid,
    pub email: String,
    pub tier: String,
    pub exp: usize,
}

fn get_jwt_secret() -> String {
    std::env::var("JWT_SECRET").expect("JWT_SECRET environment variable must be set")
}

pub async fn register(
    State(state): State<Arc<GatewayState>>,
    Json(payload): Json<RegisterRequest>,
) -> Result<impl IntoResponse, (StatusCode, &'static str)> {
    let password = payload.password.clone();
    let hashed_password = tokio::task::spawn_blocking(move || {
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map(|hash| hash.to_string())
    })
    .await
    .unwrap()
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Hashing failed"))?;

    let result = sqlx::query(
        "INSERT INTO users (email, password_hash, tier_id) VALUES ($1, $2, 'free') RETURNING id",
    )
    .bind(&payload.email)
    .bind(&hashed_password)
    .fetch_one(&state.db_pool)
    .await;

    match result {
        Ok(_) => Ok((StatusCode::CREATED, "User registered successfully")),
        Err(_) => Err((StatusCode::CONFLICT, "Email already exists")),
    }
}

pub async fn login(
    State(state): State<Arc<GatewayState>>,
    Json(payload): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, (StatusCode, &'static str)> {
    let record = sqlx::query("SELECT id, password_hash, tier_id FROM users WHERE email = $1")
        .bind(&payload.email)
        .fetch_optional(&state.db_pool)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Database error"))?;

    let user = record.ok_or((StatusCode::UNAUTHORIZED, "Invalid credentials"))?;

    let user_id: Uuid = user.get("id");
    let password_hash: String = user.get("password_hash");
    let tier_id: Option<String> = user.get("tier_id");
    let password_attempt = payload.password.clone();

    let is_valid = tokio::task::spawn_blocking(move || {
        let parsed_hash = PasswordHash::new(&password_hash).unwrap();
        Argon2::default()
            .verify_password(password_attempt.as_bytes(), &parsed_hash)
            .is_ok()
    })
    .await
    .unwrap();

    if !is_valid {
        return Err((StatusCode::UNAUTHORIZED, "Invalid credentials"));
    }

    let expiration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as usize
        + (24 * 3600);
    let claims = Claims {
        sub: user_id,
        email: payload.email,
        tier: tier_id.unwrap_or_else(|| "free".to_string()),
        exp: expiration,
    };

    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(get_jwt_secret().as_bytes()),
    )
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Token generation failed"))?;

    // Return token in JSON body instead of a Cookie
    Ok(Json(LoginResponse {
        token,
        message: "Login successful".to_string(),
    }))
}

pub async fn get_me(headers: HeaderMap) -> Result<Json<Claims>, (StatusCode, &'static str)> {
    // Read from the Authorization header instead of Cookies
    let auth_header = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|val| val.to_str().ok())
        .ok_or((StatusCode::UNAUTHORIZED, "Missing Authorization header"))?;

    let token = auth_header.trim_start_matches("Bearer ").trim();

    let token_data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(get_jwt_secret().as_bytes()),
        &Validation::default(),
    )
    .map_err(|_| (StatusCode::UNAUTHORIZED, "Session expired or invalid"))?;

    Ok(Json(token_data.claims))
}

pub async fn logout() -> impl IntoResponse {
    (StatusCode::OK, "Logged out")
}
