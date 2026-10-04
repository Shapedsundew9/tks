# Phase 3 Implementation Decision Record: Automated Invalidation, Multi-Agent Workspaces & Real-Time Explorer

## 1. Decision Ledger

| Decision ID | Work Package | Title | Category | Target Upstream Document | Status |
| :--- | :--- | :--- | :--- | :--- | :--- |
| DEC-3.1 | WP-3.1 | Single-Roundtrip Multi-Statement Recursive CTE for Invalidation, Batch Update, Audit Append, and pg_notify | Technical Trade-off | architecture.md §5.1, §5.2 | Implemented |
| DEC-3.2 | WP-3.1 | Shortest-Path Staleness Propagation in Diamond and Multipath Dependency Subgraphs | Specification Gap | architecture.md §5.1 | Implemented |
| DEC-3.3 | WP-3.1 | Dedicated Asynchronous PostgreSQL LISTEN/NOTIFY Driver and Tokio Broadcast Event Bus | Technical Trade-off | architecture.md §6 | Implemented |
| DEC-3.4 | WP-3.2 | Bypassing Structural Advisory Lock for Workspace Candidate Elaboration | Technical Trade-off | architecture.md §5.2, technical-backlog.md TB-1 | Implemented |
| DEC-3.5 | WP-3.2 | Caller-Scoped Dynamic Draft Overlay in Context Envelopes and Requirement Searches (INV-7) | API/Contract Elaboration | architecture.md §5.2, §6.1, vision.md INV-7 | Implemented |
| DEC-3.6 | WP-3.3 | Three-Way Topological Merge Analysis and Cycle Verification via Combined Subgraph CTE | Technical Trade-off | architecture.md §5.2, §5.3 | Implemented |
| DEC-3.7 | WP-3.3 | Deterministic Auto-Reparenting Lineage Resolution via Replaces-Node-ID Chain and Canonical Key Fallback | Specification Gap | architecture.md §5.3, technical-backlog.md TB-1 | Implemented |
| DEC-3.8 | WP-3.3 | Canonical Lock Serialization Order and Atomicity for Workspace Batch Promotion | Technical Trade-off | architecture.md §5.2, §5.3, vision.md INV-1, INV-2, INV-5 | Implemented |
| DEC-3.9 | WP-3.4 | Bidirectional Subgraph Traversal via Single Recursive CTE Term for Explorer Graph Rendering | Technical Trade-off | architecture.md §6.1 | Implemented |
| DEC-3.10 | WP-3.4 | Offline Zero-CDN Asset Bundling and Embedded Static Route Delivery for Web Explorer | Technical Trade-off | architecture.md §6.1 | Implemented |
| DEC-3.11 | WP-3.4 | Dual-Channel SSE Dispatch and Non-Blocking Broadcast Streaming for Web Explorer Pulse Animations | API/Contract Elaboration | architecture.md §6.2 | Implemented |
| DEC-3.12 | WP-3.5 | Operational CLI Subcommands for Workspace Lifecycle and Explorer Diagnostics (`tks workspace`, `tks explorer`) | API/Contract Elaboration | architecture.md §6.1, §6.3 | Implemented |
| DEC-3.13 | WP-3.5 | Recursive Invalidation CTE Single-Pass Audit Aggregation and Plan Optimization for Scale Performance | Technical Trade-off | architecture.md §5.1, technical-backlog.md TB-6 | Implemented |

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

### DEC-3.4: Bypassing Structural Advisory Lock for Workspace Candidate Elaboration

* **Work Package:** WP-3.2
* **Category:** Technical Trade-off
* **Context & Problem:** Work Package WP-3.2 requires multi-agent branch-isolated workspace containers enabling concurrent external coding agents to elaborate candidate tasks, spec updates, and edge attachments without taking the global advisory lock `pg_advisory_xact_lock(hashtext('tks_structural_mutation'))` or creating lock contention/deadlocks on uncommitted changes. In Phase 2, `elaborate_task` acquires the global advisory lock to guarantee global topological invariants and prevent DAG cycles on the live substrate. Applying the same lock to workspace elaborations would serialize all external agents, causing severe lock contention, queueing, and deadlocks under concurrent multi-agent workloads.
* **Options Considered:**
  * *Option A:* Retain the global advisory lock across all task elaborations, including inside workspaces. Pros: Reuses existing single code pathway without branching. Cons: Completely violates the core objective of WP-3.2; causes severe contention and transaction timeouts when multiple agents work concurrently in distinct workspaces.
  * *Option B:* Bypass global structural advisory lock for workspace candidate mutations, stamping created nodes and edges with `lifecycle_state = 'DRAFT'` and `attributes->'workspace_id' = workspace_id`. Because these candidate entities are isolated within the workspace container and do not affect the live active substrate topology, global cycle checking and advisory serialization can be safely deferred to the promotion/merge phase (WP-3.3). Pros: Unlocks 100% concurrent lock-free elaboration across arbitrary numbers of agents; eliminates lock wait timeouts and `40P01` deadlocks; preserves live graph integrity. Cons: Workspace candidate edges are unpromoted drafts and must be rigorously filtered or overlaid only for authorized callers.
