//! ---------------------------------------------------------------------------
//! Fluxgate Enterprise API Gateway
//! Module: Main Orchestrator & Entrypoint
//! ---------------------------------------------------------------------------
//!
//! The absolute root of the Gateway execution. Responsible for reading system
//! environment variables, initializing distributed connection pools (Redis/Postgres),
//! constructing the Axum routers, and binding to the network interfaces.

use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use axum_server::tls_rustls::RustlsConfig;
use redis::AsyncCommands;
use serde::Deserialize;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::signal;

// Internal Modules
pub mod admin;
pub mod auth;
pub mod doh;
pub mod infrastructure;
pub mod proxy;
pub mod rate_limiter;

use infrastructure::GatewayState;

/// Payload structure for the internal Control Plane to update API Key rules dynamically.
#[derive(Deserialize, Debug)]
struct AdminUpdateLimitRequest {
    pub identifier: String, // The hashed API key or user ID
    pub capacity: f64,
    pub refill_rate: f64,
}

/// Control Plane Handler: The Penalty Box Override
async fn handle_admin_update_limit(
    State(state): State<Arc<GatewayState>>,
    Json(payload): Json<AdminUpdateLimitRequest>,
) -> (axum::http::StatusCode, &'static str) {
    tracing::warn!(
        "Control Plane invoked Override for Identity: {}",
        payload.identifier
    );

    let Ok(mut con) = state.redis_client.get_async_connection().await else {
        tracing::error!("Failed to connect to Redis for rate limit override");
        return (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "Redis unavailable",
        );
    };

    let bucket_key = format!("fluxgate:rate_limit:{}", payload.identifier);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();

    match redis::pipe()
        .atomic()
        .hset(&bucket_key, "tokens", payload.capacity)
        .hset(&bucket_key, "last_refill", now)
        .hset(&bucket_key, "capacity", payload.capacity)
        .hset(&bucket_key, "refill_rate", payload.refill_rate)
        .expire(&bucket_key, 86400)
        .query_async(&mut con)
        .await
    {
        Ok(()) => (
            axum::http::StatusCode::OK,
            "Gateway rules successfully overridden in Cache.\n",
        ),
        Err(e) => {
            tracing::error!("Redis pipeline failed: {e}");
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to update cache",
            )
        }
    }
}

/// Control Plane Handler: Telemetry Endpoint for the AI Analyzer
async fn handle_admin_metrics(
    State(state): State<Arc<GatewayState>>,
) -> Json<HashMap<String, f64>> {
    let mut snapshot = HashMap::new();
    if let Ok(mut con) = state.redis_client.get_async_connection().await {
        if let Ok(keys) = con.keys::<&str, Vec<String>>("fluxgate:rate_limit:*").await {
            for key in keys {
                if let Ok(tokens) = con.hget::<&str, &str, f64>(&key, "tokens").await {
                    let identifier = key.replace("fluxgate:rate_limit:", "");
                    snapshot.insert(identifier, tokens);
                }
            }
        }
    }
    Json(snapshot)
}

