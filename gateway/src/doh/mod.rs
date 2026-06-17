//! ---------------------------------------------------------------------------
//! Fluxgate Enterprise API Gateway
//! Module: Modular DNS-over-HTTPS (DoH) Router
//! ---------------------------------------------------------------------------
//!
//! This module provides a high-performance, asynchronous DNS-over-HTTPS (DoH)
//! resolution endpoint. It is designed to intercept DNS queries from clients
//! (like mobile apps) and route them through a "Chain of Responsibility".
//!
//! # Architecture
//! 1. **Local Resolution**: Queries are first checked against the `primary_provider`
//!    (usually the LocalClusterProvider) to see if they belong to our internal network.
//! 2. **Upstream Fallback**: If the local provider ignores the query (returns `None`),
//!    the query is forwarded to an external `fallback_provider` (e.g., Cloudflare, Google).

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

/// The architectural contract for any DNS resolution provider.
/// Any struct implementing this trait can be plugged into the DoH resolution chain.
#[async_trait::async_trait]
pub trait DnsProvider: Send + Sync {
    /// Attempts to resolve a domain name into a list of IP addresses.
    ///
    /// # Arguments
    /// * `domain` - The fully qualified domain name (FQDN) without the trailing dot.
    ///
    /// # Returns
    /// * `Some(Vec<IpAddr>)` - A list of resolved IP addresses if the provider handles this domain.
    /// * `None` - If the provider does not handle this domain, allowing the chain to continue.
    async fn resolve(&self, domain: &str) -> Option<Vec<IpAddr>>;
}

/// The shared state for the DoH router, holding our configured resolution chain.
pub struct DohState {
    /// The first provider in the chain (e.g., local cluster routing).
    pub primary_provider: Box<dyn DnsProvider>,
    /// The fallback provider if the primary skips the request (e.g., Public DoH).
    pub fallback_provider: Option<Box<dyn DnsProvider>>,
}

/// Core Axum Handler: Intercepts binary DNS packets from DoH clients.
///
/// This endpoint expects an `application/dns-message` content type containing
/// a raw wire-format DNS packet. It parses the packet, queries the provider chain,
/// and returns a constructed wire-format response.
pub async fn handle_dns_query(State(state): State<Arc<DohState>>, body: Bytes) -> Response {
    // 1. Parse the binary DNS packet using the Trust-DNS protocol
    let message = match trust_dns_proto::op::Message::from_vec(&body) {
        Ok(m) => m,
        Err(e) => {
            tracing::error!("Failed to parse incoming DNS packet: {}", e);
            return (StatusCode::BAD_REQUEST).into_response();
        }
    };

    let query = match message.queries().first() {
        Some(q) => q,
        None => return (StatusCode::BAD_REQUEST).into_response(),
    };

    let domain = query.name().to_string();
    let clean_domain = domain.trim_end_matches('.');

    // 2. Route the query through our Provider Chain (Chain of Responsibility pattern)
    let mut resolved_ips = state.primary_provider.resolve(clean_domain).await;

    // If local fails, try the fallback (e.g., Cloudflare/Google) if configured
    if resolved_ips.is_none() {
        if let Some(fallback) = &state.fallback_provider {
            resolved_ips = fallback.resolve(clean_domain).await;
        }
    }

    // 3. Construct the binary response packet
    let mut response = trust_dns_proto::op::Message::new();
    response.set_id(message.id());
    response.add_query(query.clone());

    if let Some(ips) = resolved_ips {
        // We successfully resolved the IP(s); append A/AAAA records
        response.set_message_type(trust_dns_proto::op::MessageType::Response);
        response.set_response_code(trust_dns_proto::op::ResponseCode::NoError);

        for ip in ips {
            let rdata = match ip {
                IpAddr::V4(ipv4) => trust_dns_proto::rr::RData::A(ipv4.into()),
                IpAddr::V6(ipv6) => trust_dns_proto::rr::RData::AAAA(ipv6.into()),
            };

            let record = trust_dns_proto::rr::Record::from_rdata(
                query.name().clone(),
                60, // Standard TTL: 60 seconds
                rdata,
            );
            response.add_answer(record);
        }
    } else {
        // Neither local nor fallback could find the domain
        response.set_response_code(trust_dns_proto::op::ResponseCode::NXDomain);
    }

    // 4. Return the binary DoH payload to the client
    let response_bytes = response.to_vec().unwrap();
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/dns-message")],
        response_bytes,
    )
        .into_response()
}
