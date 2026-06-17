use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

/// Initializes the PostgreSQL Connection Pool.
pub async fn init_postgres(database_url: &str) -> PgPool {
    tracing::info!("Initializing PostgreSQL Connection Pool...");
    PgPoolOptions::new()
        .max_connections(50) // Scale this based on your cloud hardware
        .connect(database_url)
        .await
        .expect("CRITICAL: Failed to connect to PostgreSQL Single Source of Truth.")
}
