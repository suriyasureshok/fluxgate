//! Reverse Proxy Multiplexer (Streaming-Enabled)
//!
//! Intercepts inbound HTTP frames, sanitizes headers to mitigate security risks,
//! and forwards the payload to downstream AI services. Response payloads are
//! streamed chunk-by-chunk to eliminate gateway latency and RAM pinning.

use axum::{
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use std::sync::Arc;

/// Shared configuration state for the proxy engine.
pub struct ProxyState {
    /// The asynchronous HTTP client pool.
    pub client: reqwest::Client,
    /// The base URL of the downstream AI service (e.g., `http://host.docker.internal:11434`).
    pub downstream_url: String,
}

/// Maps inbound Axum headers to outbound Reqwest headers.
///
/// Explicitly whitelists and transforms safe headers to prevent header-smuggling
/// attacks and avoid leaking internal gateway state to downstream providers.
fn prepare_forward_headers(inbound_headers: &HeaderMap) -> reqwest::header::HeaderMap {
    let mut forward_headers = reqwest::header::HeaderMap::new();

    // Safely extract and forward the Authorization token if present
    if let Some(auth) = inbound_headers.get(axum::http::header::AUTHORIZATION) {
        if let Ok(val) = reqwest::header::HeaderValue::from_bytes(auth.as_bytes()) {
            forward_headers.insert(reqwest::header::AUTHORIZATION, val);
        } else {
            tracing::warn!("Received malformed Authorization header; dropping from proxy frame.");
        }
    }

    // Enforce strict JSON payloads for all downstream AI model interactions
    forward_headers.insert(
        reqwest::header::CONTENT_TYPE,
        reqwest::header::HeaderValue::from_static("application/json"),
    );

    forward_headers
}

/// Main Axum endpoint handler for dynamic API proxying.
///
/// Captures the wildcard path, reconstructs the target URI, and immediately
/// pipes the downstream byte stream back to the client as chunks arrive.
pub async fn handle_proxy_request(
    State(state): State<Arc<ProxyState>>,
    Path(remaining_path): Path<String>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    // 1. Reconstruct the exact downstream target URI
    let target_url = format!("{}/v1/{}", state.downstream_url, remaining_path);
    tracing::debug!("Streaming request multiplexed to target: {}", target_url);

    // 2. Sanitize and prepare the outbound HTTP headers
    let forward_headers = prepare_forward_headers(&headers);

    // 3. Compile the outbound request frame
    let proxy_request = match state
        .client
        .post(&target_url)
        .headers(forward_headers)
        .body(body)
        .build()
    {
        Ok(req) => req,
        Err(err) => {
            tracing::error!("Gateway Fault: Failed to compile outbound proxy request: {err}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal gateway error: Request compilation failed",
            )
                .into_response();
        }
    };

    // 4. Execute the network transit pipeline
    match state.client.execute(proxy_request).await {
        Ok(response) => {
            // Safely map the downstream status code back to the Axum response
            let status = StatusCode::from_u16(response.status().as_u16())
                .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

            // Extract essential headers to forward back to the client
            let mut response_headers = HeaderMap::new();
            if let Some(content_type) = response.headers().get(reqwest::header::CONTENT_TYPE) {
                response_headers.insert(axum::http::header::CONTENT_TYPE, content_type.clone());
            }
            
            // Ensure the client knows the data is arriving dynamically
            if response.headers().get(reqwest::header::TRANSFER_ENCODING).is_some() {
                response_headers.insert(
                    axum::http::header::TRANSFER_ENCODING,
                    axum::http::header::HeaderValue::from_static("chunked"),
                );
            }

            // ARCHITECTURE UPGRADE: Convert the reqwest response into an async stream.
            // Axum's Body::from_stream natively accepts reqwest's byte stream.
            let byte_stream = response.bytes_stream();
            let body = Body::from_stream(byte_stream);

            (status, response_headers, body).into_response()
        }
        Err(err) => {
            tracing::error!("Network route failure: Downstream infrastructure unreachable: {err}");
            (
                StatusCode::BAD_GATEWAY,
                "Downstream AI service is unreachable",
            )
                .into_response()
            }
    }
}
