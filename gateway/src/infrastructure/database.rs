//! ---------------------------------------------------------------------------
//! Fluxgate Enterprise API Gateway
//! Module: PostgreSQL Infrastructure
//! ---------------------------------------------------------------------------

use sqlx::postgres::{PgPool, PgPoolOptions};
use std::time::Duration;

/// Initializes the PostgreSQL Connection Pool with enterprise safeguards.
///
/// Implements strict timeouts and connection recycling to prevent thread starvation
/// during database locking events or network partitions.
pub async fn init_postgres(database_url: &str) -> PgPool {
    tracing::info!("Initializing PostgreSQL Connection Pool...");

    PgPoolOptions::new()
        .min_connections(5) // Maintain warm connections for sudden traffic spikes
        .max_connections(50) // Upper limit constraint per gateway node
        .acquire_timeout(Duration::from_secs(3)) // Fail-fast: Drop requests rather than hanging if DB is unresponsive
        .idle_timeout(Duration::from_secs(600)) // Prune stale connections to free up PgBouncer/DB memory
        .max_lifetime(Duration::from_secs(1800)) // Force connection rotation to prevent silent socket drops
        .connect(database_url)
        .await
        .expect("CRITICAL BOOT FAILURE: Failed to connect to PostgreSQL Single Source of Truth.")
}
