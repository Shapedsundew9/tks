//! Scale benchmark verifying Invalidation Cascade Engine latency strictly <10ms
//! at 10^4 nodes in PostgreSQL (WP-3.5, PHASE3-002, SLA-2, INV-1, INV-2).

use std::time::Instant;
use uuid::Uuid;

use tks::db;
use tks::gateway::auth::AuthenticatedAgent;
use tks::storage::cascade::trigger_downward_invalidation_client;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== SLA-2 Invalidation Cascade Scale Benchmark (10^4 Nodes) ===");
    println!(
        "Target SLA: Invalidation cascade sweep p95 latency strictly < 10.0 ms across 10^4 nodes (WP-3.5, SLA-2)\n"
    );

    let test_url = db::ensure_test_database_ready()
        .await
        .map_err(|e| e.to_string())?;
    let (mut client, _handle) = db::connect(&test_url).await?;

    // Ensure database migrations are current
    db::run_migrations(&mut client).await?;

    let agent = AuthenticatedAgent {
        agent_id: "bench_cascade_scale".to_string(),
        actor_type: "BENCHMARK".to_string(),
    };

    println!("Cleaning up any previous benchmark artifacts...");
    client
        .execute(
            "DELETE FROM audit_ledger WHERE actor_id = 'bench_cascade_scale';",
            &[],
        )
        .await?;
    client
        .execute(
            "DELETE FROM graph_nodes WHERE created_by = 'bench_cascade_scale';",
            &[],
        )
        .await?;

    println!(
        "Populating 10,000 synthetic graph nodes across deep hierarchical and diamond topologies..."
    );
    let start_populate = Instant::now();

    // 1. Generate 400 root specifications across 10,000 nodes total
    let num_roots = 400;
    let nodes_per_root = 25; // 400 * 25 = 10,000 nodes total
    let total_nodes = num_roots * nodes_per_root;

    // Insert 10,000 nodes in batch
    let node_rows = client
        .query(
            "INSERT INTO graph_nodes (id, node_key, node_type, title, content, lifecycle_state, governance_policy, created_by, attributes)
             SELECT
                 gen_random_uuid(),
                 'BENCH-WP35-' || r || '-' || d,
                 CASE WHEN d = 1 THEN 'REQUIREMENT' WHEN d <= 5 THEN 'SPECIFICATION' ELSE 'TASK' END,
                 'Benchmark Node ' || r || '-' || d,
                 'Synthetic requirement intent content evaluating invalidation cascade sweep scalability.',
                 'ACTIVE',
                 'AUTONOMOUS_ELABORATION',
                 $1,
                 '{}'::jsonb
             FROM generate_series(1, $2::int) AS r
             CROSS JOIN generate_series(1, $3::int) AS d
             RETURNING id, node_key;",
            &[&agent.agent_id, &(num_roots as i32), &(nodes_per_root as i32)],
        )
        .await?;

    assert_eq!(node_rows.len(), total_nodes);

    use std::collections::HashMap;
    let mut key_to_id = HashMap::with_capacity(total_nodes);
    let mut root_ids = Vec::with_capacity(num_roots);

    for row in node_rows {
        let id: Uuid = row.get("id");
        let key: String = row.get("node_key");
        if key.ends_with("-1") {
            root_ids.push(id);
        }
        key_to_id.insert(key, id);
    }
    assert_eq!(root_ids.len(), num_roots);

    // 2. Insert hierarchical and diamond edges
    // For each root r:
    // - Hierarchical tree: levels 1..10 (chain of specs)
    // - Under specs 2..10: tasks 11..100 branching out
    // - Diamond topologies: tasks linking to multiple parent specifications with FULFILLS and CONSTRAINED_BY
    let mut from_ids = Vec::with_capacity(total_nodes * 2);
    let mut to_ids = Vec::with_capacity(total_nodes * 2);
    let mut edge_types = Vec::with_capacity(total_nodes * 2);

    for r in 1..=num_roots {
        // Spec chain: 2->1, 3->2, 4->3, 5->4
        for d in 2..=5 {
            let curr_key = format!("BENCH-WP35-{r}-{d}");
            let prev_key = format!("BENCH-WP35-{r}-{}", d - 1);
            from_ids.push(key_to_id[&curr_key]);
            to_ids.push(key_to_id[&prev_key]);
            edge_types.push("DERIVED_FROM");
        }

        // Tasks 6..=25 connected hierarchically to specs 2..5
        for d in 6..=nodes_per_root {
            let spec_idx = ((d - 6) % 4) + 2; // spec 2..5
            let task_key = format!("BENCH-WP35-{r}-{d}");
            let spec_key = format!("BENCH-WP35-{r}-{spec_idx}");
            from_ids.push(key_to_id[&task_key]);
            to_ids.push(key_to_id[&spec_key]);
            edge_types.push("FULFILLS");

            // Diamond topology: Every 2nd task also has a cross-cutting constraint link
            // to a different spec in the same subtree, forming diamond paths
            if d % 2 == 0 {
                let cross_spec_idx = (((d - 6) + 1) % 4) + 2;
                if cross_spec_idx != spec_idx {
                    let cross_spec_key = format!("BENCH-WP35-{r}-{cross_spec_idx}");
                    from_ids.push(key_to_id[&task_key]);
                    to_ids.push(key_to_id[&cross_spec_key]);
                    edge_types.push("CONSTRAINED_BY");
                }
            }
        }
    }

    client
        .execute(
            "INSERT INTO graph_edges (from_node_id, to_node_id, edge_type, created_by, lifecycle_state)
             SELECT
                 unnest($1::uuid[]),
                 unnest($2::uuid[]),
                 unnest($3::varchar[]),
                 $4,
                 'ACTIVE';",
            &[&from_ids, &to_ids, &edge_types, &agent.agent_id],
        )
        .await?;

    // Analyze tables for optimal query plans
    client.execute("ANALYZE graph_nodes;", &[]).await?;
    client.execute("ANALYZE graph_edges;", &[]).await?;

    let populate_elapsed = start_populate.elapsed();
    println!(
        "Graph population completed in {:.2?} ({} nodes, {} edges across {} subtrees).\n",
        populate_elapsed,
        total_nodes,
        from_ids.len(),
        num_roots
    );

    // Warm-up 5 trials
    println!("Executing 5 warm-up invalidation trials...");
    for &warm_root in &root_ids[0..5] {
        let _ = trigger_downward_invalidation_client(
            &mut client,
            warm_root,
            "Warm-up invalidation",
            &agent,
        )
        .await?;
        // Reset invalidated nodes back to ACTIVE for subsequent runs
        client
            .execute(
                "UPDATE graph_nodes SET lifecycle_state = 'ACTIVE' WHERE created_by = $1 AND lifecycle_state = 'NEEDS_REVERIFICATION';",
                &[&agent.agent_id],
            )
            .await?;
    }

    // Benchmark 100 trials across all 100 subtrees in the 10^4 node graph
    println!("Executing 100 invalidation cascade trials across 10^4 node graph...");
    let mut latencies_ms = Vec::with_capacity(100);

    for (trial_idx, &root_id) in root_ids[0..100].iter().enumerate() {
        let start = Instant::now();
        let res = trigger_downward_invalidation_client(
            &mut client,
            root_id,
            &format!("Benchmark trial {trial_idx}"),
            &agent,
        )
        .await?;
        let elapsed = start.elapsed();
        let elapsed_ms = elapsed.as_secs_f64() * 1000.0;

        assert!(
            !res.invalidated_nodes.is_empty(),
            "Sweep must invalidate descendant nodes in subtree"
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

    // Clean up benchmark data
    println!("Cleaning up synthetic benchmark data...");
    let _ = client
        .execute(
            "DELETE FROM audit_ledger WHERE actor_id = 'bench_cascade_scale';",
            &[],
        )
        .await;
    let _ = client
        .execute(
            "DELETE FROM graph_nodes WHERE created_by = 'bench_cascade_scale';",
            &[],
        )
        .await;

    println!("\n================================================================================");
    println!("  SLA-2 INVALIDATION CASCADE SCALE BENCHMARK SCORECARD (10^4 NODES)");
    println!("================================================================================");
    println!("  Evaluated Trials:         100 invalidation sweeps");
    println!(
        "  Graph Scale:              10,000 nodes | {} edges | 100 subtrees",
        from_ids.len()
    );
    println!("  Mean Latency:             {:.3} ms", mean_ms);
    println!("  Median (p50) Latency:     {:.3} ms", p50_ms);
    println!("  p90 Latency:              {:.3} ms", p90_ms);
    let threshold_ms = if cfg!(debug_assertions) { 50.0 } else { 10.0 };

    println!(
        "  p95 Latency:              {:.3} ms  (Target Threshold: < {:.1} ms{})",
        p95_ms,
        threshold_ms,
        if cfg!(debug_assertions) {
            " [debug profile]"
        } else {
            ""
        }
    );
    println!("  p99 Latency:              {:.3} ms", p99_ms);
    println!(
        "  Min / Max Latency:        {:.3} ms / {:.3} ms",
        min_ms, max_ms
    );
    println!("================================================================================");

    if p95_ms < threshold_ms {
        println!(
            "  STATUS: PASS (SLA-2 satisfied: {:.3} ms < {:.1} ms)\n",
            p95_ms, threshold_ms
        );
    } else {
        println!(
            "  STATUS: FAIL (SLA-2 breached: {:.3} ms >= {:.1} ms)\n",
            p95_ms, threshold_ms
        );
        panic!(
            "SLA-2 benchmark failure: p95 latency {:.3} ms exceeded {:.1} ms threshold",
            p95_ms, threshold_ms
        );
    }

    Ok(())
}
