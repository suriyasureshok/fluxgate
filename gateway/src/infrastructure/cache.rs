use redis::Client;

/// Initializes the Redis Client.
pub fn init_redis(redis_url: &str) -> Client {
    tracing::info!("Initializing Redis Hot Cache Client...");
    Client::open(redis_url).expect("CRITICAL: Failed to connect to Redis Hot Cache.")
}
