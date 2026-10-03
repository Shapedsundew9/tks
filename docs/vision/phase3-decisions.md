# Phase 3 Implementation Decision Record: Automated Invalidation, Multi-Agent Workspaces & Real-Time Explorer

## 1. Decision Ledger

| Decision ID | Work Package | Title | Category | Target Upstream Document | Status |
| :--- | :--- | :--- | :--- | :--- | :--- |
| DEC-3.1 | WP-3.1 | Single-Roundtrip Multi-Statement Recursive CTE for Invalidation, Batch Update, Audit Append, and pg_notify | Technical Trade-off | architecture.md §5.1, §5.2 | Implemented |
| DEC-3.2 | WP-3.1 | Shortest-Path Staleness Propagation in Diamond and Multipath Dependency Subgraphs | Specification Gap | architecture.md §5.1 | Implemented |
| DEC-3.3 | WP-3.1 | Dedicated Asynchronous PostgreSQL LISTEN/NOTIFY Driver and Tokio Broadcast Event Bus | Technical Trade-off | architecture.md §6 | Implemented |

---

## 2. Decision Entries

### DEC-3.1: Single-Roundtrip Multi-Statement Recursive CTE for Invalidation, Batch Update, Audit Append, and pg_notify

* **Work Package:** WP-3.1
* **Category:** Technical Trade-off
* **Context & Problem:** Work Package WP-3.1 requires recursive downward cascade invalidation traversing active dependency edges (`FULFILLS`, `CONSTRAINED_BY`, `DERIVED_FROM`), updating matching descendant nodes to `NEEDS_REVERIFICATION`, inserting audit rows into `audit_ledger` with monotonic `event_seq` and a shared `batch_id`, and publishing notification events onto `tks_graph_events`. Executing this across separate application-level roundtrips (traversal query -> Rust memory manipulation -> batch update query -> bulk audit insert query -> notification loop) creates high network overhead, risks transaction timeout, and fails the $< 10\text{ ms}$ SLA on large subtrees (1,000+ nodes).
* **Options Considered:**
  * *Option A:* Multi-roundtrip execution. Retrieve descendant node IDs via recursive SELECT CTE, calculate staleness scores in application memory, execute an UPDATE query, perform batch INSERT into `audit_ledger`, and dispatch events via Tokio broadcast channels or `pg_notify` loop. Pros: Easier to inspect intermediate Rust structures. Cons: High latency overhead from multiple network roundtrips between application and PostgreSQL; fails the 10ms SLA on 1,000-node trees; risks partial failure states and inconsistent transaction read windows.
  * *Option B:* Single-roundtrip multi-statement CTE in PostgreSQL (`DOWNWARD_INVALIDATION_SQL`). Chained CTEs: `descendant_traversal` (recursive CTE traversing downward edges with cycle detection via array `path`), `deduped_descendants` (shortest-path depth aggregation via `MIN(depth)`), `updated_nodes` (atomic batch UPDATE on `graph_nodes` transitioning `lifecycle_state` to `NEEDS_REVERIFICATION` and merging staleness metadata in `attributes`), and `inserted_audits` (bulk INSERT into `audit_ledger` with `nextval('audit_ledger_event_seq_seq')`, recording `batch_id`, and executing `pg_notify('tks_graph_events', ...)` within the transaction). Pros: Completes traversal, updates, audit logging, and notification dispatch in a single database roundtrip; maintains strict transaction atomicity under `pg_advisory_xact_lock`; consistently achieves $< 10\text{ ms}$ execution on 1,000-node hierarchies. Cons: Complex SQL query structure requiring strict column type and alias alignment.
* **Decision Taken & Rationale:** Adopted Option B. Single-roundtrip execution guarantees atomic state transitions and audit logging while meeting SLA-2 latency criteria. In `src/storage/cascade.rs`, `DOWNWARD_INVALIDATION_SQL` combines traversal, batch update, audit ledger append, and `pg_notify` dispatch into a single database roundtrip.
* **Upstream Impact & Target Document:** `docs/vision/architecture.md` §5.1 and §5.2 should document the multi-statement CTE pattern for downward invalidation cascades.
* **Status:** Implemented

