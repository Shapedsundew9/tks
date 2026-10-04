# Technical Implementation Backlog

This document captures vital component-level designs, library evaluation tasks, sprint-level tactical choices, and downstream implementation details deferred from architectural review.

---

## TB-1

* **ID:** TB-1
* **Title:** Bare Git Repository Direct ODB Ingestion, Document Path Tree Mapping, Dedicated Write Actor, Concurrent ODB Reads, and Volume Backup Synchronization
* **Origin:** LD-8, iteration 1; LD-5, iteration 2; LD-12, iteration 3; LD-3, iteration 4; LD-1, LD-2, iteration 5; LD-8, iteration 6; LD-12, iteration 7; LD-10, iteration 10
* **Status:** Active
* **Description/Tactical Details:**
  1. **Low-Level ODB Direct Writes & Commit-Tree Graph:** In the Git document adapter module (`src/storage/git/`), configure the `git2` crate to interact with a bare Git repository (`git init --bare`). Implement direct object-database (ODB) writes via `git_blob_create_from_buffer` for raw document streams and construct standard tree and commit objects directly within the ODB backend targeting a dedicated specifications branch (`refs/heads/specs`). This bypasses the Git working tree and index (`.git/index`), completely eliminating index lock collisions (`.git/index.lock`) during concurrent document ingestion calls.
  2. **Document Path/Slug Tree Structure (`doc_path`):** When constructing the Git tree object in the ODB for a new or updated document, the adapter places the blob at its persistent document path/slug (`doc_path`, e.g. `specs/vision.md`). Storing documents under stable paths across successive revisions guarantees that standard Git history tracking works out-of-the-box (`git log specs/vision.md`, `git diff HEAD~1 specs/vision.md`), preserves document revision history across commits, and enables the decomposition pipeline to correlate prior active AST blocks and source byte spans on re-ingestion.
  3. **Dedicated Background Git Write Actor Task (Channel-Based Isolation & Panic Recovery):** To eliminate async reactor starvation and thread-safety hazards with non-`Sync` C pointers in `git2::Repository`, encapsulate all bare Git write and commit operations within a dedicated background actor thread (`src/storage/git/actor.rs`). The Git actor exclusively owns the writable `git2::Repository` handle and receives `CommitCommand` requests over a bounded Tokio mpsc channel (`tokio::sync::mpsc::Sender<GitCommand>`). The actor executes blocking libgit2 C calls sequentially off the Tokio async worker pool and returns results via `tokio::sync::oneshot` channels. The actor encapsulates its sequential command execution loop inside an internal panic recovery handler (`std::panic::catch_unwind`), catching transient libgit2 C-binding panics and returning structured error responses over the caller's oneshot channel without dropping the underlying `Receiver<GitCommand>` or disconnecting caller `Sender` clones held across the gateway and workers. This guarantees linear serialization on `refs/heads/specs` without `.lock` collisions (`GIT_ELOCKED`), prevents Tokio worker thread starvation, eliminates Rust `Send`/`Sync` boundary conflicts, and prevents permanent `SendError` channel breaks.
  4. **Concurrent Read-Only ODB Access (`tokio::task::spawn_blocking`):** Restrict the dedicated Git actor exclusively to serializing Git commits and tree updates (`refs/heads/specs` writes). For read-only blob extraction (`get_document_span`, worker decomposition), gateway handlers and workers open thread-local read-only repository handles or direct ODB blob lookups via `tokio::task::spawn_blocking` using `git2::Odb::read`. Because Git ODB blobs are content-addressed and strictly immutable once written, concurrent reads never block on `.git/refs/...lock`, execute concurrently without queuing behind write commits, and eliminate head-of-line blocking on the Git actor (addressing LD-12, iteration 7).
  5. **Dedicated Storage Volume Binding:** In the devcontainer and container deployment manifests (`.devcontainer/docker-compose.yml`), configure a dedicated, persistent filesystem volume mount for the bare Git repository co-located with the PostgreSQL persistent data volume, ensuring Git blob storage persists across container recreation and redeployment cycles.
  6. **Point-in-Time Backup & Restore Synchronization:** Author operational shell scripts in `scripts/` to orchestrate coordinated point-in-time backups: trigger a PostgreSQL WAL checkpoint / `pg_dump` and execute a simultaneous snapshot of the bare Git repository object database (`objects/` and `refs/`), preventing referential drift between PostgreSQL blob hash columns and underlying Git blobs.
  7. **Caller-Scoped Draft Batches & Staging Promotion Lifecycle:** In `src/storage/mutation.rs` and `src/storage/staging.rs`, retire branch workspace container tables (`workspaces`) and 3-way merge CTEs in favor of native caller-scoped draft batches (`created_by = $calling_actor_id`, LD-10, D-105). Drafts are dynamically overlaid during queries when `include_drafts = true` is passed (INV-7) and promoted atomically via standard staging promotion (`POST /api/v1/staging/approve`) acquiring `pg_advisory_xact_lock(hashtext('tks_structural_mutation'))` with dynamic DAG acyclicity check and auto-reparenting lineage resolution (DEC-2.8).

---

## TB-2

