//! Multi-agent branch-isolated workspace containers and candidate task elaboration (WP-3.2, PHASE3-003).
//!
//! Enables concurrent external coding agents to create isolated scratchpad branches
//! snapshotting live substrate state (`base_event_seq`), allowing speculative graph
//! elaboration (candidate tasks, spec updates, and edge attachments) without taking
//! the global structural mutation advisory lock `pg_advisory_xact_lock` or creating
//! lock contention on uncommitted changes (INV-1, INV-5, INV-7, D-8, D-20, D-63).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_postgres::Client;
use uuid::Uuid;

use crate::gateway::auth::AuthenticatedAgent;
use crate::storage::governance::{GovernanceAction, GovernancePolicy, evaluate_governance_action};
use crate::storage::mutation::{ElaboratedTaskResult, MutationError, insert_audit_event};
use crate::storage::{GraphEdge, GraphNode, StorageError};

pub use crate::storage::conflict::{
    MergeConflict, MergePreview, PromotionResult, RebaseResult, analyze_workspace_merge,
    analyze_workspace_merge_client, find_active_parent_replacement, merge_workspace,
    merge_workspace_client, promote_workspace, promote_workspace_client, sync_workspace_rebase,
    sync_workspace_rebase_client,
};

/// Relational representation of an isolated agent workspace container.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceRecord {
    pub id: Uuid,
    pub workspace_name: String,
    pub owner_agent: String,
    pub base_event_seq: i64,
    pub status: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub attributes: Value,
}

impl WorkspaceRecord {
    /// Constructs a `WorkspaceRecord` from a `tokio_postgres::Row`.
    #[must_use]
    pub fn from_row(row: &tokio_postgres::Row) -> Self {
        Self {
            id: row.get("id"),
            workspace_name: row.get("workspace_name"),
            owner_agent: row.get("owner_agent"),
            base_event_seq: row.get("base_event_seq"),
            status: row.get("status"),
            created_at: row.get("created_at"),
            attributes: row.get("attributes"),
        }
    }
}

/// Creates a new branch-isolated workspace container snapshotting the current substrate event sequence.
///
/// # Errors
///
/// Returns `MutationError::Database` on SQL failure or `MutationError::Pool` on connection failure.
pub async fn create_workspace(
    client: &mut deadpool_postgres::Client,
    name: &str,
    actor: &AuthenticatedAgent,
) -> Result<WorkspaceRecord, MutationError> {
    create_workspace_with_attributes(client, name, None, actor).await
}

/// Creates a workspace container with optional initial attributes.
///
/// # Errors
///
/// Returns `MutationError::Database` on SQL failure.
pub async fn create_workspace_with_attributes(
    client: &mut deadpool_postgres::Client,
    name: &str,
    attributes: Option<Value>,
    actor: &AuthenticatedAgent,
) -> Result<WorkspaceRecord, MutationError> {
    create_workspace_client(client, name, attributes, actor).await
}

/// Variant of `create_workspace` operating on a standard `tokio_postgres::Client`.
///
/// # Errors
///
/// Returns `MutationError::Database` on SQL failure.
pub async fn create_workspace_client(
    client: &mut Client,
    name: &str,
    attributes: Option<Value>,
    actor: &AuthenticatedAgent,
) -> Result<WorkspaceRecord, MutationError> {
    let tx = client.transaction().await?;

    // 1. Capture current maximum event_seq from audit_ledger as base_event_seq (PHASE3-003)
    let row = tx
        .query_one("SELECT COALESCE(MAX(event_seq), 0) FROM audit_ledger;", &[])
        .await?;
    let base_event_seq: i64 = row.get(0);

    let workspace_id = Uuid::new_v4();
    let attrs = attributes.unwrap_or_else(|| serde_json::json!({}));

    // 2. Register workspace container
    let row = tx
        .query_one(
            "INSERT INTO workspaces (id, workspace_name, owner_agent, base_event_seq, status, created_at, attributes) \
             VALUES ($1, $2, $3, $4, 'ACTIVE', NOW(), $5) \
             RETURNING id, workspace_name, owner_agent, base_event_seq, status, created_at, attributes;",
            &[&workspace_id, &name, &actor.agent_id, &base_event_seq, &attrs],
        )
        .await?;

    let record = WorkspaceRecord::from_row(&row);

    // 3. Log workspace creation in audit ledger
    let delta = serde_json::json!({
        "workspace_name": name,
        "base_event_seq": base_event_seq,
        "owner_agent": actor.agent_id,
    });
    let snapshot = serde_json::to_value(&record)?;

    insert_audit_event(
        &tx,
        workspace_id,
        "WORKSPACE_CREATED",
        workspace_id,
        "WORKSPACE",
        &actor.agent_id,
        &actor.actor_type,
        &actor.token_fingerprint(),
        delta,
        snapshot,
    )
    .await?;

    tx.commit().await?;
    Ok(record)
}