#[tokio::main]
async fn main() {
    // 1. Initialize Telemetry & Logging
    tracing_subscriber::fmt::init();
    tracing::info!("Booting Fluxgate Edge Gateway Engine...");

    // 2. Strict Environment Configuration (Fail-Fast Pattern)
    let redis_url = std::env::var("REDIS_URL")
        .expect("CRITICAL BOOT FAILURE: REDIS_URL environment variable is missing.");
    let database_url = std::env::var("DATABASE_URL")
        .expect("CRITICAL BOOT FAILURE: DATABASE_URL environment variable is missing.");

    let cert_path_str = std::env::var("TLS_CERT_PATH")
        .expect("CRITICAL BOOT FAILURE: TLS_CERT_PATH environment variable is missing.");
    let key_path_str = std::env::var("TLS_KEY_PATH")
        .expect("CRITICAL BOOT FAILURE: TLS_KEY_PATH environment variable is missing.");
    let cert_path = PathBuf::from(cert_path_str);
    let key_path = PathBuf::from(key_path_str);

    let public_port: u16 = std::env::var("PUBLIC_PORT")
        .unwrap_or_else(|_| "8443".to_string())
        .parse()
        .expect("CRITICAL BOOT FAILURE: PUBLIC_PORT must be a valid port number.");
    let admin_port: u16 = std::env::var("ADMIN_PORT")
        .unwrap_or_else(|_| "9090".to_string())
        .parse()
        .expect("CRITICAL BOOT FAILURE: ADMIN_PORT must be a valid port number.");

    let downstream_urls_str = std::env::var("DOWNSTREAM_AI_URLS")
        .expect("CRITICAL BOOT FAILURE: DOWNSTREAM_AI_URLS environment variable is missing.");
    let downstream_urls: Vec<String> = downstream_urls_str
        .split(',')
        .map(|s| s.trim().to_string())
        .collect();

    let cluster_ips_str = std::env::var("CLUSTER_IPS")
        .expect("CRITICAL BOOT FAILURE: CLUSTER_IPS environment variable is missing.");
    let cluster_ips: Vec<IpAddr> = cluster_ips_str
        .split(',')
        .map(|s| {
            let ip_str = s.trim();
            IpAddr::from_str(ip_str).unwrap_or_else(|_| {
                panic!(
                    "CRITICAL BOOT FAILURE: Invalid IP address in CLUSTER_IPS: '{}'",
                    ip_str
                );
            })
        })
        .collect();
    let cluster_domain =
        std::env::var("CLUSTER_DOMAIN").unwrap_or_else(|_| "api.fluxgate.local".to_string());

    // 3. Initialize the Distributed State Infrastructure
    let gateway_state =
        Arc::new(infrastructure::GatewayState::initialize(&database_url, &redis_url).await);
    let proxy_state = Arc::new(proxy::ProxyState::new(downstream_urls));

    let primary_provider = Box::new(doh::local::LocalClusterProvider::new(
        &cluster_domain,
        cluster_ips,
    ));
    let doh_state = Arc::new(doh::DohState {
        primary_provider,
        fallback_provider: None, // Can be injected later via env variable logic
    });

    // 4. Build the Segmented Axum Routers

    // Perimeter 1: Public Routes (No Auth required)
    let public_routes = Router::new()
        .route(
            "/dns-query",
            post(doh::handle_dns_query).with_state(doh_state),
        )
        .route("/health", get(|| async { "Gateway Online" }));

    // Perimeter 2: Protected AI Core (Requires Valid DB Key -> Rate Limit -> Proxy)
    let protected_routes = Router::new()
        .route(
            "/v1/*path",
            post(proxy::handle_proxy_request).with_state(proxy_state),
        )
        .layer(axum::middleware::from_fn_with_state(
            gateway_state.clone(),
            rate_limiter::semantic_rate_limiter,
        ))
        .layer(axum::middleware::from_fn_with_state(
            gateway_state.clone(),
            auth::auth_guard,
        ));

    // Merge the data plane perimeters
    let public_app = public_routes.merge(protected_routes);

    // 5. Build and Spawn the Internal Control Plane
    let admin_app = Router::new()
        .route("/admin/keys/generate", post(admin::generate_api_key))
        .route("/admin/rate_limit", post(handle_admin_update_limit))
        .route("/admin/metrics", get(handle_admin_metrics))
        .with_state(gateway_state.clone());

    tokio::spawn(async move {
        let admin_addr = SocketAddr::from(([0, 0, 0, 0], admin_port));
        let listener = tokio::net::TcpListener::bind(admin_addr).await.unwrap();
        tracing::info!(
            "Fluxgate Internal Control Plane listening securely on http://{}",
            admin_addr
        );
        axum::serve(listener, admin_app).await.unwrap();
    });

    // 6. Start the Public TLS Server
    let tls_config = RustlsConfig::from_pem_file(&cert_path, &key_path)
        .await
        .expect("CRITICAL BOOT FAILURE: Failed to load TLS Certificates.");

    let public_addr = SocketAddr::from(([0, 0, 0, 0], public_port));
    tracing::info!(
        "Fluxgate Public Data Plane running securely at https://{}",
        public_addr
    );

    axum_server::bind_rustls(public_addr, tls_config)
        .handle(shutdown_signal())
        .serve(public_app.into_make_service())
        .await
        .unwrap();
}

/// Graceful Shutdown Hook
/// Catches SIGTERM / SIGINT and allows active network requests to finish draining.
fn shutdown_signal() -> axum_server::Handle {
    let handle = axum_server::Handle::new();
    let spawn_handle = handle.clone();

    tokio::spawn(async move {
        let ctrl_c = async {
            signal::ctrl_c().await.unwrap();
        };
        #[cfg(unix)]
        let terminate = async {
            signal::unix::signal(signal::unix::SignalKind::terminate())
                .unwrap()
                .recv()
                .await;
        };
        #[cfg(not(unix))]
        let terminate = std::future::pending::<()>();

        tokio::select! { _ = ctrl_c => {}, _ = terminate => {}, }

        tracing::warn!("Termination signal received. Draining connections and shutting down...");
        spawn_handle.graceful_shutdown(Some(std::time::Duration::from_secs(10)));
    });

    handle
}