* **ID:** TB-2
* **Title:** Mechanical CommonMark AST Parsing, Structural Hierarchy Edge Generation, RFC 2119 Lexical Extraction Benchmark, and Zero-Copy Byte Slicing (`pulldown-cmark`)
* **Origin:** LD-1, iteration 2; LD-2, iteration 3; LD-3, LD-9, iteration 7; LD-8, iteration 8; LD-6, iteration 10; phase0-decisions.md DEC-0.2, DEC-0.3, DEC-0.4, DEC-0.5, DEC-0.7
* **Status:** Active
* **Description/Tactical Details:**
  1. **Streaming CommonMark Parser Implementation:** In the document ingestion module (`src/ingest/parser.rs`), implement a streaming Markdown parser using `pulldown-cmark`. Traverse the CommonMark event stream to segment ingested documents along structural boundaries (headings H1–H4, tables, blockquotes, and bulleted lists).
  2. **Zero-Token Deterministic Byte Span Capture & In-Memory Content Retention:** For each structural block event, extract exact 0-based byte offsets (`byte_start`, `byte_end`) directly from the source stream offset tracking provided by `pulldown-cmark`'s `OffsetIter`. Pointers are pinned with 100% precision without LLM inference cost. `ExtractedChunk` retains in-memory UTF-8 text via `pub content: Option<String>` alongside byte offsets for zero-source classification prompting (`build_classification_prompt(chunks: &[ExtractedChunk])`) without re-reading source files (DEC-0.5).
  3. **AST Structural Hierarchy Edge Generation & Enclosing Heading Attribution:** During Stage 1 streaming AST parsing, maintain an active heading stack tracking nesting levels (H1 through H4). Every extracted child heading or sub-block mechanically records a directed upward structural edge (`DERIVED_FROM`) pointing from the child node to its immediate parent heading node (`child_id -DERIVED_FROM-> parent_id`). When Stage 2 classifies child chunks as `SPECIFICATION` or `TASK`, this mechanical edge guarantees an unbroken path to the enclosing requirement section, ensuring the transactional ancestor CTE during staging promotion succeeds without manual edge authoring (addressing LD-3, iteration 7). Populate `heading = Some(title)` for heading chunks, and inherit `heading = Some(parent_heading_title)` for non-heading structural blocks (paragraphs, lists, tables, blockquotes) under an active heading, defaulting to `None` strictly for blocks preceding any heading, providing $O(1)$ section provenance for downstream classification prompt formatting without recursive relational lookups (DEC-0.2).
  4. **Hierarchical Heading Anchor Generation with Disambiguation (`ast_anchor`):** Concurrently with heading stack tracking, generate a deterministic slugified heading path anchor for each block (e.g. `specs/vision.md#section-1/subsection-a`), stored in `graph_nodes.attributes->'ast_anchor'`. To prevent duplicate anchor collisions across identical section titles and sub-blocks within CommonMark documents (addressing LD-8, iteration 8):
     * *Heading Slug Disambiguation:* When duplicate heading titles occur within the same parent section scope (e.g., repeated "Overview", "Rationale", or "Implementation" subsections), append an occurrence counter suffix (e.g., `doc_path#section/overview-1`, `doc_path#section/overview-2`).
     * *Ordinal Block Indexing for Non-Heading Sub-Blocks:* For non-heading structural blocks (bullet lists, tables, paragraphs) extracted under an enclosing heading, append an ordinal block index suffix (e.g., `doc_path#section/heading#block-0`, `doc_path#section/heading#block-1`).
     * *Root Pre-Heading Scope Indexing:* For non-heading structural blocks preceding the first heading in a document (e.g. document-level introductory notes or abstract blocks), scope anchors directly under the document path using an ordinal root counter (e.g., `doc_path#block-0`, `doc_path#block-1`) with `parent_heading_chunk_id = None`. This guarantees 100% roundtrip text coverage and global deterministic anchor uniqueness regardless of document heading structure (DEC-0.3).
     This guarantees that every extracted AST block generates a globally unique, deterministic `ast_anchor` within the document, ensuring Tier 2 reconciliation diffing does not conflate distinct chunks or corrupt span re-anchoring across revisions.
  5. **Zero-Copy Byte Slicing & UTF-8 Safety:** Standardize span extraction in the Rust service layer on raw byte slicing against the source buffer (`&source_bytes[byte_start..byte_end]`) prior to UTF-8 validation (`std::str::from_utf8`), eliminating string slicing panics on multi-byte UTF-8 sequences (em-dashes, curly quotes, math symbols).
  6. **Rule-Based RFC 2119 & Entity Lexical Matcher:** Implement a deterministic scanner matching RFC 2119 keywords (`MUST`, `MUST NOT`, `REQUIRED`, `SHALL`, `SHALL NOT`, `SHOULD`, `SHOULD NOT`, `RECOMMENDED`, `MAY`, `OPTIONAL`) and project entity tags (`REQ-*`, `INV-*`, `DR-*`, `C-*`, `TB-*`). Structural chunks containing these patterns are tagged as candidate requirements.
  7. **Benchmarking & Token Reduction Calibration on Stable Rust Toolchain:** Benchmark parsing throughput, memory footprint, and token reduction across representative PRDs and specification documents of varying sizes (3,000 to 10,000 words). Verify that mechanical extraction handles $\ge 80$% of structural decomposition before any external LLM invocation, measuring output token savings. To adhere strictly to stable Rust (edition 2024, C-1) and dependency minimization (C-10) without nightly `#![feature(test)]` or heavy external crates (`criterion`), configure `[[bench]]` with `harness = false` in `Cargo.toml` and implement a deterministic benchmark binary in `benches/ast_decomposition_bench.rs` using `std::time::Instant` (DEC-0.4).
  8. **Dual-Mode Evaluation Harness Supporting Offline Mock Simulation and Live REST Invocations:** In the evaluation harness (`tests/extraction_eval.rs`), implement dual-mode execution testing extraction fidelity and classification accuracy against hand-labeled ground-truth datasets (`tests/fixtures/ground_truth_requirements.json`). To avoid bloating production runtime binaries prior to gateway construction (C-10), declare `reqwest` strictly under `[dev-dependencies]`. Gated by `LLM_API_KEY` and `TKS_LIVE_EVAL=1` for live REST invocations against external OpenAI-compatible endpoints, defaulting to deterministic offline mock simulation for automated CI/CD and local test suites (DEC-0.7).
  9. **Two-Tier Atomization with Mechanical Sentence Slicing (`unicode-segmentation`):** To preserve exact 0-based byte offsets (INV-4) and minimize LLM token consumption (C-11), Stage 1 streaming CommonMark AST parser mechanically slices structural prose into sentence/clause-level chunks using `unicode-segmentation`, assigns exact `(byte_start, byte_end)` coordinates, and assigns ordinal aliases (`c1`, `c2`, ...). Stage 2 LLM inference receives the pre-atomized chunks and returns compact tuples `(alias, modality, confidence)` without echoing source text, eliminating parsing hallucinations and token context blowout (LD-6, D-112).

