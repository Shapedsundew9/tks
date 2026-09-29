# Technical Implementation Backlog

This document captures vital component-level designs, library evaluation tasks, sprint-level tactical choices, and downstream implementation details deferred from architectural review.

---

## TB-1

* **ID:** TB-1
* **Title:** Bare Git Repository Direct ODB Ingestion, Document Path Tree Mapping, Dedicated Write Actor, Concurrent ODB Reads, and Volume Backup Synchronization
* **Origin:** LD-8, iteration 1; LD-5, iteration 2; LD-12, iteration 3; LD-3, iteration 4; LD-1, LD-2, iteration 5; LD-8, iteration 6; LD-12, iteration 7
* **Status:** Active
* **Description/Tactical Details:**
  1. **Low-Level ODB Direct Writes & Commit-Tree Graph:** In the Git document adapter module (`src/storage/git/`), configure the `git2` crate to interact with a bare Git repository (`git init --bare`). Implement direct object-database (ODB) writes via `git_blob_create_from_buffer` for raw document streams and construct standard tree and commit objects directly within the ODB backend targeting a dedicated specifications branch (`refs/heads/specs`). This bypasses the Git working tree and index (`.git/index`), completely eliminating index lock collisions (`.git/index.lock`) during concurrent document ingestion calls.
  2. **Document Path/Slug Tree Structure (`doc_path`):** When constructing the Git tree object in the ODB for a new or updated document, the adapter places the blob at its persistent document path/slug (`doc_path`, e.g. `specs/vision.md`). Storing documents under stable paths across successive revisions guarantees that standard Git history tracking works out-of-the-box (`git log specs/vision.md`, `git diff HEAD~1 specs/vision.md`), preserves document revision history across commits, and enables the decomposition pipeline to correlate prior active AST blocks and source byte spans on re-ingestion.
  3. **Dedicated Background Git Write Actor Task (Channel-Based Isolation & Panic Recovery):** To eliminate async reactor starvation and thread-safety hazards with non-`Sync` C pointers in `git2::Repository`, encapsulate all bare Git write and commit operations within a dedicated background actor thread (`src/storage/git/actor.rs`). The Git actor exclusively owns the writable `git2::Repository` handle and receives `CommitCommand` requests over a bounded Tokio mpsc channel (`tokio::sync::mpsc::Sender<GitCommand>`). The actor executes blocking libgit2 C calls sequentially off the Tokio async worker pool and returns results via `tokio::sync::oneshot` channels. The actor encapsulates its sequential command execution loop inside an internal panic recovery handler (`std::panic::catch_unwind`), catching transient libgit2 C-binding panics and returning structured error responses over the caller's oneshot channel without dropping the underlying `Receiver<GitCommand>` or disconnecting caller `Sender` clones held across the gateway and workers. This guarantees linear serialization on `refs/heads/specs` without `.lock` collisions (`GIT_ELOCKED`), prevents Tokio worker thread starvation, eliminates Rust `Send`/`Sync` boundary conflicts, and prevents permanent `SendError` channel breaks.
  4. **Concurrent Read-Only ODB Access (`tokio::task::spawn_blocking`):** Restrict the dedicated Git actor exclusively to serializing Git commits and tree updates (`refs/heads/specs` writes). For read-only blob extraction (`get_document_span`, worker decomposition), gateway handlers and workers open thread-local read-only repository handles or direct ODB blob lookups via `tokio::task::spawn_blocking` using `git2::Odb::read`. Because Git ODB blobs are content-addressed and strictly immutable once written, concurrent reads never block on `.git/refs/...lock`, execute concurrently without queuing behind write commits, and eliminate head-of-line blocking on the Git actor (addressing LD-12, iteration 7).
  5. **Dedicated Storage Volume Binding:** In `docker-compose.yml` and container deployment manifests, configure a dedicated, persistent filesystem volume mount for the bare Git repository co-located with the PostgreSQL persistent data volume, ensuring Git blob storage persists across container recreation and redeployment cycles.
  6. **Point-in-Time Backup & Restore Synchronization:** Author operational shell scripts in `scripts/` to orchestrate coordinated point-in-time backups: trigger a PostgreSQL WAL checkpoint / `pg_dump` and execute a simultaneous snapshot of the bare Git repository object database (`objects/` and `refs/`), preventing referential drift between PostgreSQL blob hash columns and underlying Git blobs.

---

## TB-2

