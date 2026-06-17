//! ---------------------------------------------------------------------------
//! Fluxgate Enterprise API Gateway
//! Module: Infrastructure & Data Access Layer
//! ---------------------------------------------------------------------------

pub mod cache;
pub mod database;

use redis::Client;
use sqlx::PgPool;

/// The Global App State holding all persistent connections.
/// Wrapped in Arc inside main.rs to be shared across all request threads.
pub struct GatewayState {
    pub db_pool: PgPool,
    pub redis_client: Client,
}

impl GatewayState {
    /// Composes the infrastructure components into a single application state.
    pub async fn initialize(database_url: &str, redis_url: &str) -> Self {
        // These are now totally decoupled. If we swap Redis for Memcached later,
        // we only touch cache.rs and this struct, never the database code.

        let db_pool = database::init_postgres(database_url).await;
        let redis_client = cache::init_redis(redis_url);

        Self {
            db_pool,
            redis_client,
        }
    }
}