/// Inspects a workspace container by its identifier.
///
/// # Errors
///
/// Returns `MutationError::Database` on SQL failure.
pub async fn get_workspace(
    client: &mut deadpool_postgres::Client,
    workspace_id: Uuid,
) -> Result<Option<WorkspaceRecord>, MutationError> {
    get_workspace_client(client, workspace_id).await
}

/// Variant of `get_workspace` operating on a standard client.
///
/// # Errors
///
/// Returns `MutationError::Database` on SQL failure.
pub async fn get_workspace_client(
    client: &Client,
    workspace_id: Uuid,
) -> Result<Option<WorkspaceRecord>, MutationError> {
    let row_opt = client
        .query_opt(
            "SELECT id, workspace_name, owner_agent, base_event_seq, status, created_at, attributes \
             FROM workspaces WHERE id = $1;",
            &[&workspace_id],
        )
        .await?;
    Ok(row_opt.map(|r| WorkspaceRecord::from_row(&r)))
}

/// Lists workspace containers, optionally filtered by owner agent identifier.
///
/// # Errors
///
/// Returns `MutationError::Database` on SQL failure.
pub async fn list_workspaces(
    client: &mut deadpool_postgres::Client,
    owner_agent: Option<&str>,
) -> Result<Vec<WorkspaceRecord>, MutationError> {
    list_workspaces_client(client, owner_agent).await
}

/// Variant of `list_workspaces` operating on a standard client.
///
/// # Errors
///
/// Returns `MutationError::Database` on SQL failure.
pub async fn list_workspaces_client(
    client: &Client,
    owner_agent: Option<&str>,
) -> Result<Vec<WorkspaceRecord>, MutationError> {
    let rows = match owner_agent {
        Some(agent) => {
            client
                .query(
                    "SELECT id, workspace_name, owner_agent, base_event_seq, status, created_at, attributes \
                     FROM workspaces WHERE owner_agent = $1 ORDER BY created_at DESC;",
                    &[&agent],
                )
                .await?
        }
        None => {
            client
                .query(
                    "SELECT id, workspace_name, owner_agent, base_event_seq, status, created_at, attributes \
                     FROM workspaces ORDER BY created_at DESC;",
                    &[],
                )
                .await?
        }
    };
    Ok(rows.iter().map(WorkspaceRecord::from_row).collect())
}

/// Discards a workspace container and marks all associated candidate draft nodes and edges as discarded.
///
/// # Errors
///
/// - Returns `MutationError::NotFound` if workspace is missing.
/// - Returns `MutationError::GovernanceRejected` if caller is not authorized to discard the workspace.
/// - Returns `MutationError::Database` on SQL failure.
pub async fn discard_workspace(
    client: &mut deadpool_postgres::Client,
    workspace_id: Uuid,
    actor: &AuthenticatedAgent,
) -> Result<WorkspaceRecord, MutationError> {
    discard_workspace_client(client, workspace_id, actor).await
}