---

## TB-3

* **ID:** TB-3
* **Title:** Schema Versioning and StagedPayload Enum Mapping for Staging Queue
* **Origin:** LD-8, iteration 2; LD-3, LD-11, iteration 3
* **Status:** Retired
* **Description/Tactical Details:**
  * **Retirement Reason:** Retired in iteration 3 (LD-3, LD-11). The quarantine `staging_queue` table has been completely eliminated from the architecture. Candidate requirements extracted during document decomposition and agent proposals are now written directly to `graph_nodes` and `graph_edges` with `lifecycle_state = 'DRAFT'` and `job_id REFERENCES ingestion_jobs(job_id)`. Because candidate nodes exist as first-class relational entities with strong schema typing, multi-version JSON blob staging and the versioned Rust enum `StagedPayload` are obsolete.

---

## TB-4

* **ID:** TB-4
* **Title:** CLI Stdio Adapter Connection Resilience, Backoff Retry, and Structured Initialization Error Formatting
* **Origin:** LD-9, iteration 3
* **Status:** Active
* **Description/Tactical Details:**
  1. **Resilient Connection Check with Backoff:** In the `tks mcp-stdio` CLI proxy implementation (`src/bin/mcp_stdio.rs`), implement a graceful startup check when connecting to the `tks serve` daemon. If the HTTP/SSE connection fails initially, poll with exponential backoff (e.g. initial 50ms up to 2 seconds total) or inspect a local daemon lock/PID file before declaring the service unavailable.
  2. **Structured JSON-RPC Initialization Error:** If `tks serve` remains unreachable after the retry window, avoid abruptly terminating the process with a non-zero exit code (which causes external agent harnesses such as Claude Code, Cursor, and Windsurf to treat the adapter as crashed and disable the integration). Instead, wait for the agent's incoming JSON-RPC `initialize` request on `stdin` and respond on `stdout` with a valid JSON-RPC error response containing clear, actionable remediation guidance:

     ```json
     {
       "jsonrpc": "2.0",
       "id": 1,
       "error": {
         "code": -32000,
         "message": "TKS daemon is not running. Please start 'tks serve' in another terminal to enable Knowledge Substrate integration."
       }
     }
     ```

  3. **Diagnostic Stream Separation:** Ensure all diagnostic connection status messages, warning traces, and retry logs write exclusively to `stderr`, preserving standard output strictly for valid JSON-RPC protocol framing.
  4. **Dual Authentication Extraction for Mutation Tools:** In `src/gateway/mcp/mod.rs`, support caller authentication extraction from both HTTP `Authorization: Bearer <token>` headers and top-level or parameters-level `auth_token` fields in JSON-RPC payloads. Unauthenticated attempts to invoke any mutation tool fail immediately with standardized JSON-RPC error code `-32000` and payload `{ "code": "ERR_AUTH_FAILED", "message": "Authentication required for mutation tool" }` (DEC-2.10).

---

## TB-5

