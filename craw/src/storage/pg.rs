use sqlx::postgres::{PgPool, PgPoolOptions};
use std::time::Duration;

pub async fn connect(database_url: &str) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(48)
        .min_connections(1)
        .acquire_timeout(Duration::from_secs(30))
        .connect(database_url)
        .await
}
