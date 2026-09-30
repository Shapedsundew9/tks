//! Identity provisioning and revocation REST routes (WP-1.5, TB-5, D-49).

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::gateway::AppState;
use crate::gateway::auth::compute_token_hash;

/// Payload for provisioning a new human or agent identity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateIdentityRequest {
    /// Human-readable agent or supervisor name.
    pub name: String,
    /// Actor role (`HUMAN` or `AGENT`).
    pub role: String,
    /// Optional pre-shared bearer token. If omitted, a secure random token is generated.
    pub token: Option<String>,
}

/// Response returned when an identity is successfully provisioned.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateIdentityResponse {
    /// The canonical identity identifier.
    pub agent_id: String,
    /// The raw bearer token (to be retained by the client).
    pub token: String,
}

/// Response returned on identity revocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevokeIdentityResponse {
    pub status: String,
}

/// Handles `POST /api/v1/identities`.
///
/// Provisions an identity in PostgreSQL `agent_identities`, seeds the in-process
/// `moka` LRU cache, and returns the raw bearer token.
pub async fn create_identity(
    State(state): State<AppState>,
    Json(payload): Json<CreateIdentityRequest>,
) -> Result<Response, (StatusCode, Json<serde_json::Value>)> {
    let name = payload.name.trim();
    if name.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Identity 'name' cannot be empty" })),
        ));
    }

    let role = payload.role.trim().to_uppercase();
    if role != "HUMAN" && role != "AGENT" {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Identity 'role' must be 'HUMAN' or 'AGENT'" })),
        ));
    }

    let raw_token = payload
        .token
        .unwrap_or_else(|| format!("tks_{}_{}", name, Uuid::new_v4().simple()));

    let token_hash = compute_token_hash(&raw_token);

    let client = state.pool.get().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Pool error: {e}") })),
        )
    })?;

    client
        .execute(
            "INSERT INTO agent_identities (agent_id, token_hash, actor_type, is_active) \
             VALUES ($1, $2, $3, TRUE) \
             ON CONFLICT (agent_id) DO UPDATE \
             SET token_hash = EXCLUDED.token_hash, \
                 actor_type = EXCLUDED.actor_type, \
                 is_active = TRUE;",
            &[&name, &token_hash, &role],
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Insert identity error: {e}") })),
            )
        })?;

    // Seed in-process Moka LRU cache immediately (TB-5)
    state.auth.register_token(name, &token_hash, &role).await;

    tracing::info!("Provisioned identity '{name}' with role '{role}'");

    let resp = CreateIdentityResponse {
        agent_id: name.to_string(),
        token: raw_token,
    };

    Ok((StatusCode::CREATED, Json(resp)).into_response())
}

/// Handles `POST /api/v1/identities/{id}/revoke`.
///
/// Updates PostgreSQL `agent_identities SET is_active = FALSE` and immediately
/// evicts the credential from the daemon's in-process `moka` LRU cache (D-49, TB-5).
pub async fn revoke_identity(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> Result<Response, (StatusCode, Json<serde_json::Value>)> {
    let client = state.pool.get().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Pool error: {e}") })),
        )
    })?;

    let row_opt = client
        .query_opt(
            "UPDATE agent_identities \
             SET is_active = FALSE \
             WHERE agent_id = $1 \
             RETURNING token_hash;",
            &[&agent_id],
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Revoke update error: {e}") })),
            )
        })?;

    let token_hash: String = match row_opt {
        Some(r) => r.get("token_hash"),
        None => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": format!("Identity '{agent_id}' not found") })),
            ));
        }
    };

    // Immediate in-process Moka cache invalidation (TB-5, D-49)
    state.auth.invalidate(&agent_id, Some(&token_hash)).await;

    tracing::warn!("Revoked identity '{agent_id}' and invalidated LRU cache");

    let resp = RevokeIdentityResponse {
        status: "REVOKED".to_string(),
    };

    Ok((StatusCode::OK, Json(resp)).into_response())
}