* **ID:** TB-5
* **Title:** Identity Provisioning & Revocation via Daemon REST Gateway and Development Migration Seed
* **Origin:** LD-10, iteration 3; LD-8, iteration 5
* **Status:** Active
* **Description/Tactical Details:**
  1. **REST-Driven Administrative CLI Commands:** Implement identity management subcommands in the `tks` binary (`tks identity create` and `tks identity revoke`) as thin HTTP clients targeting the local `tks serve` daemon REST endpoints:
     * `tks identity create --name <name> --role <HUMAN|AGENT> [--token <custom_token>]` sends `POST /api/v1/identities`. The daemon inserts the record into PostgreSQL `agent_identities`, seeds or validates the in-process `moka` LRU cache, and returns the bearer token.
     * `tks identity revoke <agent_id>` sends `POST /api/v1/identities/{id}/revoke`. The daemon updates PostgreSQL `agent_identities SET is_active = false` and immediately evicts the token from its in-process `moka` LRU cache (`cache.invalidate(&token_hash)`), ensuring zero delay in blast-radius security containment. Direct database connection fallback is permitted only if `tks serve` is offline.
  2. **Development Environment Seed:** In database migrations (`refinery` / SQL migrations), include an automatic development seed that executes when `ENVIRONMENT=development` (or during initial local test database bootstrapping). The seed provisions a well-known development token (`tks_dev_token`) with role `HUMAN` and `is_active = true`, ensuring that fresh local environments can immediately run integration tests, ingest initial documents, and complete Gate 1 dogfooding without manual SQL seeding.

---

## TB-6

* **ID:** TB-6
* **Title:** Context Envelope Vector Neighbor Query Resolution, In-Process Embedding Engine, and Fallback (`src/storage/envelope.rs`, `src/worker/embedding.rs`)
* **Origin:** LD-9, iteration 5; LD-11, iteration 6; LD-8, iteration 10
* **Status:** Active
* **Description/Tactical Details:**
  1. **In-Process CPU Vector Inference Default (`fastembed-rs`):** In `src/worker/embedding.rs`, vector inference executes locally via `fastembed-rs` (`AllMiniLML6V2`) running in-process on CPU offloaded via `tokio::task::spawn_blocking`. This delivers sub-15ms deterministic embedding generation per text chunk without network round-trips, rate limit contention, or quota exhaustion.
  2. **Scoped Remote Rate Limiting & Backoff:** HTTP 429 rate limiting, token bucket throttling, and exponential backoff retry policies are isolated exclusively to optional remote embedding provider integrations (e.g. OpenAI or Ollama HTTP endpoints) when configured, never impacting local-first CPU inference.
  3. **Target Node Query Vector Resolution:** In `src/storage/envelope.rs`, implement query vector resolution for vector neighbor retrieval in `get_context_envelope(target_node_id, depth)`. When allocating slots for vector neighbor ranking, the repository queries `node_embeddings` for `target_node_id` (`SELECT embedding FROM node_embeddings WHERE node_id = $1`) to serve as the reference query vector for similarity search against other active nodes.
  4. **Ancestor Requirement Embedding Fallback for Newly Elaborated Tasks:** If `target_node_id` lacks an embedding record in `node_embeddings` (e.g. for a newly created or elaborated execution task, since candidate tasks do not enqueue embeddings until active and newly claimed tasks may have embeddings pending in `node_embeddings` with `status = 'PENDING'`), resolve the reference query vector from its immediate parent requirement node:

     ```sql
     SELECT embedding FROM node_embeddings
     WHERE node_id = (
         SELECT to_node_id FROM graph_edges
         WHERE from_node_id = $1 AND edge_type IN ('FULFILLS', 'CONSTRAINED_BY', 'DERIVED_FROM')
         LIMIT 1
     ) AND status = 'COMPLETED';
     ```

     This allows newly created tasks to immediately benefit from vector-enriched cross-cutting governance constraints without requiring synchronous embedding calculation.
  5. **Pending Embedding Graceful Fallback:** If both `target_node_id` and its parent requirement node lack an embedding in `node_embeddings` with `status = 'COMPLETED'` (because asynchronous embedding generation is pending, throttled by backoff, or external embedding API is disabled), the repository layer must gracefully skip vector similarity search.
  6. **Dynamic Quota Reallocation:** When vector neighbor search is skipped, reallocate the full 40-node context envelope budget to deterministic topological recursive CTE traversal. This ensures external agents always receive a rich, bounded context envelope of ancestor requirements and sibling constraints without failing, waiting on external APIs, or timing out.
  7. **Recursive Invalidation CTE Single-Pass Aggregation & Plan Optimization:** In `src/storage/cascade.rs`, optimize the multi-statement invalidation cascade CTE (`DOWNWARD_INVALIDATION_SQL`) for scale execution by projecting `(delta->>'depth')::int4 AS depth` directly from `inserted_audit RETURNING` into an `audit_summary` CTE that computes `array_agg(ia.entity_id ORDER BY ia.event_seq ASC)`, `coalesce(max(ia.depth), 0)`, and `coalesce(max(ia.event_seq), 0)` in a single scan while dispatching `pg_notify`. Eliminates multiple correlated subquery rescans over modified tables, reducing p95 cascade sweep latency across $10^4$ nodes from ~13.4ms to ~1.86ms (DEC-3.13).

---

## TB-7