/// Variant of `discard_workspace` operating on a standard `tokio_postgres::Client`.
///
/// # Errors
///
/// - Returns `MutationError::NotFound` if workspace is missing.
/// - Returns `MutationError::GovernanceRejected` if caller is not authorized.
/// - Returns `MutationError::Database` on SQL failure.
pub async fn discard_workspace_client(
    client: &mut Client,
    workspace_id: Uuid,
    actor: &AuthenticatedAgent,
) -> Result<WorkspaceRecord, MutationError> {
    let tx = client.transaction().await?;

    let row = tx
        .query_opt(
            "SELECT id, workspace_name, owner_agent, base_event_seq, status, created_at, attributes \
             FROM workspaces WHERE id = $1 FOR UPDATE;",
            &[&workspace_id],
        )
        .await?;

    let ws = row
        .map(|r| WorkspaceRecord::from_row(&r))
        .ok_or_else(|| MutationError::NotFound(format!("Workspace '{workspace_id}' not found")))?;

    if ws.owner_agent != actor.agent_id && actor.actor_type != "HUMAN" && actor.agent_id != "system"
    {
        return Err(MutationError::GovernanceRejected(format!(
            "Agent '{}' is not authorized to discard workspace owned by '{}'",
            actor.agent_id, ws.owner_agent
        )));
    }

    let updated_row = tx
        .query_one(
            "UPDATE workspaces SET status = 'DISCARDED' WHERE id = $1 \
             RETURNING id, workspace_name, owner_agent, base_event_seq, status, created_at, attributes;",
            &[&workspace_id],
        )
        .await?;

    let ws_id_str = workspace_id.to_string();

    // Mark candidate draft nodes and edges as ARCHIVED/REVERTED
    tx.execute(
        "UPDATE graph_nodes SET lifecycle_state = 'ARCHIVED' \
         WHERE (attributes->>'workspace_id') = $1 AND lifecycle_state = 'DRAFT';",
        &[&ws_id_str],
    )
    .await?;

    tx.execute(
        "UPDATE graph_edges SET lifecycle_state = 'REVERTED' \
         WHERE (attributes->>'workspace_id') = $1 AND lifecycle_state = 'DRAFT';",
        &[&ws_id_str],
    )
    .await?;

    let result_record = WorkspaceRecord::from_row(&updated_row);

    insert_audit_event(
        &tx,
        workspace_id,
        "WORKSPACE_DISCARDED",
        workspace_id,
        "WORKSPACE",
        &actor.agent_id,
        &actor.actor_type,
        &actor.token_fingerprint(),
        serde_json::json!({ "status": "DISCARDED" }),
        serde_json::to_value(&result_record)?,
    )
    .await?;

    tx.commit().await?;
    Ok(result_record)
}

/// Autonomously elaborates a candidate execution task inside a branch-isolated workspace container.
///
/// Creates candidate nodes and edges stamped with `attributes->'workspace_id' = workspace_id` in
/// `DRAFT` state, completely bypassing the global structural mutation advisory lock
/// `pg_advisory_xact_lock(hashtext('tks_structural_mutation'))` (INV-1, INV-5, INV-7, D-8, D-20).
///
/// # Errors
///
/// - Returns `MutationError::NotFound` if workspace or parent node is missing.
/// - Returns `MutationError::GovernanceRejected` if workspace is inactive, caller is not owner, or parent policy forbids elaboration.
/// - Returns `MutationError::GovernanceLocked` if parent node is `LOCKED`.
/// - Returns `MutationError::Database` on SQL failure.
pub async fn elaborate_in_workspace(
    client: &mut deadpool_postgres::Client,
    workspace_id: Uuid,
    parent_id: &str,
    title: &str,
    content: Option<&str>,
    attributes: Option<Value>,
    actor: &AuthenticatedAgent,
) -> Result<ElaboratedTaskResult, MutationError> {
    elaborate_in_workspace_client(
        client,
        workspace_id,
        parent_id,
        title,
        content,
        attributes,
        actor,
    )
    .await
}

