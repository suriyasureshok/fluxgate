//! ---------------------------------------------------------------------------
//! Fluxgate Enterprise API Gateway
//! Module: Main Orchestrator & Entrypoint
//! ---------------------------------------------------------------------------

use axum::http::{
    Method,
    header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE},
};
use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use axum_server::tls_rustls::RustlsConfig;
use redis::AsyncCommands;
use serde::Deserialize;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::signal;
use tower_http::cors::CorsLayer;

// Internal Modules
pub mod admin;
pub mod auth;
pub mod doh;
pub mod infrastructure;
pub mod portal_auth;
pub mod proxy;
pub mod rate_limiter;

use crate::doh::DnsProvider;
use crate::doh::local::LocalClusterProvider;
use infrastructure::GatewayState;

#[derive(Deserialize, Debug)]
struct AdminUpdateLimitRequest {
    pub identifier: String,
    pub capacity: f64,
    pub refill_rate: f64,
}

/// Control Plane Handler: The Penalty Box Override
async fn handle_admin_update_limit(
    State(state): State<Arc<GatewayState>>,
    Json(payload): Json<AdminUpdateLimitRequest>,
) -> (axum::http::StatusCode, &'static str) {
    tracing::warn!(
        "Control Plane Override for Identity: {}",
        payload.identifier
    );

    // Hardened pool acquisition with timeout
    let mut con =
        match tokio::time::timeout(Duration::from_millis(100), state.redis_pool.get()).await {
            Ok(Ok(c)) => c,
            _ => {
                tracing::error!("Redis pool exhausted during Admin override.");
                return (
                    axum::http::StatusCode::SERVICE_UNAVAILABLE,
                    "Control Plane Offline",
                );
            }
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
            "Gateway rules overridden in Cache.\n",
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

    let mut scan_con =
        match tokio::time::timeout(Duration::from_millis(100), state.redis_pool.get()).await {
            Ok(Ok(c)) => c,
            _ => return Json(snapshot), // Return empty metrics rather than hanging the AI agent
        };

    let mut fetch_con =
        match tokio::time::timeout(Duration::from_millis(100), state.redis_pool.get()).await {
            Ok(Ok(c)) => c,
            _ => return Json(snapshot), // Return empty metrics rather than hanging the AI agent
        };

    if let Ok(mut iter) = scan_con
        .scan_match::<_, String>("fluxgate:rate_limit:*")
        .await
    {
        while let Some(key) = iter.next_item().await {
            if let Ok(tokens) = fetch_con.hget::<&str, &str, f64>(&key, "tokens").await {
                let identifier = key.replace("fluxgate:rate_limit:", "");
                snapshot.insert(identifier, tokens);
            }
        }
    }

    Json(snapshot)
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    tracing::info!("Booting Fluxgate Edge Gateway Engine...");

    let redis_url = std::env::var("REDIS_URL").expect("Missing REDIS_URL");
    let database_url = std::env::var("DATABASE_URL").expect("Missing DATABASE_URL");

    let cert_path = PathBuf::from(std::env::var("TLS_CERT_PATH").expect("Missing TLS_CERT_PATH"));
    let key_path = PathBuf::from(std::env::var("TLS_KEY_PATH").expect("Missing TLS_KEY_PATH"));

    let public_port: u16 = std::env::var("PUBLIC_PORT")
        .unwrap_or_else(|_| "8443".to_string())
        .parse()
        .unwrap();
    let admin_port: u16 = std::env::var("ADMIN_PORT")
        .unwrap_or_else(|_| "9090".to_string())
        .parse()
        .unwrap();

    let downstream_urls: Vec<String> = std::env::var("DOWNSTREAM_AI_URLS")
        .expect("Missing DOWNSTREAM_AI_URLS")
        .split(',')
        .map(|s| s.trim().to_string())
        .collect();

    let cluster_domain =
        std::env::var("CLUSTER_DOMAIN").unwrap_or_else(|_| "api.fluxgate.local".to_string());

    // 1. Initialize Distributed State
    let gateway_state =
        Arc::new(infrastructure::GatewayState::initialize(&database_url, &redis_url).await);
    let proxy_state = Arc::new(proxy::ProxyState::new(downstream_urls));

    // 2. Wire up dynamic DoH provider (Pass a raw client for the background worker)
    let doh_redis_client =
        redis::Client::open(redis_url.clone()).expect("Failed to build DoH Redis Client");
    let primary_provider: Box<dyn DnsProvider> =
        Box::new(LocalClusterProvider::new(&cluster_domain, doh_redis_client));

    let doh_state = Arc::new(doh::DohState {
        primary_provider,
        fallback_provider: None,
    });

    // 3. Build Axum Routers
    let public_routes = Router::new()
        .route(
            "/dns-query",
            post(doh::handle_dns_query).with_state(doh_state),
        )
        .route("/health", get(|| async { "Gateway Online" }));

    // Portal/Frontend Routes (Publicly accessible, but handles its own session validation internally)
    let portal_routes = Router::new()
        .route("/portal/register", post(portal_auth::register))
        .route("/portal/login", post(portal_auth::login))
        .route("/portal/logout", post(portal_auth::logout))
        .route("/portal/me", get(portal_auth::get_me))
        .with_state(gateway_state.clone());

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

    // CORS Layer
    let cors = CorsLayer::new()
        // Explicitly allow your frontend origins (Update these if your local server uses a different port)
        .allow_origin([
            "http://localhost:5500".parse().unwrap(),
            "http://127.0.0.1:5500".parse().unwrap(),
            "http://localhost:3000".parse().unwrap(),
        ])
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers([CONTENT_TYPE, AUTHORIZATION, ACCEPT])
        .allow_credentials(true);

    let admin_app = Router::new()
        .route("/admin/keys/generate", post(admin::generate_api_key))
        .route("/admin/rate_limit", post(handle_admin_update_limit))
        .route("/admin/metrics", get(handle_admin_metrics))
        .with_state(gateway_state.clone())
        .layer(cors.clone());

    let public_app = public_routes
        .merge(protected_routes)
        .merge(portal_routes)
        .layer(cors);

    // 4. Spawn Control Plane
    tokio::spawn(async move {
        let admin_addr = SocketAddr::from(([0, 0, 0, 0], admin_port));
        let listener = tokio::net::TcpListener::bind(admin_addr).await.unwrap();
        tracing::info!("Internal Control Plane listening on http://{}", admin_addr);
        axum::serve(listener, admin_app).await.unwrap();
    });

    // 5. Start TLS Data Plane
    let tls_config = RustlsConfig::from_pem_file(&cert_path, &key_path)
        .await
        .unwrap();
    let public_addr = SocketAddr::from(([0, 0, 0, 0], public_port));
    tracing::info!(
        "Public Data Plane running securely at https://{}",
        public_addr
    );

    axum_server::bind_rustls(public_addr, tls_config)
        .handle(shutdown_signal())
        .serve(public_app.into_make_service())
        .await
        .unwrap();
}

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

        tracing::warn!("Termination signal received. Draining connections...");
        spawn_handle.graceful_shutdown(Some(std::time::Duration::from_secs(10)));
    });

    handle
}