* **ID:** TB-7
* **Title:** Canonical Node Key Lexical Extraction, Polymorphic Identifier Resolution, Query Sanitization, and Document Revision Reconciliation
* **Origin:** LD-2, LD-5, iteration 6; LD-7, LD-8, LD-9, LD-11, iteration 7; LD-1, LD-6, LD-8, LD-9, iteration 8; LD-3, iteration 10
* **Status:** Active
* **Description/Tactical Details:**
  1. **Canonical Node Key Extraction & Ingestion Tagging:** In `src/ingest/parser.rs`, extend the deterministic RFC 2119 keyword scanner (TB-2) to extract human-readable canonical identifiers (`REQ-*`, `INV-*`, `DR-*`, `C-*`, `TASK-*`) from section headings and block labels, populating `graph_nodes.node_key VARCHAR(64)` during mechanical decomposition.
  2. **Polymorphic Identifier Resolution:** In `src/storage/envelope.rs` and MCP tool handlers (`get_context_envelope`, `query_requirements`, `propose_node_mutation`), implement polymorphic resolution supporting both UUID and string `node_key`: inspect input string; if valid UUID syntax, query `WHERE id = $1::uuid`; otherwise, query `WHERE node_key = $1` utilizing the partial unique index `idx_graph_nodes_node_key_active`.
  3. **Query Sanitization for Hyphenated Canonical Keys:** In `src/storage/envelope.rs`, sanitize search queries executed by `query_requirements`: when the input query matches a canonical identifier pattern (regex `^[A-Za-z]+-[0-9A-Za-z-]+$`), bypass `websearch_to_tsquery` to avoid hyphen negation interpretation (`NOT`), and execute an exact/prefix key match combined with `plainto_tsquery`:

     ```sql
     SELECT id, node_key, title, content
     FROM graph_nodes
     WHERE lifecycle_state = 'ACTIVE'
       AND (node_key ILIKE $1 || '%' OR search_tsv @@ plainto_tsquery('english', $1))
     ORDER BY ts_rank(search_tsv, plainto_tsquery('english', $1)) DESC
     LIMIT $2;
     ```

     This guarantees sub-5ms exact resolution for standard keys (e.g. `REQ-CORE-001`, `INV-2`) while preserving natural language full-text search (addressing LD-7, iteration 7).
  4. **AST Revision Diffing & 3-Tier Reconciliation:** In the document decomposition worker (`src/worker/decomp.rs` / `src/ingest/reconcile.rs`), on re-ingestion of a document at persistent `doc_path`, correlate extracted CommonMark AST structural blocks against existing active nodes anchored to that `doc_path` using a strict three-tier priority hierarchy:
     * *Tier 1 (Canonical Key Match):* Match by explicit `node_key` if defined.
     * *Tier 2 (Structural Heading Anchor Match):* Match by deterministic, disambiguated hierarchical heading anchor (`ast_anchor` with heading slug disambiguation and ordinal block indexing per TB-2, e.g. `doc_path#section/overview-1`, `doc_path#section/heading#block-0`).
     * *Tier 3 (Content Hash Match):* Match by normalized SHA-256 hash of title and text content.
  5. **Staged Candidate Span Re-Anchoring & Atomic Approval Execution:** Following the consolidation of document span coordinates directly as columns on `graph_nodes` (`doc_path`, `doc_hash`, `byte_start`, `byte_end`), span re-anchoring must NEVER mutate active production requirements prior to human approval (addressing LD-1, iteration 8):
     * *Candidate Coordinate Staging:* When an active node's text content matches across document revisions and only byte offsets shifted, the decomposition worker records candidate re-anchored coordinates within `ingestion_jobs` candidate attributes (or candidate metadata). The active node row in `graph_nodes` remains untouched and continues to reference the approved Git blob and byte offsets during the staging window.
     * *Atomic Approval Execution:* The physical in-place update of `doc_hash`, `byte_start`, and `byte_end` on active nodes executes strictly inside the atomic staging approval transaction (`POST /api/v1/staging/approve` or `POST /api/v1/documents/ingest/{job_id}/approve`) under `pg_advisory_xact_lock(hashtext('tks_structural_mutation'))`.
     * *Audit Attribution:* The atomic update generates a discrete `SPAN_REANCHORED` audit event in `audit_ledger` with monotonic `event_seq` and transaction correlation `batch_id`.
     * *Zero Residue on Rejection:* If the supervisor rejects or cancels the ingestion job, the candidate coordinates are discarded, leaving active production nodes completely unmutated.
     * *Modified & Omitted Content:* For modified content, stage candidate `DRAFT` nodes referencing `replaces_node_id`. Upon staging approval, supersede previous active nodes and mark downstream child tasks `NEEDS_REVERIFICATION`. For omitted sections, active nodes anchored to `doc_path` missing from the revised candidate set must not be automatically superseded or transitioned to a preliminary deprecation state (which would trigger premature downward invalidation cascades on child tasks and violate check constraints); instead, active nodes remain strictly in `ACTIVE` state, and candidate omissions are recorded in `ingestion_jobs.attributes->'omitted_node_ids'` to be presented to the supervisor for explicit confirmation. Hard supersession to `SUPERSEDED` / `DEPRECATED` and downward invalidation cascades execute only if the supervisor explicitly confirms omission during staging approval (addressing LD-10, iteration 9; LD-3, iteration 10; D-109).
  6. **Worker Concurrency Hardening & Timeout Reclaim Draft Purging:** To prevent race conditions and duplicate draft accumulation during background decomposition (addressing LD-6, iteration 8):
     * *Conditional Terminal Transition:* When the background worker finishes Stage 2 decomposition, it executes a conditional status update:
       `UPDATE ingestion_jobs SET status = 'STAGED', updated_at = NOW() WHERE job_id = $1 AND status = 'PROCESSING';`
       If zero rows are updated (because the job was superseded by a document re-ingestion sweep or cancelled), the worker immediately aborts and purges its candidate draft nodes:
       `DELETE FROM graph_nodes WHERE job_id = $1 AND lifecycle_state = 'DRAFT';`
     * *Atomic Timeout Reclaim Purge:* When a background worker reclaims a timed-out `PROCESSING` job (`updated_at < NOW() - INTERVAL '180s'`), the worker atomically purges any existing partial candidate draft nodes (`DELETE FROM graph_nodes WHERE job_id = $1 AND lifecycle_state = 'DRAFT';`) before restarting AST parsing and semantic classification, preventing duplicate candidate draft rows.
  7. **Governance Policy Inheritance for Autonomous Tasks:** In `src/storage/mutation.rs`, when a verified agent elaborates an execution sub-task (`node_type = 'TASK'`) directly into `ACTIVE` state under a parent node with `governance_policy = 'AUTONOMOUS_ELABORATION'`, the newly created task inherits the parent's `governance_policy` (`AUTONOMOUS_ELABORATION`) by default unless explicitly overridden in the mutation payload (addressing LD-9, iteration 8). This ensures that the agent retains permission to update its own task status (`IN_PROGRESS`, `COMPLETED`) under native row locks without requiring supervisory intervention or violating NOT NULL check constraints.