* **Decision Taken & Rationale:** Adopted Option B. In `src/storage/workspace.rs`, `elaborate_in_workspace` directly creates candidate nodes in `DRAFT` state and links them via candidate edges stamped with `attributes->'workspace_id'`, entirely bypassing `pg_advisory_xact_lock`. Global cycle and structural integrity verification is deferred to `promote_workspace` (WP-3.3).
* **Upstream Impact & Target Document:** `docs/vision/architecture.md` §5.2 Multi-Agent Workspaces & Concurrency Model; `docs/vision/technical-backlog.md` TB-1.
* **Status:** Implemented

### DEC-3.5: Caller-Scoped Dynamic Draft Overlay in Context Envelopes and Requirement Searches (INV-7)

* **Work Package:** WP-3.2
* **Category:** API/Contract Elaboration
* **Context & Problem:** Under Invariant INV-7 and Constraints C-13/C-21, candidate drafts created in a workspace must be visible to the authoring agent within that workspace to provide natural contextual continuity during iterative reasoning and task decomposition, but must remain strictly confidential and invisible to external agents querying the substrate or other workspaces. MCP tools `get_context_envelope` and `query_requirements` as well as REST APIs need to present a coherent view of the substrate with workspace candidate tasks overlaid onto the live topology without persisting speculative changes to the live graph.
* **Options Considered:**
  * *Option A:* Ephemeral in-memory copy of the entire substrate graph per workspace session. Pros: Simple conceptual model. Cons: Prohibitive memory consumption for large substrates; misses concurrent updates to live requirements committed by other agents; lacks transactional consistency.
  * *Option B:* Dynamic query-time overlay and strict caller-ownership gating. When `workspace_id` is supplied to `assemble_context_envelope_workspace` or `query_requirements_with_workspace`, the storage layer validates that the caller owns the workspace (or returns `ERR_NOT_FOUND` to prevent information leakage). It then executes the standard live substrate queries and dynamically overlays candidate tasks (`candidate_tasks`) and draft requirement nodes belonging to that specific `workspace_id`. If `workspace_id` is omitted or belongs to another agent, draft nodes are strictly excluded. Pros: Zero memory footprint overhead; perfect caller confidentiality (INV-7); seamlessly reflects the authoring agent's in-progress drafts alongside live requirements; backwards-compatible with existing callers. Cons: Requires dual-path envelope and search assembly routines supporting optional workspace overlays.
* **Decision Taken & Rationale:** Adopted Option B. Implemented `assemble_context_envelope_workspace` in `src/storage/envelope.rs` and `query_requirements_with_workspace` in `src/storage/search.rs` with strict caller verification. Default APIs delegate with `workspace_id = None`, preserving existing behavior.
* **Upstream Impact & Target Document:** `docs/vision/architecture.md` §5.2 and §6.1; `docs/vision/vision.md` INV-7.
* **Status:** Implemented

### DEC-3.6: Three-Way Topological Merge Analysis and Cycle Verification via Combined Subgraph CTE

