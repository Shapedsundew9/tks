//! Explicit reverification interface for stale and degraded graph nodes (WP-2.3, D-76).
//!
//! Validates that all direct upstream dependencies are in `ACTIVE` state (satisfying INV-1),
//! transitions the node from `NEEDS_REVERIFICATION` back to `ACTIVE`, resets `staleness_score = 0.0`,
//! commits an explicit `REVERIFIED` event to `audit_ledger`, and triggers vector re-indexing in
//! `node_embeddings` under `pg_advisory_xact_lock(hashtext('tks_structural_mutation'))`.

use deadpool_postgres::Client as DeadpoolClient;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio_postgres::Client;
use uuid::Uuid;

use crate::gateway::auth::AuthenticatedAgent;
use crate::storage::envelope::validate_ancestor_path;
use crate::storage::mutation::{
    MutationError, assert_no_dag_cycle, insert_audit_event, insert_structural_edge,
    resolve_node_polymorphic,
};

/// Result of a successful node reverification operation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReverifyResult {
    /// Target node UUID that was reverified.
    pub node_id: Uuid,
    /// Resulting lifecycle state (`ACTIVE`).
    pub status: String,
    /// Reset staleness score (`0.0`).
    pub staleness_score: f64,
    /// Batch UUID correlating the REVERIFIED audit ledger event.
    pub batch_id: Uuid,
    /// Monotonic sequence number of the REVERIFIED event in audit_ledger.
    pub event_seq: i64,
}

/// Reverifies a node using a pooled connection (`&mut deadpool_postgres::Client`).
///
/// # Errors
///
/// Returns `MutationError::NotFound` if `node_id` cannot be resolved.
/// Returns `MutationError::DependencyInactive` if any direct upstream parent is not `ACTIVE`.
/// Returns `MutationError::InvalidAncestorPath` if the node lacks an unbroken path to an active requirement root.
/// Returns `MutationError` on query failure, serialization error, or lock contention.
pub async fn reverify_node(
    client: &mut DeadpoolClient,
    node_id: &str,
    rationale: &str,
    updated_attributes: Option<serde_json::Value>,
    actor: &AuthenticatedAgent,
) -> Result<ReverifyResult, MutationError> {
    reverify_node_client(client, node_id, rationale, updated_attributes, actor).await
}

