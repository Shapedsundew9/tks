//! Storage layer for TKS (The Knowledge Substrate).
//!
//! Provides bare Git document storage and relational graph persistence.

pub mod envelope;
pub mod git;
pub mod repo;
pub mod search;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub use envelope::{
    TopologicalEnvelope, assemble_context_envelope, assemble_topological_envelope,
    query_vector_neighbors, validate_ancestor_path,
};
pub use repo::{StorageRepo, acquire_structural_lock, lookup_node_polymorphic};
pub use search::query_active_requirements;

/// Relational domain entity representing a requirement, specification, task, or verification node.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GraphNode {
    pub id: Uuid,
    pub node_key: Option<String>,
    pub node_type: String,
    pub title: Option<String>,
    pub content: Option<String>,
    pub lifecycle_state: String,
    pub governance_policy: String,
    pub created_by: String,
    pub job_id: Option<Uuid>,
    pub doc_path: Option<String>,
    pub doc_hash: Option<String>,
    pub byte_start: Option<i32>,
    pub byte_end: Option<i32>,
    pub attributes: serde_json::Value,
}

impl GraphNode {
    /// Constructs a `GraphNode` from a `tokio_postgres::Row`.
    #[must_use]
    pub fn from_row(row: &tokio_postgres::Row) -> Self {
        Self {
            id: row.get("id"),
            node_key: row.get("node_key"),
            node_type: row.get("node_type"),
            title: row.get("title"),
            content: row.get("content"),
            lifecycle_state: row.get("lifecycle_state"),
            governance_policy: row.get("governance_policy"),
            created_by: row.get("created_by"),
            job_id: row.get("job_id"),
            doc_path: row.get("doc_path"),
            doc_hash: row.get("doc_hash"),
            byte_start: row.get("byte_start"),
            byte_end: row.get("byte_end"),
            attributes: row.get("attributes"),
        }
    }
}

/// Relational domain entity representing a directed property graph edge.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GraphEdge {
    pub edge_id: Uuid,
    pub from_node_id: Uuid,
    pub to_node_id: Uuid,
    pub edge_type: String,
    pub created_by: String,
    pub lifecycle_state: String,
}

impl GraphEdge {
    /// Constructs a `GraphEdge` from a `tokio_postgres::Row`.
    #[must_use]
    pub fn from_row(row: &tokio_postgres::Row) -> Self {
        Self {
            edge_id: row.get("edge_id"),
            from_node_id: row.get("from_node_id"),
            to_node_id: row.get("to_node_id"),
            edge_type: row.get("edge_type"),
            created_by: row.get("created_by"),
            lifecycle_state: row.get("lifecycle_state"),
        }
    }
}

/// Full-text search result item returned by `query_active_requirements`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SearchResultNode {
    pub id: Uuid,
    pub node_key: Option<String>,
    pub title: Option<String>,
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank: Option<f32>,
}

/// Comprehensive storage error enum covering database, pool, lock, and serialization failures.
#[derive(Debug)]
pub enum StorageError {
    Database(tokio_postgres::Error),
    Pool(deadpool_postgres::PoolError),
    BuildPool(String),
    NotFound(String),
    InvalidState(String),
    LockAcquisitionFailed(String),
    Serialization(serde_json::Error),
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Database(e) => write!(f, "Database error: {e}"),
            Self::Pool(e) => write!(f, "Connection pool error: {e}"),
            Self::BuildPool(msg) => write!(f, "Failed to build connection pool: {msg}"),
            Self::NotFound(msg) => write!(f, "Entity not found: {msg}"),
            Self::InvalidState(msg) => write!(f, "Invalid storage state: {msg}"),
            Self::LockAcquisitionFailed(msg) => write!(f, "Lock acquisition failed: {msg}"),
            Self::Serialization(e) => write!(f, "Serialization error: {e}"),
        }
    }
}

impl std::error::Error for StorageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Database(e) => Some(e),
            Self::Pool(e) => Some(e),
            Self::Serialization(e) => Some(e),
            _ => None,
        }
    }
}

impl From<tokio_postgres::Error> for StorageError {
    fn from(e: tokio_postgres::Error) -> Self {
        Self::Database(e)
    }
}

impl From<deadpool_postgres::PoolError> for StorageError {
    fn from(e: deadpool_postgres::PoolError) -> Self {
        Self::Pool(e)
    }
}

impl From<serde_json::Error> for StorageError {
    fn from(e: serde_json::Error) -> Self {
        Self::Serialization(e)
    }
}