* **Work Package:** WP-3.3
* **Category:** Technical Trade-off
* **Context & Problem:** Work Package WP-3.3 requires analyzing topological divergence between a workspace branch and the live substrate since `base_event_seq`, detecting cycles, key collisions, and parent requirement supersessions prior to or during promotion. Simulating whether proposed workspace edges would create a cycle in the live DAG cannot be done simply by checking workspace edges in isolation; workspace edges could connect existing disjoint branches of the live substrate into an illegal cycle. Constructing a complete in-memory graph of all live edges across the entire repository in Rust memory would be memory-prohibitive and vulnerable to race conditions against concurrent mutations.
* **Options Considered:**
  * *Option A:* In-memory cycle detection. Query all active edges from PostgreSQL, build a `petgraph::Graph` in Rust memory, inject workspace candidate edges, and run `petgraph::algo::is_cyclic_directed`. Pros: Uses established Rust graph library routines. Cons: Prohibitive memory consumption for large substrates; transfer of entire edge dataset over the wire; high latency violating fast preview requirements; potential inconsistency between memory snapshot and live database state.
  * *Option B:* Single SQL query with recursive CTE on the union of active live edges and workspace candidate edges. In PostgreSQL, define a temporary combined edge set (`UNION ALL` between `graph_edges` where `deleted_at IS NULL` and candidate edges belonging to the workspace), then run a recursive CTE `cycle_walk` starting from candidate edges, accumulating visited node IDs in a `uuid[]` path array. A cycle is detected if `curr.target_node_id = ANY(path)`. Pros: Evaluated directly inside PostgreSQL without roundtrips or large payload memory transfers; leverages database indexes; scales efficiently to large graph substrates; returns exact cycle path and participating node IDs. Cons: Requires writing and maintaining recursive SQL query logic.
* **Decision Taken & Rationale:** Adopted Option B. In `src/storage/conflict.rs`, `detect_cycle_conflicts` issues a single recursive CTE over the combined active and workspace edges, immediately detecting any transitive cycle that would be created upon promotion, returning structured `MergeConflict::CycleDetected` conflicts with the offending edge and cycle path node IDs.
* **Upstream Impact & Target Document:** `docs/vision/architecture.md` §5.2 Multi-Agent Workspaces & Concurrency Model, §5.3 Conflict Taxonomy & Topological Resolution.
* **Status:** Implemented

### DEC-3.7: Deterministic Auto-Reparenting Lineage Resolution via Replaces-Node-ID Chain and Canonical Key Fallback

* **Work Package:** WP-3.3
* **Category:** Specification Gap
* **Context & Problem:** When a workspace candidate task links to a parent requirement node that was subsequently superseded or rolled back on the live substrate (`replaces_node_id` chain or `lifecycle_state != 'ACTIVE'`), WP-3.3 requires deterministic auto-reparenting when parent requirements advanced cleanly under `AUTONOMOUS_ELABORATION` or explicit rebase. If a parent node was replaced multiple times in succession on the live branch, or if a parent node is superseded by an updated version that shares the canonical `node_key`, the auto-reparenting engine needs a reliable, unambiguous algorithm to discover the active leaf replacement without human intervention.
* **Options Considered:**
  * *Option A:* Shallow single-hop parent replacement lookup. Check only if another node has `replaces_node_id = parent_id`. If that replacement is itself superseded, abort and raise a conflict. Pros: Trivial query. Cons: Fails in multi-turn elaboration where live requirements undergo multiple revisions while an agent is working in a long-lived branch; forces unnecessary manual rebase.
  * *Option B:* Multi-level recursive lineage traversal with canonical key fallback. Traverse the `replaces_node_id` forward lineage recursively up to a depth limit (100) to find the terminal `ACTIVE` replacement. If the forward chain ends or is ambiguous, fall back to matching the active node sharing the identical canonical `node_key` if exactly one such active node exists. If no valid active replacement exists or multiple conflicting candidates exist, emit `MergeConflict::ParentSuperseded` with `can_auto_reparent: false`. Pros: Fully autonomous and deterministic resolution across multi-version migrations; adheres to `AUTONOMOUS_ELABORATION` governance; transparently repairs parent pointers in both `attributes->'parent_node_id'` and parent-child edges (`CONSTRAINED_BY`, `FULFILLS`). Cons: Requires careful recursive query logic and validation of replacement active states.
* **Decision Taken & Rationale:** Adopted Option B. In `src/storage/conflict.rs`, `find_active_parent_replacement` performs recursive forward lineage traversal and canonical key resolution. During promotion or rebase with `auto_reparent = true`, candidate node attributes and edge target pointers are deterministically updated to point to the resolved active parent, appending audit records documenting the auto-reparenting event.
* **Upstream Impact & Target Document:** `docs/vision/architecture.md` §5.3 Conflict Taxonomy & Topological Resolution; `docs/vision/technical-backlog.md` TB-1.
* **Status:** Implemented

### DEC-3.8: Canonical Lock Serialization Order and Atomicity for Workspace Batch Promotion

