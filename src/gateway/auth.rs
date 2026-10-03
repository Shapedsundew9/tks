//! Authentication state, credential caching, and Axum extractors (WP-1.5, TB-5, D-39, D-49).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::extract::{FromRef, FromRequestParts};
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use deadpool_postgres::Pool;
use moka::future::Cache;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;

/// Authenticated identity for an external agent or human supervisor.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthenticatedAgent {
    /// Canonical agent or human identifier (e.g. `tks_dev_token`, `agent-42`).
    pub agent_id: String,
    /// Actor classification (`HUMAN` or `AGENT`).
    pub actor_type: String,
}

impl AuthenticatedAgent {
    /// Returns token fingerprint or agent id fallback for audit ledger logging.
    #[must_use]
    pub fn token_fingerprint(&self) -> String {
        self.agent_id.clone()
    }
}

/// Errors originating from authentication and identity validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthError {
    /// Authorization header is missing or malformed.
    MissingCredentials,
    /// Token was not found, invalid, or revoked.
    Unauthorized,
    /// Database or pool communication failure.
    Internal(String),
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let (status, msg) = match self {
            Self::MissingCredentials => (
                StatusCode::UNAUTHORIZED,
                "Missing or malformed Authorization header (expected 'Bearer <token>')".to_string(),
            ),
            Self::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "Invalid or revoked credentials".to_string(),
            ),
            Self::Internal(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Internal auth error: {err}"),
            ),
        };

        (
            status,
            Json(serde_json::json!({
                "error": msg
            })),
        )
            .into_response()
    }
}

/// Computes a standard SHA-256 hexadecimal digest from a raw token.
#[must_use]
pub fn compute_token_hash(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// In-process identity cache and token validation state.
#[derive(Clone)]
pub struct AuthState {
    pool: Pool,
    token_cache: Cache<String, AuthenticatedAgent>,
    agent_to_tokens: Arc<RwLock<HashMap<String, HashSet<String>>>>,
}

impl AuthState {
    /// Creates a new `AuthState` instance with in-process LRU cache.
    #[must_use]
    pub fn new(pool: Pool) -> Self {
        let token_cache = Cache::builder()
            .max_capacity(1_000)
            .time_to_live(Duration::from_secs(3600))
            .build();

        Self {
            pool,
            token_cache,
            agent_to_tokens: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Accessor for underlying database connection pool.
    #[must_use]
    pub fn pool(&self) -> &Pool {
        &self.pool
    }

    /// Accessor for underlying Moka token cache.
    #[must_use]
    pub fn token_cache(&self) -> &Cache<String, AuthenticatedAgent> {
        &self.token_cache
    }

    /// Validates an incoming bearer token against in-process LRU cache and PostgreSQL.
    ///
    /// # Errors
    ///
    /// Returns `AuthError::Unauthorized` if credentials are invalid or revoked,
    /// or `AuthError::Internal` on pool/database errors.
    pub async fn authenticate(&self, token: &str) -> Result<AuthenticatedAgent, AuthError> {
        let trimmed = token.trim();
        if trimmed.is_empty() {
            return Err(AuthError::Unauthorized);
        }

        let hash = compute_token_hash(trimmed);

        // 1. Fast path: check in-process LRU cache for hashed token
        if let Some(agent) = self.token_cache.get(&hash).await {
            return Ok(agent);
        }

        // 2. Also check cache for raw token (e.g. dev seed token)
        if let Some(agent) = self.token_cache.get(trimmed).await {
            return Ok(agent);
        }

        // 3. Cache miss: query PostgreSQL agent_identities table
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| AuthError::Internal(e.to_string()))?;

        let row_opt = client
            .query_opt(
                "SELECT agent_id, actor_type, token_hash \
                 FROM agent_identities \
                 WHERE (token_hash = $1 OR token_hash = $2) AND is_active = TRUE;",
                &[&hash, &trimmed],
            )
            .await
            .map_err(|e| AuthError::Internal(e.to_string()))?;

        match row_opt {
            Some(row) => {
                let agent_id: String = row.get("agent_id");
                let actor_type: String = row.get("actor_type");
                let db_token_hash: String = row.get("token_hash");

                let agent = AuthenticatedAgent {
                    agent_id: agent_id.clone(),
                    actor_type,
                };

                // Seed Moka LRU cache with both the computed SHA-256 hash and DB token hash
                self.token_cache.insert(hash.clone(), agent.clone()).await;
                if db_token_hash != hash {
                    self.token_cache
                        .insert(db_token_hash.clone(), agent.clone())
                        .await;
                }

                // Track mapping for immediate invalidation
                let mut map = self.agent_to_tokens.write().await;
                let set = map.entry(agent_id).or_default();
                set.insert(hash);
                set.insert(db_token_hash);

                Ok(agent)
            }
            None => Err(AuthError::Unauthorized),
        }
    }

    /// Registers a newly provisioned identity in the in-process cache.
    pub async fn register_token(&self, agent_id: &str, token_hash: &str, actor_type: &str) {
        let agent = AuthenticatedAgent {
            agent_id: agent_id.to_string(),
            actor_type: actor_type.to_string(),
        };

        self.token_cache.insert(token_hash.to_string(), agent).await;

        let mut map = self.agent_to_tokens.write().await;
        let set = map.entry(agent_id.to_string()).or_default();
        set.insert(token_hash.to_string());
    }

    /// Immediately evicts an agent's credentials from the in-process Moka LRU cache (TB-5, D-49).
    pub async fn invalidate(&self, agent_id: &str, specific_token_hash: Option<&str>) {
        if let Some(hash) = specific_token_hash {
            self.token_cache.invalidate(hash).await;
        }

        let mut map = self.agent_to_tokens.write().await;
        if let Some(tokens) = map.remove(agent_id) {
            for token in tokens {
                self.token_cache.invalidate(&token).await;
            }
        }

        // Also invalidate by agent_id in case it was used directly as key
        self.token_cache.invalidate(agent_id).await;
    }
}

impl<S> FromRequestParts<S> for AuthenticatedAgent
where
    S: Send + Sync,
    AuthState: FromRef<S>,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let auth_state = AuthState::from_ref(state);

        let auth_header = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok());

        let token = match auth_header {
            Some(h) if h.starts_with("Bearer ") => h["Bearer ".len()..].trim(),
            _ => return Err(AuthError::MissingCredentials.into_response()),
        };

        auth_state
            .authenticate(token)
            .await
            .map_err(IntoResponse::into_response)
    }
}

/// Optional authentication extractor: returns `Some(agent)` if valid Bearer token provided,
/// `None` if no Authorization header present, or `401 Unauthorized` if invalid token.
#[derive(Clone, Debug)]
pub struct MaybeAuthenticatedAgent(pub Option<AuthenticatedAgent>);

impl<S> FromRequestParts<S> for MaybeAuthenticatedAgent
where
    S: Send + Sync,
    AuthState: FromRef<S>,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let auth_state = AuthState::from_ref(state);

        let auth_header = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok());

        let token = match auth_header {
            Some(h) if h.starts_with("Bearer ") => h["Bearer ".len()..].trim(),
            Some(_) => return Err(AuthError::MissingCredentials.into_response()),
            None => return Ok(MaybeAuthenticatedAgent(None)),
        };

        let agent = auth_state
            .authenticate(token)
            .await
            .map_err(IntoResponse::into_response)?;

        Ok(MaybeAuthenticatedAgent(Some(agent)))
    }
}
