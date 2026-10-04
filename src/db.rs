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

/// Resolves the test database name.
///
/// Priority:
/// 1. `TKS_TEST_DB_NAME` environment variable if set and non-empty.
/// 2. `suite_override` if provided.
/// 3. Current executable file stem (e.g. `cascade_engine-abc123` -> `tks_test_cascade_engine`).
/// 4. Fallback default `tks_test`.
#[must_use]
pub fn resolve_test_database_name(suite_override: Option<&str>) -> String {
    if let Ok(name) = std::env::var("TKS_TEST_DB_NAME")
        && !name.trim().is_empty()
    {
        return name;
    }

    if let Some(suite) = suite_override {
        let clean: String = suite
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if !clean.is_empty() {
            return format!("tks_test_{clean}");
        }
    }

    if let Ok(exe) = std::env::current_exe()
        && let Some(stem) = exe.file_stem().and_then(|s| s.to_str())
    {
        let base = stem.split('-').next().unwrap_or(stem);
        let clean: String = base
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if !clean.is_empty() && clean != "tks" && clean != "test" {
            return format!("tks_test_{clean}");
        }
    }

    "tks_test".to_string()
}

/// Resolves the connection string for a specific test database name.
#[must_use]
pub fn resolve_test_database_url_for(db_name: &str) -> String {
    use std::net::ToSocketAddrs;

    if let Ok(url) = std::env::var("TKS_TEST_DATABASE_URL")
        && !url.trim().is_empty()
    {
        return url;
    }

    if ("postgres", 5432).to_socket_addrs().is_ok() {
        format!("postgresql://postgres:postgres@postgres:5432/{db_name}")
    } else {
        format!("postgresql://postgres:postgres@localhost:5432/{db_name}")
    }
}

/// Resolves the test database connection string, ensuring tests do not pollute
/// the development database.
#[must_use]
pub fn resolve_test_database_url() -> String {
    let db_name = resolve_test_database_name(None);
    resolve_test_database_url_for(&db_name)
}

/// Ensures an isolated test database is ready for the specified test suite or
/// current test binary.
///
/// Uses PostgreSQL database templating (`CREATE DATABASE ... TEMPLATE tks_template`)
/// to instantaneously provision isolated test databases with pgvector and migrations,
/// enabling safe parallel test execution across test binaries.
///
/// # Errors
///
/// Returns an error if connecting to PostgreSQL or provisioning the test database fails.
pub async fn ensure_test_database_ready_for(
    suite_name: Option<&str>,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    if let Ok(url) = std::env::var("TKS_TEST_DATABASE_URL")
        && !url.trim().is_empty()
    {
        let (mut client, _handle) = connect(&url).await?;
        let _ = client
            .execute("CREATE EXTENSION IF NOT EXISTS vector", &[])
            .await;
        run_migrations(&mut client).await?;
        return Ok(url);
    }

    let db_name = resolve_test_database_name(suite_name);
    let target_url = resolve_test_database_url_for(&db_name);
    let root_url = resolve_database_url();

    let (root_client, _root_handle) = connect(&root_url).await?;

    // 1. Serialize template database creation across concurrent processes via advisory lock
    let _ = root_client
        .query_one(
            "SELECT pg_advisory_lock(hashtext('tks_template_init'))",
            &[],
        )
        .await;

    let template_exists: bool = root_client
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM pg_database WHERE datname = 'tks_template')",
            &[],
        )
        .await
        .map(|r| r.get(0))
        .unwrap_or(false);

    if !template_exists {
        let _ = root_client
            .execute("CREATE DATABASE tks_template", &[])
            .await;
        let template_url = resolve_test_database_url_for("tks_template");
        if let Ok((mut tmpl_client, _tmpl_handle)) = connect(&template_url).await {
            let _ = tmpl_client
                .execute("CREATE EXTENSION IF NOT EXISTS vector", &[])
                .await;
            let _ = run_migrations(&mut tmpl_client).await;
        }
    }

    // 2. Check if the target isolated database exists
    let db_exists: bool = root_client
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM pg_database WHERE datname = $1)",
            &[&db_name],
        )
        .await
        .map(|r| r.get(0))
        .unwrap_or(false);

    if !db_exists {
        let create_sql = format!("CREATE DATABASE \"{db_name}\" TEMPLATE tks_template;");
        let _ = root_client.execute(&create_sql, &[]).await;
    }

    // Release advisory lock on root database
    let _ = root_client
        .query_one(
            "SELECT pg_advisory_unlock(hashtext('tks_template_init'))",
            &[],
        )
        .await;

    // 3. Connect to the isolated test database to verify pgvector and migrations
    let (mut test_client, _test_handle) = connect(&target_url).await?;
    let _ = test_client
        .execute("CREATE EXTENSION IF NOT EXISTS vector", &[])
        .await;
    run_migrations(&mut test_client).await?;

    Ok(target_url)
}