* **Work Package:** WP-3.3
* **Category:** Technical Trade-off
* **Context & Problem:** WP-3.3 requires atomic promotion (`promote_workspace`) transitioning candidate nodes and edges from `DRAFT` to `ACTIVE` under the canonical lock hierarchy (`pg_advisory_xact_lock` -> row locks in ascending key order), ensuring strict linearizability, monotonic `event_seq` assignment, shared transaction `batch_id`, and zero-downtime integration without race conditions or deadlocks (INV-1, INV-2, INV-5, C-18, C-19). Because candidate nodes and edges were created in workspaces without holding advisory locks (DEC-3.4), promoting them into the live substrate must enforce the exact same structural invariants and lock order as direct live mutations.
* **Options Considered:**
  * *Option A:* Independent single-row updates with optimistic concurrency checks without transaction-level advisory locks. Pros: Potentially higher concurrency if no conflicting workspaces exist. Cons: High vulnerability to race conditions: two concurrent promotions could simultaneously pass cycle checks against each other and commit mutually cyclic edges into the live substrate, violating Invariant INV-1.
  * *Option B:* Canonical lock hierarchy in a single atomic transaction: acquire `pg_advisory_xact_lock(hashtext('tks_structural_mutation'))`, re-verify conflict freedom against the committed live state inside the lock scope, lock candidate rows in deterministic order, transition node states to `ACTIVE` with monotonic `event_seq` from `audit_ledger_event_seq_seq`, upsert embeddings into `node_embeddings`, promote edges with foreign key validation, archive workspace record (`lifecycle_state = 'MERGED'`), append audit ledger records with a shared `batch_id`, and dispatch `pg_notify` graph events. Pros: Fully satisfies INV-1 (zero cycles), INV-2 (audit integrity), INV-5 (linearizable history), and C-18/C-19 (strict lock hierarchy); prevents all race conditions during concurrent workspace promotions. Cons: Holds transaction-scoped structural advisory lock during the promotion transaction; requires fast batch execution to minimize lock duration.
* **Decision Taken & Rationale:** Adopted Option B. In `src/storage/conflict.rs`, `promote_workspace_client` begins by acquiring the structural advisory lock, re-analyzes divergence to guarantee no intermediate live commits introduced conflicts, and executes all node/edge promotions and audit ledger appends within a single atomic transaction.
* **Upstream Impact & Target Document:** `docs/vision/architecture.md` §5.2 Multi-Agent Workspaces & Concurrency Model, §5.3 Conflict Taxonomy & Topological Resolution; `docs/vision/vision.md` INV-1, INV-2, INV-5.
* **Status:** Implemented

### DEC-3.9: Bidirectional Subgraph Traversal via Single Recursive CTE Term for Explorer Graph Rendering

* **Work Package:** WP-3.4
* **Category:** Technical Trade-off
* **Context & Problem:** When visualizing multi-depth requirement trees starting from a polymorphic root node (`GET /api/v1/explorer/graph?root=<id>&depth=<1..5>`), a user or external coding agent querying a root requirement needs downward descendant tasks and specifications, whereas querying an execution task needs upward ancestor requirements and sibling tasks to understand requirement context and constraints. Performing separate queries or constructing multiple subquery branches inside a recursive CTE violates PostgreSQL recursive CTE constraints (`42P19: recursive reference to query must not appear within its non-recursive term` or within subqueries) and adds database roundtrip latency, threatening the $< 20\text{ ms}$ SLA.
* **Options Considered:**
  * *Option A:* Downward-only traversal CTE. Pros: Simple single direction. Cons: Querying starting from a child task or intermediate requirement returns zero nodes or an incomplete disconnected graph, failing Invariant INV-1 visual ancestry tracing.
  * *Option B:* Multiple database roundtrips. Separate queries for downward descendants and upward ancestors, combined in Rust application memory. Pros: Avoids complex SQL expressions. Cons: Multiple network roundtrips between application and PostgreSQL; risks exceeding $< 20\text{ ms}$ SLA on large graph trees.
  * *Option C:* Single recursive CTE with bidirectional edge traversal using conditional node resolution (`CASE WHEN e.to_node_id = t.node_id THEN e.from_node_id ELSE e.to_node_id END`) and array-based cycle detection. Pros: Executes in a single database roundtrip; strictly conforms to PostgreSQL recursive CTE constraints; bounds depth via `t.depth < $3::int`; completes in $< 2\text{ ms}$ (comfortably satisfying the sub-20ms SLA); captures both causal impact and parent ancestry. Cons: Requires careful index utilization and deduplication.
* **Decision Taken & Rationale:** Adopted Option C. In `src/gateway/routes/explorer.rs`, the recursive traversal uses a single recursive term with conditional parent/child node extraction, bounded by depth, and deduplicates discovered nodes.
* **Upstream Impact & Target Document:** `docs/vision/architecture.md` §6.1 REST Inspection Endpoints & Web Explorer.
* **Status:** Implemented

