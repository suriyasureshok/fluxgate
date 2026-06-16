//! Fluxgate Edge Gateway - Main Entrypoint
//!
//! Orchestrates the high-performance Axum web servers, binds the TLS 1.3
//! certificates, and wires the internal application states for both the
//! public Data Plane and the private Control Plane.

use axum::{
    Json, Router,
    extract::{ConnectInfo, State},
    middleware::from_fn_with_state,
    routing::post,
    response::Html,
};
use axum_server::tls_rustls::RustlsConfig;
use serde::Deserialize;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::signal;

mod doh;
mod proxy;
mod rate_limiter;

// --- Configuration Constants ---
const DEFAULT_DOWNSTREAM_URL: &str = "https://api.openai.com";
const PUBLIC_PORT: u16 = 8443;
const INTERNAL_ADMIN_PORT: u16 = 9090;

/// Payload structure for the internal MCP control plane to update IP rules dynamically.
#[derive(Deserialize)]
struct AdminUpdateLimitRequest {
    ip: IpAddr,
    capacity: f64,
    refill_rate: f64,
}

/// Internal handler allowing the Python agent to reconfigure the Token Bucket in real-time.
async fn handle_admin_update_limit(
    State(state): State<Arc<rate_limiter::RateLimiterState>>,
    Json(payload): Json<AdminUpdateLimitRequest>,
) -> &'static str {
    tracing::info!(
        "Control Plane invoked rate limit update for IP: {}",
        payload.ip
    );
    state
        .update_rules(payload.ip, payload.capacity, payload.refill_rate)
        .await;
    "Gateway rules successfully updated.\n"
}

const DEFAULT_CERT_NAME: &str = "api.fluxgate.local+2.pem";
const DEFAULT_KEY_NAME: &str = "api.fluxgate.local+2-key.pem";

#[tokio::main]
async fn main() {
    // 1. Initialize structured telemetry
    tracing_subscriber::fmt::init();
    tracing::info!("Initializing Fluxgate Edge Gateway Engine...");

    // 2. Resolve Environment Configuration
    let downstream_url =
        std::env::var("DOWNSTREAM_AI_URL").unwrap_or_else(|_| DEFAULT_DOWNSTREAM_URL.to_string());

    // Standardize certificate file names for deployment (rename your mkcert files to match these)
    let cert_name =
        std::env::var("TLS_CERT_NAME").unwrap_or_else(|_| DEFAULT_CERT_NAME.to_string());
    let key_name = std::env::var("TLS_KEY_NAME").unwrap_or_else(|_| DEFAULT_KEY_NAME.to_string());

    let cert_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("certs")
        .join(cert_name);
    let key_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("certs")
        .join(key_name);

    // 3. Instantiate Shared Thread-Safe States
    let rate_limiter_state = Arc::new(rate_limiter::RateLimiterState::new(5000.0, 500.0));
    let proxy_state = Arc::new(proxy::ProxyState {
        client: reqwest::Client::new(),
        downstream_url,
    });

    // 4. Build the Public Data Plane Router (TLS)
    let public_app = Router::new()
        .route("/", axum::routing::get(|| async { Html(include_str!("index.html")) }))
        .route("/dns-query", post(doh::handle_dns_query))
        .route(
            "/v1/*path",
            post(proxy::handle_proxy_request)
                .with_state(proxy_state)
                .layer(from_fn_with_state(
                    rate_limiter_state.clone(),
                    |state, ConnectInfo(addr): ConnectInfo<SocketAddr>, req, next| {
                        rate_limiter::semantic_rate_limiter(state, addr.ip(), req, next)
                    },
                )),
        );

    // 5. Build the Private Control Plane Router (Plaintext HTTP, strictly Localhost)
    let admin_app = Router::new()
        .route("/admin/rate_limit", post(handle_admin_update_limit))
        .with_state(rate_limiter_state.clone());

    // 6. Spawn the Admin Server as a background Tokio task
    tokio::spawn(async move {
        let admin_addr = SocketAddr::from(([127, 0, 0, 1], INTERNAL_ADMIN_PORT));
        let listener = tokio::net::TcpListener::bind(admin_addr).await.unwrap();
        tracing::info!(
            "Fluxgate Internal Control Plane listening on http://{}",
            admin_addr
        );
        axum::serve(listener, admin_app).await.unwrap();
    });

    // 7. Load Cryptographic Material and Bind Public Interface
    let tls_config = RustlsConfig::from_pem_file(&cert_path, &key_path)
        .await
        .expect("CRITICAL: Failed to load TLS cryptographic material. Ensure cert.pem and key.pem exist in /certs.");

    let public_addr = SocketAddr::from(([0, 0, 0, 0], PUBLIC_PORT));
    tracing::info!(
        "Fluxgate Public Data Plane running securely at https://{}",
        public_addr
    );

    // 8. Start Public Server with Graceful Shutdown Hook
    axum_server::bind_rustls(public_addr, tls_config)
        .handle(shutdown_signal()) // Attach the shutdown interceptor
        .serve(public_app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .unwrap();
}

/// Creates an asynchronous listener that waits for OS termination signals.
/// Ensures active network streams complete before the Axum server drops the socket.
fn shutdown_signal() -> axum_server::Handle {
    let handle = axum_server::Handle::new();
    let spawn_handle = handle.clone();

    tokio::spawn(async move {
        let ctrl_c = async {
            signal::ctrl_c()
                .await
                .expect("Failed to install Ctrl+C handler");
        };

        #[cfg(unix)]
        let terminate = async {
            signal::unix::signal(signal::unix::SignalKind::terminate())
                .expect("Failed to install signal handler")
                .recv()
                .await;
        };

        #[cfg(not(unix))]
        let terminate = std::future::pending::<()>();

        tokio::select! {
            _ = ctrl_c => {},
            _ = terminate => {},
        }

        tracing::warn!(
            "Received termination signal. Draining connections and shutting down gracefully..."
        );
        spawn_handle.graceful_shutdown(Some(std::time::Duration::from_secs(10)));
    });

    handle
}
