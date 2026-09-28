# Technical Implementation Backlog

This document captures vital component-level designs, library evaluation tasks, sprint-level tactical choices, and downstream implementation details deferred from architectural review.

---

## TB-1

* **ID:** TB-1
* **Title:** Bare Git Repository Direct ODB Ingestion, Permanent Reference Namespaces, and Volume Backup Synchronization
* **Origin:** LD-8, iteration 1; LD-5, iteration 2
* **Description/Tactical Details:**
  1. **Low-Level ODB Direct Writes:** In the Git document adapter module (`src/storage/git/`), configure the `git2` crate to interact exclusively with a bare Git repository (`git init --bare`). Implement direct object-database (ODB) writes via `git_blob_create_from_buffer` for raw document streams and construct tree/commit objects directly within the ODB backend. This bypasses the Git working tree and index (`.git/index`), completely eliminating index lock collisions (`.git/index.lock`) during concurrent document ingestion calls.
  2. **Dedicated Storage Volume Binding:** In `docker-compose.yml` and container deployment manifests, configure a dedicated, persistent filesystem volume mount for the bare Git repository co-located with the PostgreSQL persistent data volume, ensuring Git blob storage persists across container recreation and redeployment cycles.
  3. **Point-in-Time Backup & Restore Synchronization:** Author operational shell scripts in `scripts/` to orchestrate coordinated point-in-time backups: trigger a PostgreSQL WAL checkpoint / `pg_dump` and execute a simultaneous snapshot of the bare Git repository object database (`objects/` and `refs/`), preventing referential drift between PostgreSQL blob hash columns and underlying Git blobs.
  4. **Permanent Reference Namespace (`refs/tks/blobs/*`):** Whenever a document blob is written to the bare Git ODB via `git_blob_create_from_buffer`, the storage adapter must atomically create a permanent Git reference under a dedicated namespace: `refs/tks/blobs/<blob_hash>`. This ensures the blob is explicitly reachable from a named reference, preventing Git reachability traversals from categorizing it as an unreferenced loose object.
  5. **Automated Pruning Prevention Configuration:** During bare repository initialization or provisioning scripts, configure repository garbage collection settings explicitly: execute `git config gc.pruneExpire never` and `git config gc.auto 0` on the bare Git repository volume, preventing automated maintenance jobs (`git gc`, `git prune`) from silently wiping out unreferenced or stale blobs and preserving Invariant INV-4.

---

## TB-2

* **ID:** TB-2
* **Title:** Mechanical CommonMark AST Parsing & RFC 2119 Lexical Extraction Benchmark (`pulldown-cmark`)
* **Origin:** LD-1, iteration 2
* **Description/Tactical Details:**
  1. **Streaming CommonMark Parser Implementation:** In the document ingestion module (`src/ingest/parser.rs`), implement a streaming Markdown parser using `pulldown-cmark`. Traverse the CommonMark event stream to segment ingested documents along structural boundaries (headings H1–H4, tables, blockquotes, and bulleted lists).
  2. **Zero-Token Deterministic Span Capture:** For each structural block event, extract exact byte and character offsets (`char_start`, `char_end`) directly from the source stream offset tracking provided by `pulldown-cmark`. Pointers are pinned with 100% precision without LLM inference cost.
  3. **Rule-Based RFC 2119 & Entity Lexical Matcher:** Implement a deterministic scanner matching RFC 2119 keywords (`MUST`, `MUST NOT`, `REQUIRED`, `SHALL`, `SHALL NOT`, `SHOULD`, `SHOULD NOT`, `RECOMMENDED`, `MAY`, `OPTIONAL`) and project entity tags (`REQ-*`, `INV-*`, `DR-*`, `C-*`, `TB-*`). Structural chunks containing these patterns are tagged as candidate requirements.
  4. **Benchmarking & Token Reduction Calibration:** Benchmark parsing throughput, memory footprint, and token reduction across representative PRDs and specification documents of varying sizes (3,000 to 10,000 words). Verify that mechanical extraction handles $\ge 80\%$ of structural decomposition before any external LLM invocation, measuring output token savings.

---

## TB-3

* **ID:** TB-3
* **Title:** Schema Versioning and StagedPayload Enum Mapping for Staging Queue
* **Origin:** LD-8, iteration 2
* **Description/Tactical Details:**
  1. **PostgreSQL Column Versioning:** Add a `schema_version INT NOT NULL DEFAULT 1` column to the `staging_queue` table definition in PostgreSQL migrations (`refinery` / SQL migrations).
  2. **Strongly Typed Rust Enum Mapping:** Define a versioned Rust enum `StagedPayload` in the domain crate (`src/domain/staging.rs`) using Serde tagged serialization (e.g., `#[serde(tag = "schema_version")]` with variants `V1(StagedPayloadV1)`, `V2(StagedPayloadV2)`).
  3. **Migration and Deserialization Safeguards:** Enforce that all write operations to `staging_queue` serialize payloads matching the current active schema version. In the staging approval handler (`POST /api/v1/staging/approve`), implement graceful multi-version deserialization ensuring that payloads staged under earlier schema versions remain decodable and actionable without operational failure.
