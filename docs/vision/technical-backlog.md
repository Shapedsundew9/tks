# Technical Implementation Backlog

This document captures vital component-level designs, library evaluation tasks, sprint-level tactical choices, and downstream implementation details deferred from architectural review.

---

## TB-1

* **ID:** TB-1
* **Title:** Bare Git Repository Direct ODB Ingestion, Standard Branch Commits (`refs/heads/specs`), and Volume Backup Synchronization
* **Origin:** LD-8, iteration 1; LD-5, iteration 2; LD-12, iteration 3
* **Status:** Active
* **Description/Tactical Details:**
  1. **Low-Level ODB Direct Writes & Commit-Tree Graph:** In the Git document adapter module (`src/storage/git/`), configure the `git2` crate to interact with a bare Git repository (`git init --bare`). Implement direct object-database (ODB) writes via `git_blob_create_from_buffer` for raw document streams and construct standard tree and commit objects directly within the ODB backend targeting a dedicated specifications branch (`refs/heads/specs` or `specs/<document_slug>.md`). This bypasses the Git working tree and index (`.git/index`), completely eliminating index lock collisions (`.git/index.lock`) during concurrent document ingestion calls.
  2. **Standard Branch Commit Reachability (Eliminating Loose Refs):** Committing raw document blobs into a standard Git commit-tree graph naturally ensures 100% reachability for all historical blobs under standard Git garbage collection semantics. This completely eliminates the need for bespoke loose reference files (`refs/tks/blobs/*`), prevents filesystem `.lock` file collisions during concurrent uploads, avoids `git fsck` warnings ("points to a blob, not a commit or tree"), and removes the fragile repository configuration hack (`gc.pruneExpire never`). Furthermore, developers can inspect document revision history using standard Git CLI tools (`git log`, `git diff`).
  3. **Dedicated Storage Volume Binding:** In `docker-compose.yml` and container deployment manifests, configure a dedicated, persistent filesystem volume mount for the bare Git repository co-located with the PostgreSQL persistent data volume, ensuring Git blob storage persists across container recreation and redeployment cycles.
  4. **Point-in-Time Backup & Restore Synchronization:** Author operational shell scripts in `scripts/` to orchestrate coordinated point-in-time backups: trigger a PostgreSQL WAL checkpoint / `pg_dump` and execute a simultaneous snapshot of the bare Git repository object database (`objects/` and `refs/`), preventing referential drift between PostgreSQL blob hash columns and underlying Git blobs.

---

## TB-2

* **ID:** TB-2
* **Title:** Mechanical CommonMark AST Parsing, RFC 2119 Lexical Extraction Benchmark, and Zero-Copy Byte Slicing (`pulldown-cmark`)
* **Origin:** LD-1, iteration 2; LD-2, iteration 3
* **Status:** Active
* **Description/Tactical Details:**
  1. **Streaming CommonMark Parser Implementation:** In the document ingestion module (`src/ingest/parser.rs`), implement a streaming Markdown parser using `pulldown-cmark`. Traverse the CommonMark event stream to segment ingested documents along structural boundaries (headings H1–H4, tables, blockquotes, and bulleted lists).
  2. **Zero-Token Deterministic Byte Span Capture:** For each structural block event, extract exact 0-based byte offsets (`byte_start`, `byte_end`) directly from the source stream offset tracking provided by `pulldown-cmark`'s `OffsetIter`. Pointers are pinned with 100% precision without LLM inference cost.
  3. **Zero-Copy Byte Slicing & UTF-8 Safety:** Standardize span extraction in the Rust service layer on raw byte slicing against the source buffer (`&source_bytes[byte_start..byte_end]`) prior to UTF-8 validation (`std::str::from_utf8`), eliminating string slicing panics on multi-byte UTF-8 sequences (em-dashes, curly quotes, math symbols).
  4. **Rule-Based RFC 2119 & Entity Lexical Matcher:** Implement a deterministic scanner matching RFC 2119 keywords (`MUST`, `MUST NOT`, `REQUIRED`, `SHALL`, `SHALL NOT`, `SHOULD`, `SHOULD NOT`, `RECOMMENDED`, `MAY`, `OPTIONAL`) and project entity tags (`REQ-*`, `INV-*`, `DR-*`, `C-*`, `TB-*`). Structural chunks containing these patterns are tagged as candidate requirements.
  5. **Benchmarking & Token Reduction Calibration:** Benchmark parsing throughput, memory footprint, and token reduction across representative PRDs and specification documents of varying sizes (3,000 to 10,000 words). Verify that mechanical extraction handles $\ge 80\%$ of structural decomposition before any external LLM invocation, measuring output token savings.

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
* **Title:** Identity Provisioning CLI Subcommand (`tks identity create`) and Local Development Migration Seed
* **Origin:** LD-10, iteration 3
* **Status:** Active
* **Description/Tactical Details:**
  1. **Administrative CLI Command:** Implement an identity management subcommand in the `tks` binary:
     `tks identity create --name <name> --role <HUMAN|AGENT> [--token <custom_token>]`
     The command generates a cryptographically secure random token (or uses the supplied token), writes the identity record to PostgreSQL table `agent_identities`, and prints the active bearer token to stdout for developer use.
  2. **Development Environment Seed:** In database migrations (`refinery` / SQL migrations), include an automatic development seed that executes when `ENVIRONMENT=development` (or during initial local test database bootstrapping). The seed provisions a well-known development token (`tks_dev_token`) with role `HUMAN` and `is_active = true`, ensuring that fresh local environments can immediately run integration tests, ingest initial documents, and complete Gate 1 dogfooding without manual SQL seeding.
  3. **Identity Revocation Subcommand:** Implement `tks identity revoke <agent_id>` to support quick security containment by setting `is_active = false` on targeted agent instances.
