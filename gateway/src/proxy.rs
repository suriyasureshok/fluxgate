//! ---------------------------------------------------------------------------
//! Fluxgate Enterprise API Gateway
//! Module: Reverse Proxy & GPU Load Balancer
//! ---------------------------------------------------------------------------

use axum::{
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

pub struct ProxyState {
    pub client: reqwest::Client,
    pub downstream_urls: Vec<String>,
    current_index: AtomicUsize,
}

impl ProxyState {
    pub fn new(urls: Vec<String>) -> Self {
        // Hardened client configuration to prevent thread hanging
        let client = reqwest::Client::builder()
            .pool_idle_timeout(Duration::from_secs(90))
            .pool_max_idle_per_host(50) // Allow high concurrency to Ollama nodes
            .tcp_keepalive(Duration::from_secs(60))
            .build()
            .expect("CRITICAL: Failed to build Proxy HTTP Client");

        Self {
            client,
            downstream_urls: urls,
            current_index: AtomicUsize::new(0),
        }
    }

    pub fn get_next_target(&self) -> String {
        if self.downstream_urls.is_empty() {
            return String::new();
        }
        let index = self.current_index.fetch_add(1, Ordering::Relaxed);
        self.downstream_urls[index % self.downstream_urls.len()].clone()
    }
}

fn prepare_forward_headers(inbound_headers: &HeaderMap) -> reqwest::header::HeaderMap {
    let mut forward_headers = reqwest::header::HeaderMap::new();

    if let Some(auth) = inbound_headers.get(axum::http::header::AUTHORIZATION) {
        if let Ok(val) = reqwest::header::HeaderValue::from_bytes(auth.as_bytes()) {
            forward_headers.insert(reqwest::header::AUTHORIZATION, val);
        }
    }

    forward_headers.insert(
        reqwest::header::CONTENT_TYPE,
        reqwest::header::HeaderValue::from_static("application/json"),
    );

    forward_headers
}

pub async fn handle_proxy_request(
    State(state): State<Arc<ProxyState>>,
    Path(remaining_path): Path<String>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let base_url = state.get_next_target();
    if base_url.is_empty() {
        return (StatusCode::SERVICE_UNAVAILABLE, "No AI nodes available").into_response();
    }

    let target_url = format!("{}/v1/{}", base_url, remaining_path);
    let forward_headers = prepare_forward_headers(&headers);

    let proxy_request = match state
        .client
        .post(&target_url)
        .headers(forward_headers)
        .body(body)
        .build()
    {
        Ok(req) => req,
        Err(err) => {
            tracing::error!("Gateway Fault: Failed to compile request: {err}");
            return (StatusCode::INTERNAL_SERVER_ERROR).into_response();
        }
    };

    // Failsafe: 120-second hard timeout for downstream execution
    let execute_future = state.client.execute(proxy_request);
    match tokio::time::timeout(Duration::from_secs(120), execute_future).await {
        Ok(Ok(response)) => {
            let status = StatusCode::from_u16(response.status().as_u16())
                .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

            let mut response_headers = HeaderMap::new();
            if let Some(content_type) = response.headers().get(reqwest::header::CONTENT_TYPE) {
                response_headers.insert(axum::http::header::CONTENT_TYPE, content_type.clone());
            }

            if response
                .headers()
                .get(reqwest::header::TRANSFER_ENCODING)
                .is_some()
            {
                response_headers.insert(
                    axum::http::header::TRANSFER_ENCODING,
                    axum::http::header::HeaderValue::from_static("chunked"),
                );
            }

            let byte_stream = response.bytes_stream();
            let body = Body::from_stream(byte_stream);

            (status, response_headers, body).into_response()
        }
        Ok(Err(err)) => {
            tracing::error!("Network route failure: {err}");
            (StatusCode::BAD_GATEWAY, "Downstream service unreachable").into_response()
        }
        Err(_) => {
            tracing::error!("Gateway Timeout: Downstream AI node hung for >120s.");
            (
                StatusCode::GATEWAY_TIMEOUT,
                "Request timed out waiting for AI node",
            )
                .into_response()
        }
    }
}