---

## TB-8

* **ID:** TB-8
* **Title:** Operational Task Management REST Endpoints and CLI Operational Subcommand Suite (`tks task`, `tks admin`)
* **Origin:** phase2-decisions.md DEC-2.12
* **Status:** Implemented
* **Description/Tactical Details:**
  1. **Dedicated Task Enumeration Endpoint:** In `src/gateway/routes/mutation.rs`, implement `GET /api/v1/tasks` accepting `parent_id: Option<Uuid>`, `status: Option<TaskStatus>`, and `limit: Option<i64>`, querying `graph_nodes` joined with upward `graph_edges` under `lifecycle_state = 'ACTIVE'`.
  2. **Thin HTTP Client CLI Commands:** In `src/cli/task.rs` and `src/cli/admin.rs`, implement operational CLI commands interacting with running `tks serve` instances:
     * `tks task create --parent <id> --title <title> [--content <desc>] [--assignee <agent>]`
     * `tks task update <id> --status <OPEN|IN_PROGRESS|BLOCKED|COMPLETED>`
     * `tks task list [--parent <id>] [--status <status>] [--limit <n>]`
     * `tks admin revert [--batch-id <id>] [--agent-id <agent>] [--dry-run] [--force]`
     * `tks admin reverify <id> [--rationale <reason>] [--reparent-to <new_parent>]`
  3. **Zero Direct DB Connections:** CLI commands execute as thin HTTP clients communicating via REST, preserving connection pool isolation and gateway security middleware.

---

## TB-9

* **ID:** TB-9
* **Title:** Isolated Temporary Schema Architecture with Hex-Encoded UUIDs for Large-Scale Graph Benchmarks
* **Origin:** phase2-decisions.md DEC-2.13
* **Status:** Implemented
* **Description/Tactical Details:**
  1. **Hex-Encoded Temporary Schema Namespace:** In `benches/context_envelope_bench.rs`, format temporary benchmark schema names as `bench_scale_<hex_uuid>` (stripping hyphens from UUID v4 to adhere to PostgreSQL SQL identifier rules without requiring quoted identifier escaping).
  2. **Zero Test Residue & Zero Table Bloat:** Apply initial DDL migrations into the temporary schema, populate $10^5$ synthetic nodes and $>109,000$ edges across depths 1 to 6, execute SLA-2 benchmark iterations under `search_path = bench_scale_<hex_uuid>`, and tear down the schema via `DROP SCHEMA bench_scale_<hex_uuid> CASCADE`. This guarantees zero contamination of functional test suites, zero WAL bloat from bulk row deletes, and reproducible sub-2ms p95 traversal latency verification.

---

## TB-10

* **ID:** TB-10
* **Title:** PostgreSQL Template Database Isolation Pattern (`TEMPLATE tks_template`) for Safe Parallel Test Execution
* **Origin:** Phase 3 Test Harness Hardening; phase3-decisions.md DEC-3.4, DEC-3.8
* **Status:** Implemented
* **Description/Tactical Details:**
  1. **Per-Binary Database Derivation:** In `src/db.rs`, implement `ensure_test_database_ready_for` and `resolve_test_database_name`, dynamically deriving isolated test database names from `std::env::current_exe()` (e.g. `tks_test_cascade_engine`, `tks_test_dogfood_gate3`).
  2. **Advisory-Locked Template Initialization:** Bootstrap `tks_template` under PostgreSQL advisory lock `pg_advisory_xact_lock(hashtext('tks_template_init'))`, applying all embedded refinery migrations (V1 through V4) exactly once.
  3. **Instantaneous Database Cloning:** Each test suite provisions its isolated database via `CREATE DATABASE tks_test_<suite> TEMPLATE tks_template` in $<200\text{ ms}$, eliminating table lock contention and race conditions during parallel `cargo test --all-targets --all-features`.

