//! Graph topology generation and live Server-Sent Events (SSE) stream routes
//! for the Real-Time Substrate Web Explorer (WP-3.4, PHASE3-001, TB-1, TB-4).

use std::convert::Infallible;
use std::time::Duration;

use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::gateway::AppState;

/// Query parameters for fetching topological graph elements (`GET /api/v1/explorer/graph`).
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ExplorerGraphQuery {
    /// Optional root node identifier (polymorphic: UUID or canonical `node_key`).
    pub root: Option<String>,
    /// Traversal depth bounding (default: 3, clamped to 1..=5).
    pub depth: Option<u32>,
    /// Whether candidate `DRAFT` nodes and edges should be included in the graph (default: false).
    pub include_drafts: Option<bool>,
}

/// Cytoscape.js compatible graph payload container.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExplorerGraphResponse {
    pub nodes: Vec<CytoscapeNode>,
    pub edges: Vec<CytoscapeEdge>,
}

/// Cytoscape.js node element container.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CytoscapeNode {
    pub data: CytoscapeNodeData,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classes: Option<String>,
}

/// Node attributes formatted for Cytoscape.js canvas rendering and state inspection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CytoscapeNodeData {
    pub id: String,
    pub node_key: String,
    pub node_type: String,
    pub title: String,
    pub lifecycle_state: String,
    pub governance_policy: String,
    pub staleness_score: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attributes: Option<serde_json::Value>,
}

/// Cytoscape.js edge element container.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CytoscapeEdge {
    pub data: CytoscapeEdgeData,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classes: Option<String>,
}

/// Edge attributes formatted for Cytoscape.js directed arrow styling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CytoscapeEdgeData {
    pub id: String,
    pub source: String,
    pub target: String,
    pub edge_type: String,
    pub lifecycle_state: String,
}

