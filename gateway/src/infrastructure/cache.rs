//! ---------------------------------------------------------------------------
//! Fluxgate Enterprise API Gateway
//! Module: Redis Hot Cache Infrastructure
//! ---------------------------------------------------------------------------

use deadpool_redis::{Config, Pool, Runtime};

/// Initializes the Redis Connection Pool and validates network reachability.
///
/// Transitions from a per-request TCP handshake to a pre-warmed connection pool.
/// Includes a fail-fast boot check to ensure the cache is fully operational before
/// the gateway accepts inbound traffic.
pub async fn init_redis_pool(redis_url: &str) -> Pool {
    tracing::info!("Initializing Redis Hot Cache Pool...");

    let cfg = Config::from_url(redis_url);

    let pool = cfg
        .create_pool(Some(Runtime::Tokio1))
        .expect("CRITICAL BOOT FAILURE: Failed to configure the Redis connection pool.");

    // Fail-Fast: Actively ping Redis to ensure the network route is open.
    // Client::open() does not verify connections, so we must force a query here.
    let mut conn = pool
        .get()
        .await
        .expect("CRITICAL BOOT FAILURE: Redis server is unreachable. Check network/credentials.");

    let _: () = redis::cmd("PING")
        .query_async(&mut conn)
        .await
        .expect("CRITICAL BOOT FAILURE: Redis PING command failed or timed out.");

    tracing::info!("Redis Hot Cache successfully verified and pooled.");
    pool
}
