//! Scale traversal benchmark verifying SLA-2 Context Envelope Assembly Latency
//! strictly <100ms at 10^5 nodes in PostgreSQL (WP-2.5, SLA-2, INV-6).

use std::time::Instant;
use tks::db;
use tks::storage::assemble_topological_envelope;
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== SLA-2 Scale Traversal Benchmark: Context Envelope Assembly at 10^5 Nodes ===");
    println!(
        "Target SLA: Context Envelope Assembly Latency strictly < 100 ms at 10^5 nodes in PostgreSQL (SLA-2)\n"
    );

    let test_url = db::ensure_test_database_ready()
        .await
        .map_err(|e| e.to_string())?;
    let (client, _handle) = db::connect(&test_url).await?;

    let schema_name = format!("bench_scale_{}", Uuid::new_v4().simple());
    println!("Creating isolated benchmark schema: {schema_name}...");

    client
        .batch_execute(&format!(
            "CREATE SCHEMA {schema_name};
             SET search_path TO {schema_name}, public;

             CREATE TABLE {schema_name}.graph_nodes (
                 id UUID PRIMARY KEY,
                 node_key VARCHAR(128),
                 node_type VARCHAR(32) NOT NULL,
                 title VARCHAR(512),
                 content TEXT,
                 lifecycle_state VARCHAR(20) NOT NULL DEFAULT 'ACTIVE',
                 governance_policy VARCHAR(32) NOT NULL DEFAULT 'AUTONOMOUS_ELABORATION',
                 created_by VARCHAR(64) NOT NULL DEFAULT 'bench',
                 job_id UUID,
                 doc_path VARCHAR(255),
                 doc_hash VARCHAR(64),
                 byte_start INT,
                 byte_end INT,
                 attributes JSONB NOT NULL DEFAULT '{{}}'
             );

             CREATE TABLE {schema_name}.graph_edges (
                 edge_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                 from_node_id UUID NOT NULL,
                 to_node_id UUID NOT NULL,
                 edge_type VARCHAR(32) NOT NULL,
                 created_by VARCHAR(64) NOT NULL DEFAULT 'bench',
                 lifecycle_state VARCHAR(20) NOT NULL DEFAULT 'ACTIVE',
                 attributes JSONB NOT NULL DEFAULT '{{}}'
             );

             CREATE UNIQUE INDEX idx_{schema_name}_nodes_key ON {schema_name}.graph_nodes(node_key) WHERE lifecycle_state = 'ACTIVE';
             CREATE UNIQUE INDEX idx_{schema_name}_edges_active_unique ON {schema_name}.graph_edges (from_node_id, to_node_id, edge_type) WHERE lifecycle_state = 'ACTIVE';
             CREATE INDEX idx_{schema_name}_edges_to_node_active ON {schema_name}.graph_edges (to_node_id, from_node_id, edge_type) WHERE lifecycle_state = 'ACTIVE';"
        ))
        .await?;

    println!("Populating 100,000 synthetic nodes across hierarchical depths 1 to 6...");
    let start_populate = Instant::now();

    client
        .batch_execute(&format!(
            "-- 1. Populate 100,000 nodes across depths 1-6
             INSERT INTO {schema_name}.graph_nodes (id, node_key, node_type, title, content, lifecycle_state)
             SELECT
                 (lpad(to_hex(i), 32, '0'))::uuid,
                 'BENCH-NODE-' || i,
                 CASE
                     WHEN i <= 200 THEN 'REQUIREMENT'
                     WHEN i <= 30000 THEN 'SPECIFICATION'
                     ELSE 'TASK'
                 END,
                 'Benchmark Synthetic Node ' || i,
                 'Synthetic intent content for benchmark node ' || i || ' evaluating scale context envelope traversal.',
                 'ACTIVE'
             FROM generate_series(1, 100000) AS s(i);

             -- 2. Populate hierarchical upward edges (depths 2-6)
             INSERT INTO {schema_name}.graph_edges (from_node_id, to_node_id, edge_type)
             SELECT
                 (lpad(to_hex(i), 32, '0'))::uuid,
                 (lpad(to_hex(
                     CASE
                         WHEN i <= 2000 THEN ((i - 201) % 200) + 1
                         WHEN i <= 10000 THEN ((i - 2001) % 1800) + 201
                         WHEN i <= 30000 THEN ((i - 10001) % 8000) + 2001
                         WHEN i <= 65000 THEN ((i - 30001) % 20000) + 10001
                         ELSE ((i - 65001) % 35000) + 30001
                     END
                 ), 32, '0'))::uuid,
                 CASE
                     WHEN i <= 10000 THEN 'DERIVED_FROM'
                     WHEN i <= 65000 THEN 'FULFILLS'
                     ELSE 'DERIVED_FROM'
                 END
             FROM generate_series(201, 100000) AS s(i);

             -- 3. Populate cross-cutting constraints
             INSERT INTO {schema_name}.graph_edges (from_node_id, to_node_id, edge_type)
             SELECT
                 (lpad(to_hex(i), 32, '0'))::uuid,
                 (lpad(to_hex((i % 200) + 1), 32, '0'))::uuid,
                 'CONSTRAINED_BY'
             FROM generate_series(10001, 20000) AS s(i);

             ANALYZE {schema_name}.graph_nodes;
             ANALYZE {schema_name}.graph_edges;"
        ))
        .await?;

    let populate_elapsed = start_populate.elapsed();
    println!(
        "Graph population completed in {:.2?} (100,000 nodes, 109,800 edges).",
        populate_elapsed
    );

    // Verify row counts
    let node_count: i64 = client
        .query_one(
            &format!("SELECT count(*) FROM {schema_name}.graph_nodes;"),
            &[],
        )
        .await?
        .get(0);
    let edge_count: i64 = client
        .query_one(
            &format!("SELECT count(*) FROM {schema_name}.graph_edges;"),
            &[],
        )
        .await?
        .get(0);
    assert_eq!(node_count, 100_000, "Node count must be exactly 100,000");
    assert_eq!(edge_count, 109_800, "Edge count must be exactly 109,800");

    // Generate 100 pseudo-random target indices spread across depths 2-6
    let mut target_indices = Vec::with_capacity(100);
    for sample in 0..100 {
        // Pseudo-random linear congruential sampling with emphasis on deeper execution nodes
        let idx = (sample * 997 + 2503) % 100_000 + 1;
        target_indices.push(idx);
    }

    // Warm-up 5 queries
    println!("Executing warm-up iterations...");
    for &idx in &target_indices[0..5] {
        let hex_id = format!("{:032x}", idx);
        let target_uuid = Uuid::parse_str(&hex_id)?;
        let _ = assemble_topological_envelope(&client, target_uuid, 3, None).await?;
    }

    // Benchmark 100 target queries
    println!(
        "Measuring assemble_topological_envelope latency across 100 random targets (depth=3)..."
    );
    let mut latencies_ms = Vec::with_capacity(100);

    for &idx in &target_indices {
        let hex_id = format!("{:032x}", idx);
        let target_uuid = Uuid::parse_str(&hex_id)?;

        let start = Instant::now();
        let envelope = assemble_topological_envelope(&client, target_uuid, 3, None).await?;
        let elapsed = start.elapsed();
        let elapsed_ms = elapsed.as_secs_f64() * 1000.0;

        assert_eq!(envelope.target_node.id, target_uuid);
        assert!(
            envelope.total_nodes >= 1,
            "Context envelope must contain at least target node"
        );
        latencies_ms.push(elapsed_ms);
    }

    // Calculate percentiles
    latencies_ms.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = latencies_ms.len();
    let min_ms = latencies_ms[0];
    let max_ms = latencies_ms[n - 1];
    let mean_ms = latencies_ms.iter().sum::<f64>() / n as f64;
    let p50_ms = latencies_ms[n / 2];
    let p90_ms = latencies_ms[(n * 90) / 100];
    let p95_ms = latencies_ms[(n * 95) / 100];
    let p99_ms = latencies_ms[(n * 99) / 100];

    // Clean up temporary schema
    println!("Cleaning up temporary benchmark schema...");
    let _ = client
        .execute(&format!("DROP SCHEMA {schema_name} CASCADE;"), &[])
        .await;

    println!("\n================================================================================");
    println!("  SLA-2 CONTEXT ENVELOPE SCALE BENCHMARK SCORECARD (10^5 NODES)");
    println!("================================================================================");
    println!("  Evaluated Targets:        100 nodes");
    println!("  Graph Scale:              100,000 nodes | 109,800 edges | Depths 1-6");
    println!("  Mean Latency:             {:.3} ms", mean_ms);
    println!("  Median (p50) Latency:     {:.3} ms", p50_ms);
    println!("  p90 Latency:              {:.3} ms", p90_ms);
    println!(
        "  p95 Latency:              {:.3} ms  (SLA-2 Threshold: < 100.0 ms)",
        p95_ms
    );
    println!("  p99 Latency:              {:.3} ms", p99_ms);
    println!(
        "  Min / Max Latency:        {:.3} ms / {:.3} ms",
        min_ms, max_ms
    );
    println!("================================================================================");

    if p95_ms < 100.0 {
        println!(
            "  STATUS: PASS (SLA-2 satisfied: {:.3} ms < 100.0 ms)\n",
            p95_ms
        );
    } else {
        println!(
            "  STATUS: FAIL (SLA-2 breached: {:.3} ms >= 100.0 ms)\n",
            p95_ms
        );
        panic!(
            "SLA-2 benchmark failure: p95 latency {:.3} ms exceeded 100 ms threshold",
            p95_ms
        );
    }

    Ok(())
}
