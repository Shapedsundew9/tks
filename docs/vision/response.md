# Lead Developer Sub-Agent Review (Iteration 3)

The technical alignment loop for Iteration 3 has concluded. As Lead Developer Sub-Agent, I evaluated [architecture.md](file:///workspaces/tks/docs/vision/architecture.md), [vision.md](file:///workspaces/tks/docs/vision/vision.md), [strategic-planning-backlog.md](file:///workspaces/tks/docs/vision/strategic-planning-backlog.md), and [technical-backlog.md](file:///workspaces/tks/docs/vision/technical-backlog.md) from the stance of an implementation pragmatist and master builder committed to the system's operational success.

The architectural direction remains strong, but several critical operational and state-model hazards have been identified that would impede Phase 1 construction, corrupt graph queries, or cause runtime panics.

A total of **10 ranked findings** (1 Blocker, 7 Major, 2 Minor) and **3 whole-component simplifications** have been documented in [`/workspaces/tks/docs/vision/response.md`](file:///workspaces/tks/docs/vision/response.md).

---

## Executive Summary of Findings

### Critical & Major Findings (`LD-1` to `LD-8`)

1. **[`LD-1`](file:///workspaces/tks/docs/vision/response.md#ld-1) (Blocker — Missing Activity/Lifecycle State on `graph_edges`):**
   Invariant INV-2 forbids deleting edges, yet `graph_edges` contains no `lifecycle_state` or `is_active` column. Reversion or supersession cannot deactivate an edge without in-place mutation or physical deletion, and recursive CTE traversals will traverse obsolete relationships.
   *Fix:* Add `lifecycle_state` (`DRAFT`, `ACTIVE`, `SUPERSEDED`, `REVERTED`) and a partial index for active edges to `graph_edges`.

2. **[`LD-2`](file:///workspaces/tks/docs/vision/response.md#ld-2) (Major — Character vs. Byte Offset Impedance Mismatch):**
   `source_spans` names fields `char_start` and `char_end`, populated by `pulldown-cmark`'s byte offsets. Rust string slicing `&str[start..end]` operates on byte offsets; indexing on character codepoints over multi-byte UTF-8 text panics at runtime.
   *Fix:* Standardize strictly on 0-based byte offsets (`byte_start`, `byte_end`) across PostgreSQL, Rust models, and MCP contracts.

3. **[`LD-3`](file:///workspaces/tks/docs/vision/response.md#ld-3) (Major — Draft Topology vs. Quarantine Staging Queue Schizophrenia):**
   Decision D-16 added `DRAFT` state to `graph_nodes` to allow agents to build draft subtrees, but the architecture retained `staging_queue` JSON blobs alongside it. Candidate nodes trapped in `staging_queue` cannot have hierarchical edges or be queried via CTEs.
   *Fix:* Consolidate candidate nodes directly into `graph_nodes` and `graph_edges` with `lifecycle_state = 'DRAFT'`.

4. **[`LD-4`](file:///workspaces/tks/docs/vision/response.md#ld-4) (Major — Invariant INV-1 Deferred Trigger Failures):**
   INV-1's deferred constraint trigger requiring paths to active requirements will abort valid draft subtree creation and fail automated rollback sweeps when parent requirements transition to `SUPERSEDED`.
   *Fix:* Scope INV-1 strictly to active execution tasks (`lifecycle_state = 'ACTIVE'`), exempt non-active states, and enforce path constraints during draft activation.

5. **[`LD-5`](file:///workspaces/tks/docs/vision/response.md#ld-5) (Major — Missing Ephemeral Draft Revision Storage):**
   Compaction on approval writes a `draft_evolution_summary` JSONB, but intermediate draft edits have no defined storage table.
   *Fix:* Formally introduce an ephemeral `draft_revisions` table that logs intermediate patches and is squashed and purged upon draft approval.

6. **[`LD-6`](file:///workspaces/tks/docs/vision/response.md#ld-6) (Major — Out-of-Order Overwrite Race in Embedding Worker):**
   `embedding_queue` lacks deduplication on `node_id`, and `node_embeddings` lacks content hash/version checks, allowing slow out-of-band worker jobs to overwrite newer embeddings with stale data.
   *Fix:* Add `content_hash` and optimistic versioning checks to `node_embeddings` and deduplicate `embedding_queue`.

7. **[`LD-7`](file:///workspaces/tks/docs/vision/response.md#ld-7) (Major — All-or-Nothing Staging Approval):**
   `POST /api/v1/staging/approve` and `tks staging approve` only approve entire jobs, with no mechanism to reject false positives or selectively approve candidate nodes.
   *Fix:* Support selective approval (`approved_node_ids`) and a rejection endpoint (`POST /api/v1/staging/reject`).

8. **[`LD-8`](file:///workspaces/tks/docs/vision/response.md#ld-8) (Major — Unspecified Context Envelope Vector-Topological Budgeting):**
   `get_context_envelope` does not define how the 40-node budget balances recursive CTE traversal with vector search, risking semantic pollution from unrelated global vector matches.
   *Fix:* Guarantee 30 of 40 nodes strictly for topological hierarchy; use vector search only for secondary ranking or cross-cutting constraints.

### Minor Findings (`LD-9`, `LD-10`)

* **[`LD-9`](file:///workspaces/tks/docs/vision/response.md#ld-9) (Minor):** CLI stdio proxy (`tks mcp-stdio`) crashes abruptly if `tks serve` is offline, breaking agent harnesses. Implement resilient connection retry and a structured JSON-RPC error response.
* **[`LD-10`](file:///workspaces/tks/docs/vision/response.md#ld-10) (Minor):** Missing CLI command (`tks identity create`) and migration development seed for provisioning initial identities during Phase 1 dogfooding.

---

## Whole-Component Simplifications (`LD-11` to `LD-13`)

* **[`LD-11`](file:///workspaces/tks/docs/vision/response.md#ld-11): Consolidate `staging_queue` into `graph_nodes` & Retire TB-3:**
  Eliminates the separate `staging_queue` table and retires [technical-backlog.md](file:///workspaces/tks/docs/vision/technical-backlog.md) item TB-3 (Serde `StagedPayload` versioning), treating draft requirements as first-class graph entities from inception.
* **[`LD-12`](file:///workspaces/tks/docs/vision/response.md#ld-12): Replace `refs/tks/blobs/*` with Standard Git Branch Commits:**
  Eliminates loose blob reference clutter, `git fsck` warnings, and ref-lock collisions by committing documents directly into a dedicated Git specifications branch (`refs/heads/specs`).
* **[`LD-13`](file:///workspaces/tks/docs/vision/response.md#ld-13): Replace Leaf Advisory Locks with Native PostgreSQL `FOR UPDATE`:**
  Eliminates 32-bit `hashtext(node_id)` birthday collisions on single-node attribute edits by adopting native, collision-free PostgreSQL row-level locking.