/// Handler for `GET /api/v1/explorer/graph`.
///
/// Fetches multi-depth requirement trees, structural derivation edges (`DERIVED_FROM`),
/// fulfillment edges (`FULFILLS`), and constraint links (`CONSTRAINED_BY`) formatted
/// for Cytoscape.js rendering.
pub async fn get_explorer_graph(
    State(state): State<AppState>,
    Query(query): Query<ExplorerGraphQuery>,
) -> Result<Json<ExplorerGraphResponse>, (StatusCode, Json<serde_json::Value>)> {
    let client = state.pool.get().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Pool error: {e}") })),
        )
    })?;

    let include_drafts = query.include_drafts.unwrap_or(false);
    let depth = query.depth.unwrap_or(3).clamp(1, 5) as i32;

    let node_rows = if let Some(ref root_raw) = query.root {
        let root_str = root_raw.trim();
        if !root_str.is_empty() {
            // Resolve polymorphic root node identifier
            let root_id: Uuid = if let Ok(u) = Uuid::parse_str(root_str) {
                let row = client
                    .query_opt(
                        "SELECT id FROM graph_nodes WHERE id = $1 AND ($2::bool OR lifecycle_state != 'DRAFT');",
                        &[&u, &include_drafts],
                    )
                    .await
                    .map_err(|e| {
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({ "error": format!("Query error: {e}") })),
                        )
                    })?;
                match row {
                    Some(r) => r.get("id"),
                    None => {
                        return Err((
                            StatusCode::NOT_FOUND,
                            Json(
                                serde_json::json!({ "error": format!("Root node '{root_str}' not found") }),
                            ),
                        ));
                    }
                }
            } else {
                let row = client
                    .query_opt(
                        "SELECT id FROM graph_nodes WHERE node_key = $1 AND ($2::bool OR lifecycle_state != 'DRAFT') \
                         ORDER BY CASE WHEN lifecycle_state = 'ACTIVE' THEN 0 ELSE 1 END LIMIT 1;",
                        &[&root_str, &include_drafts],
                    )
                    .await
                    .map_err(|e| {
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({ "error": format!("Query error: {e}") })),
                        )
                    })?;
                match row {
                    Some(r) => r.get("id"),
                    None => {
                        return Err((
                            StatusCode::NOT_FOUND,
                            Json(
                                serde_json::json!({ "error": format!("Root node '{root_str}' not found") }),
                            ),
                        ));
                    }
                }
            };

            // Recursive CTE traversing both downward (children) and upward (ancestors) within `depth` hops
            client
                .query(
                    "WITH RECURSIVE traversal AS (
                        SELECT $1::uuid AS node_id, 0 AS depth, ARRAY[$1::uuid] AS visited

                        UNION ALL

                        SELECT
                            CASE WHEN e.to_node_id = t.node_id THEN e.from_node_id ELSE e.to_node_id END AS node_id,
                            t.depth + 1 AS depth,
                            t.visited || (CASE WHEN e.to_node_id = t.node_id THEN e.from_node_id ELSE e.to_node_id END) AS visited
                        FROM graph_edges e
                        JOIN traversal t ON (e.to_node_id = t.node_id OR e.from_node_id = t.node_id)
                        WHERE ($2::bool OR e.lifecycle_state != 'DRAFT')
                          AND t.depth < $3::int
                          AND NOT ((CASE WHEN e.to_node_id = t.node_id THEN e.from_node_id ELSE e.to_node_id END) = ANY(t.visited))
                    )
                    SELECT DISTINCT
                        n.id,
                        n.node_key,
                        n.node_type,
                        n.title,
                        n.lifecycle_state,
                        n.governance_policy,
                        COALESCE((n.attributes->>'staleness_score')::float8, 0.0) AS staleness_score,
                        n.created_by,
                        n.attributes
                    FROM traversal tv
                    JOIN graph_nodes n ON n.id = tv.node_id
                    WHERE ($2::bool OR n.lifecycle_state != 'DRAFT')
                    ORDER BY n.node_key ASC;",
                    &[&root_id, &include_drafts, &depth],
                )
                .await
                .map_err(|e| {
                    let msg = if let Some(d) = e.as_db_error() {
                        format!("{}: {} (detail: {:?})", d.code().code(), d.message(), d.detail())
                    } else {
                        e.to_string()
                    };
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({ "error": format!("Query error: {msg}") })),
                    )
                })?
        } else {
            // Root was empty string
            query_all_nodes(&client, include_drafts).await?
        }
    } else {
        // No root specified, query all active nodes (up to limit)
        query_all_nodes(&client, include_drafts).await?
    };

    let mut nodes = Vec::with_capacity(node_rows.len());
    let mut node_ids = Vec::with_capacity(node_rows.len());

    for row in node_rows {
        let id: Uuid = row.get("id");
        let node_key: String = row
            .get::<_, Option<String>>("node_key")
            .unwrap_or_else(|| id.to_string());
        let node_type: String = row.get("node_type");
        let title: String = row.get::<_, Option<String>>("title").unwrap_or_default();
        let lifecycle_state: String = row.get("lifecycle_state");
        let governance_policy: String = row.get("governance_policy");
        let staleness_score: f64 = row.get("staleness_score");
        let created_by: Option<String> = row.get("created_by");
        let attributes: Option<serde_json::Value> = row.get("attributes");

        let mut classes_vec = vec![
            node_type.to_lowercase(),
            lifecycle_state.to_lowercase(),
            governance_policy.to_lowercase(),
        ];
        if staleness_score > 0.0 {
            classes_vec.push("stale".to_string());
        }
        if lifecycle_state == "NEEDS_REVERIFICATION" {
            classes_vec.push("pulsing".to_string());
        }
        let classes = classes_vec.join(" ");

        node_ids.push(id);
        nodes.push(CytoscapeNode {
            data: CytoscapeNodeData {
                id: id.to_string(),
                node_key,
                node_type,
                title,
                lifecycle_state,
                governance_policy,
                staleness_score,
                created_by,
                attributes,
            },
            classes: Some(classes),
        });
    }

    let mut edges = Vec::new();
    if !node_ids.is_empty() {
        let edge_rows = client
            .query(
                "SELECT
                    edge_id AS id,
                    from_node_id,
                    to_node_id,
                    edge_type,
                    lifecycle_state
                FROM graph_edges
                WHERE ($1::bool OR lifecycle_state != 'DRAFT')
                  AND from_node_id = ANY($2::uuid[])
                  AND to_node_id = ANY($2::uuid[]);",
                &[&include_drafts, &node_ids],
            )
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": format!("Query error: {e}") })),
                )
            })?;

        for row in edge_rows {
            let id: Uuid = row.get("id");
            let from_node_id: Uuid = row.get("from_node_id");
            let to_node_id: Uuid = row.get("to_node_id");
            let edge_type: String = row.get("edge_type");
            let lifecycle_state: String = row.get("lifecycle_state");

            let classes = format!(
                "{} {}",
                edge_type.to_lowercase(),
                lifecycle_state.to_lowercase()
            );

            edges.push(CytoscapeEdge {
                data: CytoscapeEdgeData {
                    id: id.to_string(),
                    source: from_node_id.to_string(),
                    target: to_node_id.to_string(),
                    edge_type,
                    lifecycle_state,
                },
                classes: Some(classes),
            });
        }
    }

    Ok(Json(ExplorerGraphResponse { nodes, edges }))
}

/// Helper executing global node query when no specific root is requested.
async fn query_all_nodes(
    client: &deadpool_postgres::Client,
    include_drafts: bool,
) -> Result<Vec<tokio_postgres::Row>, (StatusCode, Json<serde_json::Value>)> {
    client
        .query(
            "SELECT
                id,
                node_key,
                node_type,
                title,
                lifecycle_state,
                governance_policy,
                COALESCE((attributes->>'staleness_score')::float8, 0.0) AS staleness_score,
                created_by,
                attributes
            FROM graph_nodes
            WHERE ($1::bool OR lifecycle_state != 'DRAFT')
            ORDER BY node_key ASC, id ASC
            LIMIT 1000;",
            &[&include_drafts],
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Query error: {e}") })),
            )
        })
}

/// Handler for `GET /api/v1/explorer/events`.
///
/// Connects incoming clients to the `GraphEventBus` via Server-Sent Events (SSE),
/// streaming real-time JSON events when mutations, invalidations, or promotions occur.
pub async fn stream_explorer_events(
    State(state): State<AppState>,
) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    let rx = state.event_bus.subscribe();
    let stream = futures_util::stream::unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(event) => match serde_json::to_string(&event) {
                    Ok(json) => {
                        let sse_event = Event::default().event("graph_event").data(json);
                        return Some((Ok::<_, Infallible>(sse_event), rx));
                    }
                    Err(e) => {
                        tracing::warn!("Failed to serialize GraphChangeEvent for SSE: {e}");
                        continue;
                    }
                },
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!("SSE subscriber lagged, skipped {skipped} messages");
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    return None;
                }
            }
        }
    });

    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}
