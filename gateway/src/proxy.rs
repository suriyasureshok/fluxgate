//! ---------------------------------------------------------------------------
//! Fluxgate Enterprise API Gateway
//! Module: Reverse Proxy & GPU Load Balancer
//! ---------------------------------------------------------------------------
//!
//! Intercepts inbound HTTP frames, sanitizes headers to mitigate security risks,
//! and forwards the payload to a cluster of downstream AI services.
//!
//! # Architecture
//! 1. **Round-Robin Distribution**: Utilizes a non-blocking `AtomicUsize` to
//!    evenly distribute incoming inference requests across multiple Ollama nodes.
//! 2. **Streaming Passthrough**: Response payloads are streamed chunk-by-chunk
//!    directly back to the client to eliminate gateway latency and RAM pinning.

use axum::{
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Shared configuration state for the proxy engine and load balancer.
pub struct ProxyState {
    /// The asynchronous HTTP client pool.
    pub client: reqwest::Client,
    /// A list of available downstream AI service base URLs.
    pub downstream_urls: Vec<String>,
    /// Thread-safe atomic counter for Round-Robin selection.
    current_index: AtomicUsize,
}

impl ProxyState {
    /// Initializes a new ProxyState with a cluster of downstream URLs.
    pub fn new(urls: Vec<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            downstream_urls: urls,
            current_index: AtomicUsize::new(0),
        }
    }

    /// Selects the next downstream URL using a fast, non-blocking atomic counter.
    pub fn get_next_target(&self) -> String {
        if self.downstream_urls.is_empty() {
            return String::new();
        }
        let index = self.current_index.fetch_add(1, Ordering::Relaxed);
        self.downstream_urls[index % self.downstream_urls.len()].clone()
    }
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
/// Captures the wildcard path, selects a downstream node via Round-Robin,
/// reconstructs the target URI, and immediately pipes the downstream
/// byte stream back to the client as chunks arrive.
pub async fn handle_proxy_request(
    State(state): State<Arc<ProxyState>>,
    // FUTURE: axum::extract::Extension(redis_cache): axum::extract::Extension<redis::Client>, // Instantly access the cache
    Path(remaining_path): Path<String>,
    headers: HeaderMap,
    body: axum::body::Bytes, // <-- In a stateful proxy, we must deserialize this!
) -> Response {
    // 1. Load Balancing: Select the next available GPU node
    let base_url = state.get_next_target();
    if base_url.is_empty() {
        tracing::error!("Gateway Fault: No downstream AI nodes configured.");
        return (StatusCode::SERVICE_UNAVAILABLE, "No AI nodes available").into_response();
    }

    // 2. Reconstruct the exact downstream target URI
    let target_url = format!("{}/v1/{}", base_url, remaining_path);
    tracing::debug!("Streaming request multiplexed to target: {}", target_url);

    // 3. Sanitize and prepare the outbound HTTP headers
    let forward_headers = prepare_forward_headers(&headers);

    // =========================================================================
    // FUTURE MILESTONE: STATEFUL SESSION MANAGEMENT (RAG & HISTORY)
    // =========================================================================
    // To transition from a Stateless "Dumb Pipe" to a Stateful Enterprise Gateway:
    //
    // 1. EXTRACT: Use `serde_json` to parse the `body` bytes into a struct.
    //    Find the user's `session_id` and their raw `"messages": [...]` payload.
    // 2. QUERY: Use `GatewayState.redis_client` to fetch the last 10 messages
    //    associated with that `session_id`.
    // 3. MERGE: Append the user's new message to the Redis history.
    // 4. OVERRIDE: Serialize the combined history back into bytes and reassign
    //    it to the `body` variable below.
    // 5. CACHE ASIDE: After `state.client.execute` finishes, grab the AI's
    //    streaming text and append it to Redis asynchronously.
    // =========================================================================

    // 4. Compile the outbound request frame
    let proxy_request = match state
        .client
        .post(&target_url)
        .headers(forward_headers)
        .body(body) // <-- In the future, this becomes `enriched_body`
        .build()
    {
        Ok(req) => req,
        Err(err) => {
            tracing::error!("Gateway Fault: Failed to compile outbound proxy request: {err}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Request compilation failed",
            )
                .into_response();
        }
    };

    // 5. Execute the network transit pipeline
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

            // ARCHITECTURE UPGRADE: Convert the reqwest response into an async stream.
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