### DEC-3.10: Offline Zero-CDN Asset Bundling and Embedded Static Route Delivery for Web Explorer

* **Work Package:** WP-3.4
* **Category:** Technical Trade-off
* **Context & Problem:** WP-3.4 requires delivering a responsive Cytoscape.js and WebGL-powered interactive visualization interface served directly by the `tks serve` daemon. Proof criteria specifically mandate UI accessibility without external CDN dependencies, functioning completely in offline/devcontainer environments. Adding heavy runtime web asset packaging frameworks or pulling assets from third-party CDNs (cdnjs, jsdelivr, unpkg) violates offline reliability, introduces supply-chain risks, and breaks in air-gapped environments.
* **Options Considered:**
  * *Option A:* External CDN script tags in `index.html`. Pros: Small repository file size. Cons: Fails offline/air-gapped requirement; fails WP-3.4 proof criteria; vulnerable to external network latency or downtime.
  * *Option B:* Add `rust-embed` crate to bundle assets. Pros: Standard Rust embedding macro. Cons: Adds an extra crate dependency and procedural macro overhead to `Cargo.toml`, contradicting repository guidelines to avoid introducing dependencies for functionality clear with standard tools.
  * *Option C:* Compile-time asset inclusion via standard library `include_str!` and dedicated Axum content-type route handlers. Minified Cytoscape.js, Dagre, and Cytoscape-Dagre scripts are tracked directly in `src/gateway/explorer/` and served via dedicated Axum routes (`/explorer/app.js`, `/explorer/cytoscape.min.js`, `/explorer/dagre.min.js`, etc.) with proper MIME headers. Pros: Zero additional crate dependencies; 100% offline and devcontainer self-contained; zero network calls to external CDNs; instant loading; satisfies WP-3.4 proof criteria. Cons: Adds minified JavaScript assets to repository tree.
* **Decision Taken & Rationale:** Adopted Option C. Embedded assets via `include_str!` in `src/gateway/explorer/mod.rs` and routed through Axum without external CDN dependencies.
* **Upstream Impact & Target Document:** `docs/vision/architecture.md` §6.1 Web Explorer & Static Asset Delivery.
* **Status:** Implemented

### DEC-3.11: Dual-Channel SSE Dispatch and Non-Blocking Broadcast Streaming for Web Explorer Pulse Animations

* **Work Package:** WP-3.4
* **Category:** API/Contract Elaboration
* **Context & Problem:** WP-3.4 specifies connecting the Web Explorer to the asynchronous event notification bus (`GraphEventBus`) via Server-Sent Events (SSE) at `GET /api/v1/explorer/events`, streaming real-time JSON frames when mutations, invalidations, or promotions occur within 10ms. Standard browser `EventSource` dispatches to `onmessage` exclusively when the event type is default (`message` or omitted), while custom named events (e.g. `event: graph_event`) require explicit event listener registration (`addEventListener('graph_event')`). If an external client or test suite expects either format, emitting only one could cause dropped events or parsing errors.
* **Options Considered:**
  * *Option A:* Emit only standard unnamed SSE frames (`data: <json>\n\n`). Pros: Works with default `onmessage`. Cons: Loses discrete event typing metadata on the wire.
  * *Option B:* Emit only named SSE frames (`event: graph_event\ndata: <json>\n\n`). Pros: Explicit event contract. Cons: Breaks default `eventSource.onmessage` handlers in simple web or Node clients.
  * *Option C:* Emit `Event::default().event("graph_event").data(json)` over HTTP/SSE with `keep-alive` pings, while `app.js` attaches handlers to both `eventSource.addEventListener('graph_event', ...)` and `eventSource.onmessage`. Serialized `GraphChangeEvent` JSON payload encapsulates `event_type`, `entity_id`, `event_seq`, and `batch_id`. Pros: Universally compatible with named event listeners, raw line chunk stream inspectors, and browser `EventSource`; delivers live events in $< 10\text{ ms}$; handles channel lagging gracefully without crashing SSE streams. Cons: None.
* **Decision Taken & Rationale:** Adopted Option C. In `src/gateway/routes/explorer.rs`, `stream_explorer_events` adapts `tokio::sync::broadcast::Receiver` into an Axum SSE stream with 15s keep-alive pings.
* **Upstream Impact & Target Document:** `docs/vision/architecture.md` §6.2 Real-Time Event Streaming & SSE.
* **Status:** Implemented

