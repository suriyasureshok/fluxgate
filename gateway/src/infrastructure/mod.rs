//! ---------------------------------------------------------------------------
//! Fluxgate Enterprise API Gateway
//! Module: Infrastructure & Data Access Layer
//! ---------------------------------------------------------------------------

pub mod cache;
pub mod database;

use deadpool_redis::Pool as RedisPool;
use sqlx::PgPool;

/// The Global App State holding all persistent, pooled connections.
/// Wrapped in `Arc` inside `main.rs` to be shared safely across all asynchronous Axum threads.
pub struct GatewayState {
    pub db_pool: PgPool,
    pub redis_pool: RedisPool,
}

impl GatewayState {
    /// Bootstraps and composes the infrastructure components into a single application state.
    ///
    /// # Arguments
    /// * `database_url` - Full connection string for the PostgreSQL cluster.
    /// * `redis_url` - Full connection string for the Redis cache instance.
    pub async fn initialize(database_url: &str, redis_url: &str) -> Self {
        // Data sources are spun up serially to prevent catastrophic cascading failures
        // during environment boot sequences. If DB fails, we don't bother pinging Redis.

        let db_pool = database::init_postgres(database_url).await;
        let redis_pool = cache::init_redis_pool(redis_url).await;

        Self {
            db_pool,
            redis_pool,
        }
    }
}
