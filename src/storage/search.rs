//! Native PostgreSQL full-text search with canonical key query sanitization (TB-7).

use regex::Regex;
use std::sync::LazyLock;
use tokio_postgres::Client;

use crate::storage::{SearchResultNode, StorageError};

static CANONICAL_KEY_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z]+-[0-9A-Za-z-]+$").expect("Invalid regex"));

/// Queries active requirements using native PostgreSQL full-text search against `search_tsv`.
///
/// Sanitizes queries matching canonical identifier patterns (`^[A-Za-z]+-[0-9A-Za-z-]+$`)
/// by using exact/prefix key matching combined with `plainto_tsquery` to prevent hyphen-negation
/// syntax corruption. Natural language queries execute using `websearch_to_tsquery`.
///
/// # Errors
///
/// Returns `StorageError` if the database query fails.
pub async fn query_active_requirements(
    client: &Client,
    query: &str,
    limit: u32,
) -> Result<Vec<SearchResultNode>, StorageError> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }

    let limit_i64 = if limit == 0 { 20 } else { limit.min(100) } as i64;

    let rows = if CANONICAL_KEY_REGEX.is_match(query) {
        client
            .query(
                "SELECT id, node_key, title, content, \
                 (CASE WHEN node_key ILIKE $1 THEN 100.0::real \
                       WHEN node_key ILIKE $1 || '%' THEN 50.0::real \
                       ELSE 0.0::real END + \
                  ts_rank(search_tsv, plainto_tsquery('english', $1))) AS rank \
                 FROM graph_nodes \
                 WHERE lifecycle_state = 'ACTIVE' \
                   AND (node_key ILIKE $1 || '%' OR search_tsv @@ plainto_tsquery('english', $1)) \
                 ORDER BY rank DESC \
                 LIMIT $2;",
                &[&query, &limit_i64],
            )
            .await?
    } else {
        client
            .query(
                "SELECT id, node_key, title, content, \
                 ts_rank(search_tsv, websearch_to_tsquery('english', $1)) AS rank \
                 FROM graph_nodes \
                 WHERE lifecycle_state = 'ACTIVE' \
                   AND (search_tsv @@ websearch_to_tsquery('english', $1) OR node_key ILIKE '%' || $1 || '%') \
                 ORDER BY rank DESC \
                 LIMIT $2;",
                &[&query, &limit_i64],
            )
            .await?
    };

    let results = rows
        .iter()
        .map(|r| SearchResultNode {
            id: r.get("id"),
            node_key: r.get("node_key"),
            title: r.get("title"),
            content: r.get("content"),
            rank: r.try_get("rank").ok(),
        })
        .collect();

    Ok(results)
}
