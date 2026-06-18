//! ---------------------------------------------------------------------------
//! Fluxgate Enterprise API Gateway
//! Module: Dynamic Local Cluster DNS Provider
//! ---------------------------------------------------------------------------

use crate::doh::DnsProvider;
use redis::AsyncCommands;
use std::net::IpAddr;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::RwLock;

/// A dynamic DNS resolver that intercepts requests for a specific domain.
/// Synchronizes continuously with Redis to discover active gateway nodes dynamically.
pub struct LocalClusterProvider {
    /// The specific internal domain to intercept (e.g., "api.fluxgate.local")
    target_domain: String,
    /// Thread-safe, read-optimized cache of active cluster IPs
    node_ips: Arc<RwLock<Vec<IpAddr>>>,
    /// Thread-safe atomic counter for non-blocking Round-Robin distribution
    current_index: AtomicUsize,
}

impl LocalClusterProvider {
    /// Initializes the provider and spawns a background synchronization task.
    ///
    /// # Arguments
    /// * `domain` - The internal domain to intercept.
    /// * `redis_client` - Client to fetch the active 'fluxgate:cluster:nodes' set.
    pub fn new(domain: &str, redis_client: redis::Client) -> Self {
        let node_ips = Arc::new(RwLock::new(Vec::new()));

        // Spawn the background discovery worker
        let sync_ips = node_ips.clone();
        tokio::spawn(async move {
            Self::sync_worker(redis_client, sync_ips).await;
        });

        Self {
            target_domain: domain.to_string(),
            node_ips,
            current_index: AtomicUsize::new(0),
        }
    }

    /// Background daemon that periodically refreshes the active node list from Redis.
    async fn sync_worker(redis_client: redis::Client, cache: Arc<RwLock<Vec<IpAddr>>>) {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(10));

        loop {
            interval.tick().await;
            if let Ok(mut con) = redis_client.get_async_connection().await {
                // Assume infrastructure registers active nodes to a Redis Set
                let result: redis::RedisResult<Vec<String>> =
                    con.smembers("fluxgate:cluster:nodes").await;

                if let Ok(ip_strings) = result {
                    let mut fresh_ips = Vec::new();
                    for ip_str in ip_strings {
                        if let Ok(ip) = IpAddr::from_str(&ip_str) {
                            fresh_ips.push(ip);
                        }
                    }

                    if !fresh_ips.is_empty() {
                        let mut write_lock = cache.write().await;
                        *write_lock = fresh_ips;
                    }
                }
            }
        }
    }
}

#[async_trait::async_trait]
impl DnsProvider for LocalClusterProvider {
    async fn resolve(&self, domain: &str) -> Option<Vec<IpAddr>> {
        if domain == self.target_domain {
            let read_lock = self.node_ips.read().await;

            if read_lock.is_empty() {
                return None;
            }

            // Atomic Round-Robin Selection
            let index = self.current_index.fetch_add(1, Ordering::Relaxed);
            let selected_ip = read_lock[index % read_lock.len()];

            tracing::debug!(
                "DNS Intercept: Routed {} to Dynamic Node {}",
                domain,
                selected_ip
            );
            return Some(vec![selected_ip]);
        }

        None
    }
}