* **ID:** TB-2
* **Title:** Mechanical CommonMark AST Parsing, Structural Hierarchy Edge Generation, RFC 2119 Lexical Extraction Benchmark, and Zero-Copy Byte Slicing (`pulldown-cmark`)
* **Origin:** LD-1, iteration 2; LD-2, iteration 3; LD-3, LD-9, iteration 7
* **Status:** Active
* **Description/Tactical Details:**
  1. **Streaming CommonMark Parser Implementation:** In the document ingestion module (`src/ingest/parser.rs`), implement a streaming Markdown parser using `pulldown-cmark`. Traverse the CommonMark event stream to segment ingested documents along structural boundaries (headings H1–H4, tables, blockquotes, and bulleted lists).
  2. **Zero-Token Deterministic Byte Span Capture:** For each structural block event, extract exact 0-based byte offsets (`byte_start`, `byte_end`) directly from the source stream offset tracking provided by `pulldown-cmark`'s `OffsetIter`. Pointers are pinned with 100% precision without LLM inference cost.
  3. **AST Structural Hierarchy Edge Generation Rule:** During Stage 1 streaming AST parsing, maintain an active heading stack tracking nesting levels (H1 through H4). Every extracted child heading or sub-block mechanically records a directed upward structural edge (`DERIVED_FROM`) pointing from the child node to its immediate parent heading node (`child_id -DERIVED_FROM-> parent_id`). When Stage 2 classifies child chunks as `SPECIFICATION` or `TASK`, this mechanical edge guarantees an unbroken path to the enclosing requirement section, ensuring the transactional ancestor CTE during staging promotion succeeds without manual edge authoring (addressing LD-3, iteration 7).
  4. **Hierarchical Heading Anchor Generation (`ast_anchor`):** Concurrently with heading stack tracking, generate a deterministic slugified heading path anchor for each block (e.g. `specs/vision.md#section-1/subsection-a`), stored in `graph_nodes.attributes->'ast_anchor'`. This deterministic structural anchor serves as the second-tier correlation key during document revision diffing when canonical `node_key` is not defined (addressing LD-9, iteration 7).
  5. **Zero-Copy Byte Slicing & UTF-8 Safety:** Standardize span extraction in the Rust service layer on raw byte slicing against the source buffer (`&source_bytes[byte_start..byte_end]`) prior to UTF-8 validation (`std::str::from_utf8`), eliminating string slicing panics on multi-byte UTF-8 sequences (em-dashes, curly quotes, math symbols).
  6. **Rule-Based RFC 2119 & Entity Lexical Matcher:** Implement a deterministic scanner matching RFC 2119 keywords (`MUST`, `MUST NOT`, `REQUIRED`, `SHALL`, `SHALL NOT`, `SHOULD`, `SHOULD NOT`, `RECOMMENDED`, `MAY`, `OPTIONAL`) and project entity tags (`REQ-*`, `INV-*`, `DR-*`, `C-*`, `TB-*`). Structural chunks containing these patterns are tagged as candidate requirements.
  7. **Benchmarking & Token Reduction Calibration:** Benchmark parsing throughput, memory footprint, and token reduction across representative PRDs and specification documents of varying sizes (3,000 to 10,000 words). Verify that mechanical extraction handles $\ge 80$% of structural decomposition before any external LLM invocation, measuring output token savings.

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
* **Title:** Context Envelope Vector Neighbor Query Resolution and Embedding Fallback (`src/storage/envelope.rs`)
* **Origin:** LD-9, iteration 5; LD-11, iteration 6
* **Status:** Active
* **Description/Tactical Details:**
  1. **Target Node Query Vector Resolution:** In `src/storage/envelope.rs`, implement query vector resolution for vector neighbor retrieval in `get_context_envelope(target_node_id, depth)`. When allocating slots for vector neighbor ranking, the repository queries `node_embeddings` for `target_node_id` (`SELECT embedding FROM node_embeddings WHERE node_id = $1`) to serve as the reference query vector for similarity search against other active nodes.
  2. **Ancestor Requirement Embedding Fallback for Newly Elaborated Tasks:** If `target_node_id` lacks an embedding record in `node_embeddings` (e.g. for a newly created or elaborated execution task, since candidate tasks do not enqueue embeddings until active and newly claimed tasks may have embeddings pending in `embedding_queue`), resolve the reference query vector from its immediate parent requirement node:

     ```sql
     SELECT embedding FROM node_embeddings
     WHERE node_id = (
         SELECT to_node_id FROM graph_edges
         WHERE from_node_id = $1 AND edge_type IN ('FULFILLS', 'CONSTRAINED_BY', 'DERIVED_FROM')
         LIMIT 1
     );
     ```

     This allows newly created tasks to immediately benefit from vector-enriched cross-cutting governance constraints without requiring synchronous embedding calculation.
  3. **Pending Embedding Graceful Fallback:** If both `target_node_id` and its parent requirement node lack an embedding in `node_embeddings` (because asynchronous embedding generation is pending, throttled by backoff, or external embedding API is disabled), the repository layer must gracefully skip vector similarity search.
  4. **Dynamic Quota Reallocation:** When vector neighbor search is skipped, reallocate the full 40-node context envelope budget to deterministic topological recursive CTE traversal. This ensures external agents always receive a rich, bounded context envelope of ancestor requirements and sibling constraints without failing, waiting on external APIs, or timing out.

---

## TB-7

* **ID:** TB-7
* **Title:** Canonical Node Key Lexical Extraction, Polymorphic Identifier Resolution, Query Sanitization, and Document Revision Reconciliation
* **Origin:** LD-2, LD-5, iteration 6; LD-7, LD-8, LD-9, LD-11, iteration 7
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
     * *Tier 2 (Structural Heading Anchor Match):* Match by deterministic hierarchical heading anchor (`ast_anchor` from TB-2, e.g. `specs/vision.md#architecture/storage`).
     * *Tier 3 (Content Hash Match):* Match by normalized SHA-256 hash of title and text content.
  5. **In-Place Embedded Span Re-Anchoring:** Following the elimination of the standalone `source_spans` table (LD-11, iteration 7), document span coordinates are stored directly as columns on `graph_nodes` (`doc_path`, `doc_hash`, `byte_start`, `byte_end`). When an active node's text content is identical across document revisions and only byte offsets shifted:
     * Update `graph_nodes` columns (`doc_hash = $2, byte_start = $3, byte_end = $4`) in place.
     * Do NOT generate candidate replacement nodes or orphan drafts.
     * For modified content, stage candidate `DRAFT` nodes referencing `replaces_node_id`. Upon staging approval, supersede previous active nodes and mark downstream child tasks `NEEDS_REVERIFICATION`.
     * For deleted sections, transition active nodes anchored to `doc_path` missing from the approved revision to `SUPERSEDED`.
