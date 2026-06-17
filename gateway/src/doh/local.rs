//! ---------------------------------------------------------------------------
//! Fluxgate Enterprise API Gateway
//! Module: Local Cluster DNS Provider
//! ---------------------------------------------------------------------------

use crate::doh::DnsProvider;
use std::net::IpAddr;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A local DNS resolver that intercepts requests for a specific domain
/// and round-robins traffic across multiple physical cluster nodes.
pub struct LocalClusterProvider {
    /// The specific internal domain to intercept (e.g., "api.fluxgate.local")
    target_domain: String,
    /// A list of physical IP addresses corresponding to Gateway nodes
    node_ips: Vec<IpAddr>,
    /// Thread-safe atomic counter for Round-Robin load balancing
    current_index: AtomicUsize,
}

impl LocalClusterProvider {
    /// Initializes a new LocalClusterProvider.
    pub fn new(domain: &str, ips: Vec<IpAddr>) -> Self {
        Self {
            target_domain: domain.to_string(),
            node_ips: ips,
            current_index: AtomicUsize::new(0),
        }
    }
}

#[async_trait::async_trait]
impl DnsProvider for LocalClusterProvider {
    async fn resolve(&self, domain: &str) -> Option<Vec<IpAddr>> {
        // Strict match: Only intercept traffic meant for our specific network
        if domain == self.target_domain {
            if self.node_ips.is_empty() {
                return None;
            }

            // Atomic Round-Robin Selection (Fast, non-blocking)
            let index = self.current_index.fetch_add(1, Ordering::Relaxed);
            let selected_ip = self.node_ips[index % self.node_ips.len()];
            tracing::debug!("DNS Intercept: Routed {} to Node {}", domain, selected_ip);

            // Return exactly one IP to force the client to connect to that specific node
            return Some(vec![selected_ip]);
        }

        // Pass the request down the chain if it isn't our target domain
        None
    }
}