/// Ensures the test database exists, has `vector` extension enabled,
/// and has embedded migrations applied. Automatically isolates databases
/// per test binary based on `std::env::current_exe()` to support full test parallelism.
///
/// # Errors
///
/// Returns an error if connecting to PostgreSQL or creating the test database fails.
pub async fn ensure_test_database_ready() -> Result<String, Box<dyn std::error::Error + Send + Sync>>
{
    ensure_test_database_ready_for(None).await
}

/// Canonical test context providing an isolated test database and connection pool.
pub struct TestContext {
    pub db_name: String,
    pub database_url: String,
    pub pool: deadpool_postgres::Pool,
}

impl TestContext {
    /// Creates a new `TestContext` isolated to the calling test binary.
    ///
    /// # Errors
    ///
    /// Returns an error if database provisioning or connection pool creation fails.
    pub async fn new() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        Self::for_suite(None).await
    }

    /// Creates a new `TestContext` isolated to a specific named test suite.
    ///
    /// # Errors
    ///
    /// Returns an error if database provisioning or connection pool creation fails.
    pub async fn for_suite(
        suite_name: Option<&str>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let database_url = ensure_test_database_ready_for(suite_name).await?;
        let pool = create_pool(&database_url)?;
        let db_name = resolve_test_database_name(suite_name);
        Ok(Self {
            db_name,
            database_url,
            pool,
        })
    }

    /// Acquires a client connection from the isolated pool.
    ///
    /// # Errors
    ///
    /// Returns a `PoolError` if acquiring a client fails.
    pub async fn get_client(
        &self,
    ) -> Result<deadpool_postgres::Client, deadpool_postgres::PoolError> {
        self.pool.get().await
    }
}

/// Ephemeral test database context that automatically drops its database when dropped.
pub struct EphemeralTestContext {
    pub db_name: String,
    pub database_url: String,
    pub pool: deadpool_postgres::Pool,
    root_url: String,
}

impl EphemeralTestContext {
    /// Creates an ephemeral, uniquely named test database cloned from `tks_template`.
    ///
    /// # Errors
    ///
    /// Returns an error if database provisioning or pool initialization fails.
    pub async fn new() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let eph_id = uuid::Uuid::new_v4().simple().to_string();
        let suite = format!("eph_{eph_id}");
        let database_url = ensure_test_database_ready_for(Some(&suite)).await?;
        let pool = create_pool(&database_url)?;
        let root_url = resolve_database_url();
        Ok(Self {
            db_name: format!("tks_test_{suite}"),
            database_url,
            pool,
            root_url,
        })
    }

    /// Acquires a client from the ephemeral connection pool.
    ///
    /// # Errors
    ///
    /// Returns a `PoolError` if acquiring a client fails.
    pub async fn get_client(
        &self,
    ) -> Result<deadpool_postgres::Client, deadpool_postgres::PoolError> {
        self.pool.get().await
    }
}

impl Drop for EphemeralTestContext {
    fn drop(&mut self) {
        let root_url = self.root_url.clone();
        let db_name = self.db_name.clone();
        tokio::spawn(async move {
            if let Ok((client, _)) = connect(&root_url).await {
                let drop_sql = format!("DROP DATABASE IF EXISTS \"{db_name}\" WITH (FORCE);");
                let _ = client.execute(&drop_sql, &[]).await;
            }
        });
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
