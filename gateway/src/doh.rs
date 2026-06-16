//! DNS-over-HTTPS (DoH) Edge Resolver
//!
//! This module intercepts secure DoH traffic (`application/dns-message`),
//! decodes the raw binary DNS wire format, evaluates query matches against
//! designated internal routing configurations, and encodes responses.

use axum::{
    body::Bytes,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use std::net::Ipv4Addr;
use std::str::FromStr;
use trust_dns_proto::{
    op::{Message, MessageType, ResponseCode},
    rr::{Name, RData, Record},
};

/// The internal application routing domain authorized for local resolution.
const TARGET_DOMAIN: &str = "api.fluxgate.local.";

/// The loopback destination for the gateway proxy target.
const LOCAL_PROXY_IP: Ipv4Addr = Ipv4Addr::new(127, 0, 0, 1);

/// Cache Time-To-Live (TTL) configuration in seconds for returned A records.
const DEFAULT_TTL: u32 = 300;

/// A pure protocol engine function that evaluates a DNS query and maps it to a response message.
///
/// By decoupling this from Axum's HTTP layer, the core routing logic becomes
/// completely deterministic and open to isolated unit testing.
fn process_dns_message(request_msg: &Message) -> Message {
    let mut response_msg = Message::new();

    // Mirror standard transaction properties from the incoming wire packet
    response_msg
        .set_id(request_msg.id())
        .set_message_type(MessageType::Response)
        .set_op_code(request_msg.op_code())
        .set_response_code(ResponseCode::NoError);

    // Statically parse target domain; safe to unwrap given fixed constant layout
    let target_name = Name::from_str(TARGET_DOMAIN).unwrap();
    let mut is_fluxgate_query = false;

    // Echo incoming queries back inside the response block per DNS specifications
    for query in request_msg.queries() {
        response_msg.add_query(query.clone());

        if query.name() == &target_name {
            is_fluxgate_query = true;
        }
    }

    if is_fluxgate_query {
        let answer_record = Record::from_rdata(
            target_name,
            DEFAULT_TTL,
            RData::A(trust_dns_proto::rr::rdata::A(LOCAL_PROXY_IP)),
        );
        response_msg.add_answer(answer_record);
        tracing::debug!("Resolved internal zone hit for: {}", TARGET_DOMAIN);
    } else {
        // Explicitly reject foreign recursive queries to block open-resolver attack vectors
        response_msg.set_response_code(ResponseCode::NXDomain);
    }

    response_msg
}

/// Main Axum endpoint handler for inbound secure DNS queries.
///
/// Orchestrates the HTTP wrapper layer, enforces wire boundaries, and
/// ensures comprehensive telemetry tracking across pipeline boundaries.
pub async fn handle_dns_query(headers: HeaderMap, body: Bytes) -> Response {
    // 1. Validate the DoH Content-Type Header
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok());
    if content_type != Some("application/dns-message") {
        tracing::warn!(
            "Inbound request dropped due to unexpected Content-Type: {:?}",
            content_type
        );
        return (
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Expected application/dns-message",
        )
            .into_response();
    }

    // 2. Deserialize the raw bytes into a Trust-DNS Message structure
    let request_msg = match Message::from_vec(&body) {
        Ok(msg) => msg,
        Err(err) => {
            tracing::error!(
                "Protocol error: Failed to parse raw binary DNS stream: {}",
                err
            );
            return (StatusCode::BAD_REQUEST, "Malformed DNS payload").into_response();
        }
    };

    // 3. Evaluate queries against local network zone definitions
    let response_msg = process_dns_message(&request_msg);

    // 4. Serialize the structured response message back into wire-format bytes
    let response_bytes = match response_msg.to_vec() {
        Ok(bytes) => bytes,
        Err(err) => {
            tracing::error!(
                "Serialization panic: Failed to project DNS response to wire format: {}",
                err
            );
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "DNS Serialization failed",
            )
                .into_response();
        }
    };

    // 5. Stream the secure binary answer back to the requesting client
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/dns-message")],
        response_bytes,
    )
        .into_response()
}