/// Variant of `elaborate_in_workspace` operating on a standard `tokio_postgres::Client`.
///
/// # Errors
///
/// Returns `MutationError` on validation or persistence error.
pub async fn elaborate_in_workspace_client(
    client: &mut Client,
    workspace_id: Uuid,
    parent_id: &str,
    title: &str,
    content: Option<&str>,
    attributes: Option<Value>,
    actor: &AuthenticatedAgent,
) -> Result<ElaboratedTaskResult, MutationError> {
    let tx = client.transaction().await?;
    // Note: STRICTLY DO NOT ACQUIRE pg_advisory_xact_lock (bypassing global structural advisory lock)

    // 1. Verify workspace exists, is ACTIVE, and caller is authorized
    let ws_row = tx
        .query_opt(
            "SELECT id, workspace_name, owner_agent, base_event_seq, status, created_at, attributes \
             FROM workspaces WHERE id = $1;",
            &[&workspace_id],
        )
        .await?;

    let ws = ws_row
        .map(|r| WorkspaceRecord::from_row(&r))
        .ok_or_else(|| MutationError::NotFound(format!("Workspace '{workspace_id}' not found")))?;

    if ws.status != "ACTIVE" {
        return Err(MutationError::GovernanceRejected(format!(
            "Workspace '{workspace_id}' is not ACTIVE (status: '{}')",
            ws.status
        )));
    }

    if ws.owner_agent != actor.agent_id && actor.actor_type != "HUMAN" && actor.agent_id != "system"
    {
        return Err(MutationError::GovernanceRejected(format!(
            "Agent '{}' is not authorized to elaborate in workspace owned by '{}'",
            actor.agent_id, ws.owner_agent
        )));
    }

    // 2. Resolve parent node polymorphically (active in substrate OR candidate draft in this workspace)
    let parent = resolve_parent_for_workspace(&tx, parent_id, workspace_id).await?;

    // 3. Evaluate governance action
    let parent_policy = parent
        .governance_policy
        .parse::<GovernancePolicy>()
        .map_err(MutationError::GovernanceRejected)?;

    evaluate_governance_action(
        parent_policy,
        GovernanceAction::ElaborateTask,
        &parent.node_type,
        &parent.lifecycle_state,
    )?;

    if parent.lifecycle_state != "ACTIVE" && parent.lifecycle_state != "DRAFT" {
        return Err(MutationError::GovernanceRejected(format!(
            "Parent node is in state '{}', expected 'ACTIVE' or workspace 'DRAFT' for task elaboration",
            parent.lifecycle_state
        )));
    }

    // 4. Inherited governance policy and attributes
    let inherited_policy = attributes
        .as_ref()
        .and_then(|a| a.get("governance_policy"))
        .and_then(|v| v.as_str())
        .map(|s| {
            s.parse::<GovernancePolicy>()
                .map_err(MutationError::GovernanceRejected)
        })
        .transpose()?
        .unwrap_or(GovernancePolicy::AutonomousElaboration);

    let node_key = attributes
        .as_ref()
        .and_then(|a| a.get("node_key"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let mut task_attributes = attributes.unwrap_or_else(|| serde_json::json!({}));
    if !task_attributes.is_object() {
        task_attributes = serde_json::json!({});
    }
    if task_attributes.get("execution_status").is_none() {
        task_attributes["execution_status"] = serde_json::json!("OPEN");
    }
    task_attributes["workspace_id"] = serde_json::json!(workspace_id.to_string());
    task_attributes["parent_node_id"] = serde_json::json!(parent.id.to_string());
    task_attributes["created_by"] = serde_json::json!(actor.agent_id);

    let task_id = Uuid::new_v4();

    // 5. Insert TASK candidate directly into graph_nodes in DRAFT state
    let row = tx
        .query_one(
            "INSERT INTO graph_nodes ( \
                 id, node_key, node_type, title, content, lifecycle_state, \
                 governance_policy, created_by, attributes \
             ) VALUES ($1, $2, 'TASK', $3, $4, 'DRAFT', $5, $6, $7) \
             RETURNING id, node_key, node_type, title, content, lifecycle_state, \
                       governance_policy, created_by, job_id, doc_path, doc_hash, \
                       byte_start, byte_end, attributes;",
            &[
                &task_id,
                &node_key,
                &title,
                &content,
                &inherited_policy.as_str(),
                &actor.agent_id,
                &task_attributes,
            ],
        )
        .await?;

    let task_node = GraphNode::from_row(&row);

    // 6. Insert upward candidate edge from child to parent in DRAFT state
    let edge_type = task_attributes
        .get("edge_type")
        .and_then(|v| v.as_str())
        .unwrap_or("FULFILLS");
    let edge_id = Uuid::new_v4();
    let edge_attrs = serde_json::json!({
        "workspace_id": workspace_id.to_string(),
    });

    tx.execute(
        "INSERT INTO graph_edges (edge_id, from_node_id, to_node_id, edge_type, created_by, lifecycle_state, attributes) \
         VALUES ($1, $2, $3, $4, $5, 'DRAFT', $6);",
        &[
            &edge_id,
            &task_id,
            &parent.id,
            &edge_type,
            &actor.agent_id,
            &edge_attrs,
        ],
    )
    .await?;

    // 7. Insert audit event into audit_ledger
    let batch_id = Uuid::new_v4();
    let delta = serde_json::json!({
        "parent_id": parent.id,
        "workspace_id": workspace_id,
        "title": title,
        "content": content,
        "edge_id": edge_id,
        "edge_type": edge_type,
        "governance_policy": inherited_policy.as_str(),
    });
    let snapshot = serde_json::to_value(&task_node)?;

    let event_seq = insert_audit_event(
        &tx,
        batch_id,
        "WORKSPACE_TASK_ELABORATED",
        task_id,
        "TASK",
        &actor.agent_id,
        &actor.actor_type,
        &actor.token_fingerprint(),
        delta,
        snapshot,
    )
    .await?;

    tx.commit().await?;

    Ok(ElaboratedTaskResult {
        task_id,
        node_key,
        status: "OPEN".to_string(),
        lifecycle_state: "DRAFT".to_string(),
        governance_policy: inherited_policy.as_str().to_string(),
        edge_id,
        batch_id,
        event_seq,
        node: task_node,
    })
}

/// Polymorphically resolves parent node: checks active substrate nodes OR candidate drafts
/// belonging to the specified `workspace_id`.
async fn resolve_parent_for_workspace(
    client: &tokio_postgres::Transaction<'_>,
    identifier: &str,
    workspace_id: Uuid,
) -> Result<GraphNode, MutationError> {
    let identifier = identifier.trim();
    if identifier.is_empty() {
        return Err(MutationError::NotFound(
            "Empty parent identifier".to_string(),
        ));
    }

    let ws_id_str = workspace_id.to_string();

    if let Ok(uuid) = Uuid::parse_str(identifier) {
        let row_opt = client
            .query_opt(
                "SELECT id, node_key, node_type, title, content, lifecycle_state, \
                 governance_policy, created_by, job_id, doc_path, doc_hash, \
                 byte_start, byte_end, attributes \
                 FROM graph_nodes \
                 WHERE id = $1 \
                   AND (lifecycle_state = 'ACTIVE' OR (lifecycle_state = 'DRAFT' AND (attributes->>'workspace_id') = $2));",
                &[&uuid, &ws_id_str],
            )
            .await?;
        row_opt.map(|r| GraphNode::from_row(&r)).ok_or_else(|| {
            MutationError::NotFound(format!(
                "Parent node '{identifier}' not found in substrate or workspace '{workspace_id}'"
            ))
        })
    } else {
        let row_opt = client
            .query_opt(
                "SELECT id, node_key, node_type, title, content, lifecycle_state, \
                 governance_policy, created_by, job_id, doc_path, doc_hash, \
                 byte_start, byte_end, attributes \
                 FROM graph_nodes \
                 WHERE node_key = $1 \
                   AND (lifecycle_state = 'ACTIVE' OR (lifecycle_state = 'DRAFT' AND (attributes->>'workspace_id') = $2));",
                &[&identifier, &ws_id_str],
            )
            .await?;
        row_opt
            .map(|r| GraphNode::from_row(&r))
            .ok_or_else(|| {
                MutationError::NotFound(format!(
                    "Parent node with key '{identifier}' not found in substrate or workspace '{workspace_id}'"
                ))
            })
    }
}

/// Retrieves all candidate draft nodes elaborated in the specified workspace container.
///
/// # Errors
///
/// Returns `StorageError::Database` on SQL failure.
pub async fn get_workspace_nodes(
    client: &Client,
    workspace_id: Uuid,
) -> Result<Vec<GraphNode>, StorageError> {
    let ws_id_str = workspace_id.to_string();
    let rows = client
        .query(
            "SELECT id, node_key, node_type, title, content, lifecycle_state, \
             governance_policy, created_by, job_id, doc_path, doc_hash, \
             byte_start, byte_end, attributes \
             FROM graph_nodes \
             WHERE (attributes->>'workspace_id') = $1 AND lifecycle_state = 'DRAFT' \
             ORDER BY created_by, title;",
            &[&ws_id_str],
        )
        .await?;
    Ok(rows.iter().map(GraphNode::from_row).collect())
}

/// Retrieves all candidate draft edges attached in the specified workspace container.
///
/// # Errors
///
/// Returns `StorageError::Database` on SQL failure.
pub async fn get_workspace_edges(
    client: &Client,
    workspace_id: Uuid,
) -> Result<Vec<GraphEdge>, StorageError> {
    let ws_id_str = workspace_id.to_string();
    let rows = client
        .query(
            "SELECT edge_id, from_node_id, to_node_id, edge_type, created_by, lifecycle_state, attributes \
             FROM graph_edges \
             WHERE (attributes->>'workspace_id') = $1 AND lifecycle_state = 'DRAFT' \
             ORDER BY edge_type, created_by;",
            &[&ws_id_str],
        )
        .await?;
    Ok(rows.iter().map(GraphEdge::from_row).collect())
}