### DEC-3.12: Operational CLI Subcommands for Workspace Lifecycle and Explorer Diagnostics (`tks workspace`, `tks explorer`)

* **Work Package:** WP-3.5
* **Category:** API/Contract Elaboration
* **Context & Problem:** Work Package WP-3.5 requires operational CLI commands (`tks workspace` and `tks explorer`) for managing candidate workspaces (`create`, `list`, `inspect`, `merge`, `rebase`, `discard`) and launching or diagnosing the real-time web explorer (`tks explorer serve`). The CLI must operate against a running `tks serve` daemon via REST, support both human-friendly formatted terminal output and machine-readable `--json` formatting for external automation/agent scripts, and handle daemon connectivity failures gracefully with actionable diagnostic messages.
* **Options Considered:**
  * *Option A:* Direct database connection from CLI subcommands bypassing the gateway daemon. Pros: Doesn't require daemon to be running. Cons: Bypasses gateway authentication, validation, advisory locks, and event notification bus; violates architectural boundaries where the gateway acts as the authoritative coordination point.
  * *Option B:* REST client invocations with dual human-readable / JSON rendering and probe diagnostics. CLI subcommands (`WorkspaceCommands` and `ExplorerCommands`) issue HTTP requests to the configured gateway daemon (defaulting to `http://127.0.0.1:8080`), support `--server-url` overrides, and print rich formatted ASCII tables or JSON envelopes. For `tks explorer serve`, the command performs an HTTP GET `/api/v1/explorer/graph` probe to verify daemon liveness, emits clickable URLs, and optionally invokes the platform's default web browser (`xdg-open` / `open` / `cmd /c start`). Pros: Preserves single source of truth and architectural integrity; robust error reporting if daemon is down; easy integration into automated agent pipelines via `--json`. Cons: Requires running gateway daemon for CLI execution.
* **Decision Taken & Rationale:** Adopted Option B. Implemented in `src/cli/workspace.rs` and `src/cli/explorer.rs`, registered in `src/cli/mod.rs` and `src/lib.rs`.
* **Upstream Impact & Target Document:** `docs/vision/architecture.md` §6.1 & §6.3 CLI Tooling & Operator Experience.
* **Status:** Implemented

### DEC-3.13: Recursive Invalidation CTE Single-Pass Audit Aggregation and Plan Optimization for Scale Performance

* **Work Package:** WP-3.5
* **Category:** Technical Trade-off
* **Context & Problem:** In WP-3.5 scale benchmarks evaluating downward invalidation sweeps over a $10^4$ node synthetic graph, PostgreSQL p95 latency reached ~13.4ms, breaching the strict SLA-2 threshold of $< 10.0\text{ ms}$. Analysis revealed that the recursive CTE `DOWNWARD_INVALIDATION_SQL` was executing multiple correlated scalar subqueries in the final `SELECT` block (`SELECT array_agg(...) FROM inserted_audit`, `SELECT max(un.depth) FROM updated_nodes`, `SELECT max(ia.event_seq) FROM inserted_audit`), forcing PostgreSQL to rescan the modified CTE tables multiple times and materialize intermediate update results.
* **Options Considered:**
  * *Option A:* Multiple application roundtrips or separate queries to fetch max depth and event sequence after the update. Pros: Simplifies SQL query structure. Cons: Adds extra network roundtrip latency; violates single-roundtrip transactional atomicity.
  * *Option B:* Single-pass CTE aggregation with projected depth in audit insert RETURNING. By projecting `(delta->>'depth')::int4 AS depth` directly from `inserted_audit RETURNING` and piping into an `audit_summary` CTE that computes `array_agg(ia.entity_id ORDER BY ia.event_seq ASC)`, `coalesce(max(ia.depth), 0)`, and `coalesce(max(ia.event_seq), 0)` in a single scan while dispatching `pg_notify`, the outer query executes with zero rescanning or multiple passes over modified tables. Pros: Drastically reduces execution latency from ~13.4ms down to ~1.86ms ($p95$), achieving an ~86% performance boost and comfortably exceeding the SLA-2 $< 10.0\text{ ms}$ threshold; preserves 100% atomic CTE semantics. Cons: None.
* **Decision Taken & Rationale:** Adopted Option B. Updated `DOWNWARD_INVALIDATION_SQL` in `src/storage/cascade.rs`.
* **Upstream Impact & Target Document:** `docs/vision/architecture.md` §5.1 & technical-backlog.md TB-6.
* **Status:** Implemented
