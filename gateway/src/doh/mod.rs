//! ---------------------------------------------------------------------------
//! Fluxgate Enterprise API Gateway
//! Module: Modular DNS-over-HTTPS (DoH) Router
//! ---------------------------------------------------------------------------
//!
//! High-performance, asynchronous DoH resolution endpoint. Uses a Chain of
//! Responsibility pattern to evaluate local cluster routing before falling back
//! to external public DNS providers.

use axum::{
    body::Bytes,
    extract::State,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use std::net::IpAddr;
use std::sync::Arc;

pub mod local;
pub mod upstream;

/// Architectural contract for DNS resolution providers in the DoH chain.
#[async_trait::async_trait]
pub trait DnsProvider: Send + Sync {
    /// Attempts to resolve a domain name into a list of IP addresses.
    ///
    /// # Arguments
    /// * `domain` - The fully qualified domain name (FQDN) without the trailing dot.
    ///
    /// # Returns
    /// * `Some(Vec<IpAddr>)` - Resolved IPs if the provider handles this domain.
    /// * `None` - If unhandled, allowing the next provider in the chain to execute.
    async fn resolve(&self, domain: &str) -> Option<Vec<IpAddr>>;
}

/// Shared state for the DoH router, holding the configured resolution chain.
pub struct DohState {
    /// Primary provider (e.g., dynamic local cluster routing).
    pub primary_provider: Box<dyn DnsProvider>,
    /// Fallback provider (e.g., Cloudflare/Google public DoH).
    pub fallback_provider: Option<Box<dyn DnsProvider>>,
}

/// Core Axum Handler: Intercepts binary DNS packets from DoH clients.
pub async fn handle_dns_query(State(state): State<Arc<DohState>>, body: Bytes) -> Response {
    // 1. Parse the incoming binary DNS packet
    let message = match trust_dns_proto::op::Message::from_vec(&body) {
        Ok(m) => m,
        Err(e) => {
            tracing::error!("Fault: Failed to parse incoming DNS packet: {}", e);
            return (StatusCode::BAD_REQUEST).into_response();
        }
    };

    let mut response = trust_dns_proto::op::Message::new();
    response.set_id(message.id());
    response.set_message_type(trust_dns_proto::op::MessageType::Response);

    let mut all_resolved = true;

    // 2. Iterate through ALL queries in the packet (fixes the single-query bottleneck)
    for query in message.queries() {
        response.add_query(query.clone());

        let domain = query.name().to_string();
        let clean_domain = domain.trim_end_matches('.');

        // 3. Chain of Responsibility: Local first, then Fallback
        let mut resolved_ips = state.primary_provider.resolve(clean_domain).await;

        if resolved_ips.is_none() {
            if let Some(fallback) = &state.fallback_provider {
                resolved_ips = fallback.resolve(clean_domain).await;
            }
        }

        // 4. Append answers if resolved
        if let Some(ips) = resolved_ips {
            for ip in ips {
                let rdata = match ip {
                    IpAddr::V4(ipv4) => trust_dns_proto::rr::RData::A(ipv4.into()),
                    IpAddr::V6(ipv6) => trust_dns_proto::rr::RData::AAAA(ipv6.into()),
                };

                // TTL set to 30s to allow fast failover during cluster scaling
                let record =
                    trust_dns_proto::rr::Record::from_rdata(query.name().clone(), 30, rdata);
                response.add_answer(record);
            }
        } else {
            all_resolved = false;
        }
    }

    if all_resolved {
        response.set_response_code(trust_dns_proto::op::ResponseCode::NoError);
    } else {
        response.set_response_code(trust_dns_proto::op::ResponseCode::NXDomain);
    }

    // 5. Serialize and return the DoH payload
    match response.to_vec() {
        Ok(bytes) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/dns-message")],
            bytes,
        )
            .into_response(),
        Err(e) => {
            tracing::error!("Fault: Failed to serialize DNS response: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR).into_response()
        }
    }
}