/// Reverifies a node using a raw client connection (`&mut tokio_postgres::Client`).
///
/// # Errors
///
/// Returns `MutationError::NotFound` if `node_id` cannot be resolved.
/// Returns `MutationError::DependencyInactive` if any direct upstream parent is not `ACTIVE`.
/// Returns `MutationError::InvalidAncestorPath` if the node lacks an unbroken path to an active requirement root.
/// Returns `MutationError` on query failure, serialization error, or lock contention.
pub async fn reverify_node_client(
    client: &mut Client,
    node_id: &str,
    rationale: &str,
    updated_attributes: Option<serde_json::Value>,
    actor: &AuthenticatedAgent,
) -> Result<ReverifyResult, MutationError> {
    // 1. Resolve node polymorphically (returns GraphNode or NotFound)
    let current_node = resolve_node_polymorphic(client, node_id).await?;
    let resolved_id = current_node.id;

    let current_state = current_node.lifecycle_state;
    let node_type = current_node.node_type;
    let current_title = current_node.title;
    let current_content = current_node.content;
    let current_attributes = current_node.attributes;

    if current_state != "NEEDS_REVERIFICATION" && current_state != "ACTIVE" {
        return Err(MutationError::Other(format!(
            "Node '{node_id}' in lifecycle state '{current_state}' cannot be reverified (must be in NEEDS_REVERIFICATION or ACTIVE)"
        )));
    }

    // 2. Begin transaction and acquire structural mutation advisory lock FIRST (C-19, D-66)
    let tx = client.transaction().await?;
    tx.execute(
        "SELECT pg_advisory_xact_lock(hashtext('tks_structural_mutation'));",
        &[],
    )
    .await?;

    // 3. Handle optional re-parenting if supplied in updated_attributes
    if let Some(new_parent_id_or_key) = updated_attributes.as_ref().and_then(|a| {
        a.get("reparent_to")
            .or_else(|| a.get("parent_id"))
            .and_then(serde_json::Value::as_str)
    }) {
        let new_parent_node = resolve_node_polymorphic(&tx, new_parent_id_or_key).await?;
        let new_parent_id = new_parent_node.id;

        // Assert DAG cycle prevention before adding new parent edge
        assert_no_dag_cycle(&tx, resolved_id, new_parent_id).await?;

        // Mark previous active upward edges from this node as REVERTED
        tx.execute(
            "UPDATE graph_edges \
             SET lifecycle_state = 'REVERTED' \
             WHERE from_node_id = $1 \
               AND edge_type IN ('FULFILLS', 'CONSTRAINED_BY', 'DERIVED_FROM') \
               AND lifecycle_state = 'ACTIVE';",
            &[&resolved_id],
        )
        .await?;

        // Insert new active upward edge
        let edge_type = if node_type == "SPECIFICATION" {
            "CONSTRAINED_BY"
        } else {
            "FULFILLS"
        };
        insert_structural_edge(
            &tx,
            resolved_id,
            new_parent_id,
            edge_type,
            &actor.agent_id,
            "ACTIVE",
        )
        .await?;
    }

    // 5. Validate that all direct upstream dependencies are in ACTIVE state (INV-1, D-76)
    if node_type != "REQUIREMENT" {
        let parent_rows = tx
            .query(
                "SELECT e.edge_id, e.to_node_id, p.node_key, p.lifecycle_state, p.node_type \
                 FROM graph_edges e \
                 JOIN graph_nodes p ON p.id = e.to_node_id \
                 WHERE e.from_node_id = $1 \
                   AND e.edge_type IN ('FULFILLS', 'CONSTRAINED_BY', 'DERIVED_FROM') \
                   AND e.lifecycle_state = 'ACTIVE';",
                &[&resolved_id],
            )
            .await?;

        if parent_rows.is_empty() {
            return Err(MutationError::DependencyInactive(format!(
                "Node '{node_id}' has zero active upstream dependency edges (Invariant INV-1 violated)"
            )));
        }

        for p_row in &parent_rows {
            let p_state: String = p_row.get("lifecycle_state");
            let p_key: Option<String> = p_row.get("node_key");
            let p_id: Uuid = p_row.get("to_node_id");

            if p_state != "ACTIVE" {
                return Err(MutationError::DependencyInactive(format!(
                    "Direct upstream dependency '{}' ({p_id}) is in state '{p_state}', expected ACTIVE",
                    p_key.as_deref().unwrap_or(&p_id.to_string())
                )));
            }
        }

        // Full ancestor path validation to active requirement root
        let path_valid = validate_ancestor_path(&tx, &[resolved_id], false).await?;
        if !path_valid {
            return Err(MutationError::InvalidAncestorPath(format!(
                "Node '{node_id}' does not have an unbroken directed path terminating at an authorized active requirement"
            )));
        }
    }

    // 6. Layer updated attributes, reset staleness score to 0.0, and clear staleness reason
    let mut merged_attributes = current_attributes.clone();
    if let Some(ref update) = updated_attributes
        && let (Some(base_obj), Some(upd_obj)) =
            (merged_attributes.as_object_mut(), update.as_object())
    {
        for (k, v) in upd_obj {
            base_obj.insert(k.clone(), v.clone());
        }
    }

    if let Some(obj) = merged_attributes.as_object_mut() {
        obj.insert("staleness_score".to_string(), serde_json::json!(0.0));
        obj.remove("staleness_reason");
    }

    let updated_title = updated_attributes
        .as_ref()
        .and_then(|a| a.get("title").and_then(serde_json::Value::as_str))
        .map(String::from)
        .or(current_title);

    let updated_content = updated_attributes
        .as_ref()
        .and_then(|a| a.get("content").and_then(serde_json::Value::as_str))
        .map(String::from)
        .or(current_content);

    tx.execute(
        "UPDATE graph_nodes \
         SET lifecycle_state = 'ACTIVE', \
             title = $2, \
             content = $3, \
             attributes = $4 \
         WHERE id = $1;",
        &[
            &resolved_id,
            &updated_title,
            &updated_content,
            &merged_attributes,
        ],
    )
    .await?;

    // 7. Commit REVERIFIED audit ledger event
    let batch_id = Uuid::new_v4();
    let delta = serde_json::json!({
        "previous_lifecycle_state": current_state,
        "new_lifecycle_state": "ACTIVE",
        "rationale": rationale,
        "staleness_score": 0.0,
        "updated_attributes": updated_attributes,
    });

    let snapshot = serde_json::json!({
        "id": resolved_id,
        "lifecycle_state": "ACTIVE",
        "title": updated_title,
        "content": updated_content,
        "attributes": merged_attributes,
    });

    let event_seq = insert_audit_event(
        &tx,
        batch_id,
        "REVERIFIED",
        resolved_id,
        &node_type,
        &actor.agent_id,
        &actor.actor_type,
        &actor.agent_id,
        delta,
        snapshot,
    )
    .await?;

    // 8. Re-enqueue vector embedding in node_embeddings (D-76)
    let combined_text = format!(
        "{} {}",
        updated_title.as_deref().unwrap_or(""),
        updated_content.as_deref().unwrap_or("")
    );
    let mut hasher = Sha256::new();
    hasher.update(combined_text.as_bytes());
    let content_hash = format!("{:x}", hasher.finalize());

    tx.execute(
        "INSERT INTO node_embeddings (node_id, content_hash, status, scheduled_at) \
         VALUES ($1, $2, 'PENDING', clock_timestamp()) \
         ON CONFLICT (node_id) DO UPDATE SET \
             content_hash = EXCLUDED.content_hash, \
             status = 'PENDING', \
             scheduled_at = clock_timestamp();",
        &[&resolved_id, &content_hash],
    )
    .await?;

    // 9. Commit transaction
    tx.commit().await?;

    Ok(ReverifyResult {
        node_id: resolved_id,
        status: "ACTIVE".to_string(),
        staleness_score: 0.0,
        batch_id,
        event_seq,
    })
}
