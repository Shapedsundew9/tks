//! Database connection, pooling, and migration harness.

use refinery::Report;
use tokio_postgres::{Client, NoTls};

mod embedded {
    use refinery::embed_migrations;
    embed_migrations!("migrations");
}

/// Resolves the database connection string.
///
/// Priority:
/// 1. `TKS_DATABASE_URL` environment variable if set and non-empty.
/// 2. `DATABASE_URL` environment variable if set and non-empty, unless it points to an
///    external host without pgvector (e.g. host LAN leak).
/// 3. Devcontainer internal service hostname `postgres:5432` if resolvable.
/// 4. Localhost fallback `localhost:5432`.
#[must_use]
pub fn resolve_database_url() -> String {
    use std::net::ToSocketAddrs;

    if let Ok(url) = std::env::var("TKS_DATABASE_URL")
        && !url.trim().is_empty()
    {
        return url;
    }

    if let Ok(url) = std::env::var("DATABASE_URL")
        && !url.trim().is_empty()
        && !url.contains("192.168.")
        && !url.contains("shapedsundew9")
    {
        return url;
    }

    if ("postgres", 5432).to_socket_addrs().is_ok() {
        "postgresql://postgres:postgres@postgres:5432/postgres".to_string()
    } else {
        "postgresql://postgres:postgres@localhost:5432/postgres".to_string()
    }
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