---

## TB-11

* **ID:** TB-11
* **Title:** Brownfield Intent Scaffolding Adapters, Root Requirement Synthesis, and Authority Inheritance Pipeline
* **Origin:** LD-6, iteration 9; LD-4, iteration 10
* **Status:** Active
* **Description/Tactical Details:**
  1. **Pluggable ScaffoldingAdapter Trait:** In `src/ingest/scaffolding.rs`, define a pluggable `ScaffoldingAdapter` trait supporting format-specific mechanical parsing:
     ```rust
     pub trait ScaffoldingAdapter: Send + Sync {
         fn can_handle(&self, path: &str, content: &str) -> bool;
         fn parse(&self, path: &str, content: &str) -> Result<Vec<ExtractedChunk>, IngestionError>;
     }
     ```
  2. **Zero-Token OpenAPI and AsyncAPI Schema Parser with Root Requirement Synthesis:** Implement `OpenApiAdapter` using `serde_yaml` / `serde_json` to parse OpenAPI 3.0/3.1 specifications. To satisfy Invariant INV-1 without requiring manual requirement authoring, the adapter mechanically synthesizes a root `REQUIREMENT` node from the document-level container (`info.title`, `info.description`, `info.version`) as the structural anchor. It then extracts path operations (`paths.<path>.<method>`) and component schemas (`components.schemas.<name>`) directly into normalized `SPECIFICATION` nodes with canonical keys (e.g. `API-GET-USERS`), linking each specification upward to the synthesized root requirement via directed `DERIVED_FROM` edges.
  3. **Architecture Decision Record (ADR) Parser:** Implement `AdrAdapter` matching standard MADR and Nygard ADR templates (`docs/adr/*.md`). Mechanically synthesize a root `REQUIREMENT` node for the architectural domain or ADR collection, and map accepted individual records to `DECISION` nodes with canonical keys (e.g. `ADR-0012`), linking them upward via `DERIVED_FROM` edges.
  4. **Spec-Driven Development (SDD) & Agent Steering Parsers:** Implement zero-token mechanical parsers for GitHub Spec Kit (`.github/specs/`), AWS Kiro (`.kiro/`), BDD Feature files, and repository instruction files (`AGENTS.md`, `CLAUDE.md`). Decompose hierarchy into a synthesized root `REQUIREMENT` (e.g. from BDD `Feature: <Title>`) and child `REQUIREMENT` / `SPECIFICATION` nodes with upward `DERIVED_FROM` edges.
  5. **Authority Inheritance Pipeline & Source Trust Binding:** In `src/ingest/job.rs`, evaluate the `authority_source` attribute of the ingestion request (e.g. `BRANCH_PROTECTED_CODEOWNERS`, `COMMITTED_SPEC_MAIN`). When verified against pre-governed sources, candidate nodes inherit baseline authority: the ingestion pipeline bypasses manual staging approval, atomically inserts records directly into `graph_nodes` in `ACTIVE` state (root requirement and child specifications/decisions linked upward), enqueues embedding tasks, and records canonical provenance in `audit_ledger` with `actor_type = 'SYSTEM_AUTHORITY_IMPORT'`. This guarantees that brownfield imported trees strictly adhere to INV-1 upon entry into the graph.

---

## TB-12

