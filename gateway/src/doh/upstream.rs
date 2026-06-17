//! ---------------------------------------------------------------------------
//! Fluxgate Enterprise API Gateway
//! Module: Upstream Public DNS Provider
//! ---------------------------------------------------------------------------

use crate::doh::DnsProvider;
use serde::Deserialize;
use std::net::IpAddr;
use std::str::FromStr;

// --- Standardized Public DoH Endpoints ---
pub const PROVIDER_CLOUDFLARE: &str = "https://cloudflare-dns.com/dns-query";
pub const PROVIDER_GOOGLE: &str = "https://dns.google/resolve";
pub const PROVIDER_QUAD9: &str = "https://dns.quad9.net:5053/dns-query";

/// Expected JSON structure from RFC 8427 / Google DoH API specifications
#[derive(Deserialize, Debug)]
struct DohJsonResponse {
    #[serde(rename = "Answer")]
    answer: Option<Vec<DohAnswer>>,
}

#[derive(Deserialize, Debug)]
struct DohAnswer {
    #[serde(rename = "type")]
    record_type: u16, // Type 1 = A (IPv4), Type 28 = AAAA (IPv6)
    data: String, // The actual IP address string
}

/// A generic DoH resolver that connects to standard public JSON DoH APIs.
pub struct UpstreamProvider {
    /// The configured DoH endpoint URL (e.g., Cloudflare, Google)
    endpoint_url: String,
    /// An asynchronous HTTP client (reused for connection pooling)
    client: reqwest::Client,
}

impl UpstreamProvider {
    /// Initializes a new UpstreamProvider targeting a specific DoH URL.
    ///
    /// # Example
    /// ```
    /// let provider = UpstreamProvider::new(PROVIDER_CLOUDFLARE);
    /// ```
    pub fn new(endpoint_url: &str) -> Self {
        Self {
            endpoint_url: endpoint_url.to_string(),
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait::async_trait]
impl DnsProvider for UpstreamProvider {
    async fn resolve(&self, domain: &str) -> Option<Vec<IpAddr>> {
        let mut ips = Vec::new();

        // Query both A and AAAA records
        for record_type in ["A", "AAAA"] {
            let url = format!("{}?name={}&type={}", self.endpoint_url, domain, record_type);

            let response = match self
                .client
                .get(&url)
                .header("Accept", "application/dns-json")
                .send()
                .await
            {
                Ok(resp) => resp,
                Err(e) => {
                    tracing::warn!(
                        "Upstream DoH Provider timeout/failure for {}: {}",
                        record_type,
                        e
                    );
                    continue;
                }
            };

            if let Ok(json) = response.json::<DohJsonResponse>().await {
                if let Some(answers) = json.answer {
                    for answer in answers {
                        if answer.record_type == 1 || answer.record_type == 28 {
                            if let Ok(ip) = IpAddr::from_str(&answer.data) {
                                ips.push(ip);
                            }
                        }
                    }
                }
            }
        }

        if !ips.is_empty() {
            tracing::debug!("Upstream resolved {} to {:?}", domain, ips);
            return Some(ips);
        }

        None
    }
}
