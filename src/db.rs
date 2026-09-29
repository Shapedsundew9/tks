//! Database connection, pooling, and migration harness.

use refinery::Report;
use tokio_postgres::{Client, NoTls};

mod embedded {
    use refinery::embed_migrations;
    embed_migrations!("migrations");
}

/// Runs embedded database migrations asynchronously using refinery.
///
/// # Errors
///
/// Returns a `refinery::Error` if any migration fails.
pub async fn run_migrations(client: &mut Client) -> Result<Report, refinery::Error> {
    embedded::migrations::runner().run_async(client).await
}

/// Connects to the database at `database_url` using `tokio-postgres`.
///
/// Returns the client and the background connection driver task handle.
///
/// # Errors
///
/// Returns an error if the connection cannot be established.
pub async fn connect(
    database_url: &str,
) -> Result<(Client, tokio::task::JoinHandle<()>), tokio_postgres::Error> {
    let (client, connection) = tokio_postgres::connect(database_url, NoTls).await?;
    let handle = tokio::spawn(async move {
        if let Err(e) = connection.await {
            tracing::error!("PostgreSQL connection error: {e}");
        }
    });
    Ok((client, handle))
}

/// Creates a `deadpool_postgres::Pool` configured from a database URL.
///
/// # Errors
///
/// Returns an error if pool creation fails.
pub fn create_pool(
    database_url: &str,
) -> Result<deadpool_postgres::Pool, Box<dyn std::error::Error + Send + Sync>> {
    let tokio_config: tokio_postgres::Config = database_url.parse()?;
    let mgr_config = deadpool_postgres::ManagerConfig {
        recycling_method: deadpool_postgres::RecyclingMethod::Fast,
    };
    let mgr = deadpool_postgres::Manager::from_config(tokio_config, NoTls, mgr_config);
    let pool = deadpool_postgres::Pool::builder(mgr).max_size(16).build()?;
    Ok(pool)
}