* **ID:** TB-12
* **Title:** Pre-Merge CI Test-Run Task Attribute Reporting, Merge Commit Materialization, and Scoped Release Readiness
* **Origin:** LD-1, LD-12, iteration 9; LD-1, LD-2, LD-7, iteration 10
* **Status:** Active
* **Description/Tactical Details:**
  1. **Pre-Merge Test Run Ingestion (`POST /api/v1/verification/test-run`):** In `src/gateway/routes/verification.rs`, implement `POST /api/v1/verification/test-run` accepting test suite execution payloads (`test_suite: String`, `passed: bool`, `pr_commit_sha: String`, `target_task_ids: Vec<Uuid>`, `duration_ms: u64`, `manifest_hash: String`). Because `graph_edges` does not have edge attributes and `TASK` is a single endpoint, pre-merge test runs must not create relationally invalid `VERIFIED_BY` edges on `TASK`. Instead, for each targeted active `TASK` node, the handler atomically appends the test run object directly to the task's JSONB array in `graph_nodes.attributes->'pre_merge_verification'` (mirroring `attributes->'vcs_commits'`). Standardizes pre-merge verification without polluting the property graph with provisional `CODE_COMMIT` nodes or violating edge relational integrity (addressing LD-1, iteration 10; D-107).
  2. **Canonical Merge Commit Materialization (`POST /api/v1/vcs/commits`):** In `src/gateway/routes/vcs.rs`, implement the VCS commit webhook handler. When a pull request merges into `main` (or when a release tag is pushed), the endpoint creates a permanent `CODE_COMMIT` node in `graph_nodes` (`node_type = 'CODE_COMMIT'`, `lifecycle_state = 'ACTIVE'`) with commit SHA, author, and timestamp. It queries task execution attributes (`attributes->'vcs_commits'`) or correlates PR commit history, atomically generating upward directed `IMPLEMENTS` edges pointing from the newly created `CODE_COMMIT` node to resolved `TASK` entities (`from_node_id = commit_id, to_node_id = task_id, edge_type = 'IMPLEMENTS'`) under `pg_advisory_xact_lock`. This reorients commit lineage upward, ensuring that every active `CODE_COMMIT` maintains an unbroken path to an active `REQUIREMENT` without inverting the root-directed invariant INV-1 (addressing LD-2, iteration 10; D-108).
  3. **Scoped Release Readiness Inspection (`GET /api/v1/release/readiness`):** In `src/gateway/routes/release.rs`, implement `GET /api/v1/release/readiness` accepting `scope: ReleaseScope` (`pre-merge` or `release`) alongside target qualifiers (`pr_commit_sha: Option<String>`, `requirement_root_id: Option<Uuid>`, `milestone: Option<String>`). Resolves the circular gating deadlock by providing distinct verification stages:
     * *Pre-Merge Scope (`scope=pre-merge&pr_commit_sha=<sha>`):* Evaluates pull request readiness prior to merge by verifying that target `TASK` nodes are active and their `attributes->'pre_merge_verification'` records passing CI runs for the specified PR commit SHA. It explicitly does not require materialized `CODE_COMMIT` nodes or `IMPLEMENTS` edges.
     * *Post-Merge / Release Scope (`scope=release&milestone=<tag>`):* Evaluates deployment milestone readiness by executing recursive CTE traversal from active `REQUIREMENT` nodes through `SPECIFICATION` and `TASK` nodes, verifying terminating upward `IMPLEMENTS` edges from canonical `CODE_COMMIT` nodes and valid test verifications. Returns a structured readiness summary (e.g. `scope`, `satisfied_requirements`, `unverified_tasks`, `uncommitted_tasks`, `readiness_percentage`) (addressing LD-7, iteration 10; D-113).

---

## TB-13

* **ID:** TB-13
* **Title:** MCP camelCase Wire Protocol Conformance, Multi-Axis Planning Context Handler (`get_elaboration_context`), Edge Target Pre-Validation, and Actionable Remediation Envelope Builder
* **Origin:** LD-3, LD-4, LD-5, iteration 9; LD-5, iteration 10
* **Status:** Active
* **Description/Tactical Details:**
  1. **MCP Wire Protocol camelCase Serialization:** In `src/gateway/mcp/mod.rs` and `src/bin/mcp_stdio.rs`, audit all JSON-RPC serialization structs. Ensure tool definition schemas serialize parameter schemas strictly under `inputSchema` (camelCase) rather than `input_schema` (snake_case), adhering to the official MCP 2024-11-05 specification.
  2. **Multi-Axis Planning Dossier Endpoint (`get_elaboration_context`):** In `src/storage/envelope.rs` and `src/gateway/mcp/tools.rs`, implement `get_elaboration_context(node_id: String)`. When an agent requests elaboration context for a parent requirement or deliverable:
     * *Axis 1 (Normative Boundaries):* Queries and formats active system invariants (`INV-1` through `INV-9`) and governance policies.
     * *Axis 2 (Architectural Precedents):* Retrieves historical `D-*` decision records and tags matching the component domain.
     * *Axis 3 (Physical Schema Contracts):* Dynamically queries first-class `SCHEMA_CONTRACT` nodes in `graph_nodes` (or active OpenAPI/JSON-Schema specifications registered in the repository) rather than static hardcoded prompts, synthesizing active PostgreSQL enum constraints (`chk_node_type`, `chk_lifecycle_state`), edge type rules, and required attribute keys.
     * *Axis 4 (Codebase Blueprints):* Dynamically queries first-class `CODE_BLUEPRINT` nodes in `graph_nodes` or the specs tree manifest (`refs/heads/specs`) rather than hardcoded pattern strings, surfacing canonical repository implementation patterns (e.g. Axum route handler templates, CTE query conventions, test templates).
  3. **Pre-Transaction Relational Edge Target Validation:** In `src/storage/mutation.rs`, before initiating any edge creation transaction (`propose_node_mutation`, `create_subtask`), execute an endpoint pre-validation check: verify that both `from_node_id` and `to_node_id` parse as valid UUIDs and exist in `graph_nodes`. If a caller supplies an invalid format or non-existent ID (such as an external Git commit SHA string), reject immediately with HTTP 422 / JSON-RPC error code `ERR_INVALID_EDGE_TARGET`, returning `{ "code": "ERR_INVALID_EDGE_TARGET", "message": "Edge endpoint is not a registered node UUID", "invalid_field": "to_node_id", "invalid_value": "<value>" }`.
  4. **Actionable Remediation Envelope Builder:** In `src/gateway/error.rs`, implement structured remediation payload generation for Invariant INV-1 ancestry failures (`ERR_INVALID_ANCESTOR_PATH`). Include the target node ID, terminal node ID, terminal node type (`SPECIFICATION`), and deterministic repair options (`PROMOTE_ANCESTOR` action with `node_id`, `target_type = 'REQUIREMENT'`, and `authorized = true/false` based on caller claims; or `REORIENT_EDGE` / `LINK_PARENT_TASK` to ensure upward directed ancestry).