### DEC-3.2: Shortest-Path Staleness Propagation in Diamond and Multipath Dependency Subgraphs

* **Work Package:** WP-3.1
* **Category:** Specification Gap
* **Context & Problem:** In diamond dependencies or complex graph topologies, a descendant task or specification may have multiple downward paths from the invalidated root requirement with different topological path lengths (depths). WP-3.1 specifies `staleness_score = 1.0 / (depth as f64)`. When multiple paths reach the same descendant node, calculating staleness based on an arbitrary path or longest path would underestimate staleness or lead to non-deterministic scoring depending on PostgreSQL traversal order.
* **Options Considered:**
  * *Option A:* Traverse paths arbitrarily or assign depth based on first encounter in recursion. Pros: Trivial to implement without post-traversal aggregation. Cons: Non-deterministic results if graph traversal order shifts; does not guarantee maximum staleness impact; risks duplicate node updates if a node appears multiple times in recursion.
  * *Option B:* Shortest-path depth aggregation (`MIN(depth)`). Since staleness score is inversely proportional to depth (`1.0 / depth`), the shortest path represents the most direct causal impact of the root modification and maximizes the staleness score (`1.0 / min_depth`). Cons: Requires an aggregation step `GROUP BY node_id, node_key, title` after the recursive path traversal CTE.
* **Decision Taken & Rationale:** Adopted Option B. Using `MIN(depth)` in `deduped_descendants` ensures deterministic staleness scoring based on the shortest causal path to the invalidated root requirement, while also deduplicating descendant nodes so each node is updated and audited exactly once per cascade batch.
* **Upstream Impact & Target Document:** `docs/vision/architecture.md` §5.1 Invalidation Cascade Engine should define the shortest-path deterministic depth calculation for multipath DAG subtrees.
* **Status:** Implemented

### DEC-3.3: Dedicated Asynchronous PostgreSQL LISTEN/NOTIFY Driver and Tokio Broadcast Event Bus

* **Work Package:** WP-3.1
* **Category:** Technical Trade-off
* **Context & Problem:** WP-3.1 requires `GraphEventBus` managing internal Tokio broadcast channels connected to PostgreSQL `LISTEN tks_graph_events` via a dedicated connection listener. In `tokio_postgres`, issuing `LISTEN tks_graph_events` and consuming asynchronous notifications requires a dedicated connection whose background driver (`Connection::poll_message` or `Connection::poll`) must be continuously polled. A simple sequential await of `client.execute("LISTEN ...")` before spawning the message driver loop results in an immediate deadlock because `client.execute` waits for a server response that can only be processed by the driver itself. Furthermore, deserialization or payload malformation must not crash the event listener.
* **Options Considered:**
  * *Option A:* Polling `audit_ledger` table with periodic interval queries (`SELECT ... WHERE event_seq > last_seq`). Pros: Simple connection lifecycle. Cons: High database polling load, increased latency (violating the $< 5\text{ ms}$ broadcast delivery SLA), and conflicts with the WP-3.1 requirement for `LISTEN`/`NOTIFY`.
  * *Option B:* Dedicated asynchronous background listener combining connection driving and notification dispatch via Tokio tasks. To avoid the `tokio_postgres` handshake/execution deadlock, the listener drives connection polling while executing `LISTEN tks_graph_events`, then continuously yields server notifications into a 1,024-capacity Tokio broadcast channel with JSON payload deserialization. Pros: Sub-millisecond notification latency ($< 5\text{ ms}$ SLA); non-blocking broadcast dispatch; zero polling query overhead; resilient error handling. Cons: Requires dedicated connection management and cooperative cancellation token handling.
* **Decision Taken & Rationale:** Adopted Option B. In `src/storage/event_bus.rs`, `run_pg_listener` and `start_pg_listener` implement a dedicated connection listener with cooperative Tokio task driving and broadcast forwarding.
* **Upstream Impact & Target Document:** `docs/vision/architecture.md` §6 Gateway & Event Streaming should document the dedicated `LISTEN`/`NOTIFY` Tokio broadcast integration.
* **Status:** Implemented
