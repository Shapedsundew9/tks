# Phase 2 Implementation Decision Record: Bounded Agent Mutation & Per-Node Governance

## 1. Decision Ledger

| Decision ID | Work Package | Title | Category | Target Upstream Document | Status |
| :--- | :--- | :--- | :--- | :--- | :--- |
| DEC-2.1 | WP-2.1 | DAG Cycle Detection Traversal Across Active and Candidate Edges | Specification Gap | architecture.md §5.2 | Implemented |
| DEC-2.2 | WP-2.1 | Higher-Ranked BoxFuture Transaction Wrapper for Structural Mutations | Technical Trade-off | architecture.md §5.2 | Implemented |
| DEC-2.3 | WP-2.2 | Upward Lineage and Active Ancestor Edge Mirroring for Normative Draft Proposals | Specification Gap | architecture.md §5.1 | Implemented |
| DEC-2.4 | WP-2.2 | Dual Client Signatures (&mut deadpool_postgres::Client and &mut tokio_postgres::Client) for Mutation Pathways | API/Contract Elaboration | architecture.md §5.2 | Implemented |
| DEC-2.5 | WP-2.2 | Dedicated ERR_GOVERNANCE_LOCKED Error Code and Variant for Safety-Critical Locked Nodes | Specification Gap | architecture.md §5.1 | Implemented |

---

## 2. Decision Entries

### DEC-2.1: DAG Cycle Detection Traversal Across Active and Candidate Edges

* **Work Package:** WP-2.1
* **Category:** Specification Gap
* **Context & Problem:** Decision D-8 and Decision D-20 specify in-database cycle prevention via recursive CTEs traversing upward relationships (`FULFILLS`, `CONSTRAINED_BY`, `DERIVED_FROM`), but do not explicitly specify whether candidate draft edges (`lifecycle_state = 'DRAFT'`) must be included in the cycle traversal. WP-2.1 verification criteria mandate: "Insertion of an edge creating a directed cycle between active or candidate nodes aborts with `MutationError::CycleDetected` and leaves graph topology unaltered." If the CTE traverses only `ACTIVE` edges, cyclic structures could be authored across pending candidate subtrees during multi-agent elaboration or document staging, corrupting DAG topology upon batch promotion.
* **Options Considered:**
  * *Option A:* Traverse only `lifecycle_state = 'ACTIVE'` edges in `check_dag_cycle` and defer draft cycle validation to batch staging approval. Pros: Marginally smaller CTE traversal scope on large graphs. Cons: Fails WP-2.1 verification criteria; allows external agents to author self-referential or cyclic draft trees; delays cycle detection feedback until approval.
  * *Option B:* Traverse both `ACTIVE` and `DRAFT` edges (`lifecycle_state IN ('ACTIVE', 'DRAFT')`) in `check_dag_cycle`, excluding historical `SUPERSEDED` and `REVERTED` edges. Pros: Immediately catches and rejects cyclic edge proposals during authoring for both candidate and active entities; guarantees topological integrity before promotion; satisfies WP-2.1 proof criteria. Cons: Traverses uncommitted candidate edges alongside active topology.
* **Decision Taken & Rationale:** Adopted Option B. In `src/storage/mutation.rs`, `check_dag_cycle` filters `e.lifecycle_state IN ('ACTIVE', 'DRAFT')`. This prevents agents and decomposition workers from constructing cyclic candidate subtrees while ensuring historical superseded/reverted edges do not trigger false positive cycle rejections.
* **Upstream Impact & Target Document:** `docs/vision/architecture.md` §5.2 and Decision D-8 should be updated to clarify that DAG cycle detection encompasses both active and candidate draft edges (`lifecycle_state IN ('ACTIVE', 'DRAFT')`).
* **Status:** Implemented

### DEC-2.2: Higher-Ranked BoxFuture Transaction Wrapper for Structural Mutations

* **Work Package:** WP-2.1
* **Category:** Technical Trade-off
* **Context & Problem:** Implementing `execute_structural_mutation` on `StorageRepo` requires running a caller-supplied async closure within a transaction holding the global structural mutation advisory lock (`pg_advisory_xact_lock(hashtext('tks_structural_mutation'))`), with automatic commit on success and rollback on failure. In Rust edition 2024, closures accepting `&mut StructuralMutationTx` across async boundaries encounter lifetime invariance conflicts if parameterized by an unconstrained generic `Fut: Future`.
* **Options Considered:**
  * *Option A:* Require callers to manually manage client acquisition, transaction start, lock acquisition, and commit/rollback without an `execute_structural_mutation` repository method. Pros: Avoids higher-ranked closure signatures. Cons: Violates WP-2.1 task 3 and target artifacts specification; duplicates boilerplate across handlers; risks leaking locks or omitting transaction rollback.
  * *Option B:* Define `execute_structural_mutation` using a higher-ranked closure signature returning a boxed future (`for<'tx> FnOnce(&'tx mut StructuralMutationTx<'_>) -> BoxFuture<'tx, Result<T, MutationError>>`), paired with the direct helper `begin_structural_mutation` on `StorageRepo`. Pros: Cleanly decouples the borrow lifetime `'tx` of the transaction wrapper from the connection lifetime; guarantees lock release and automatic rollback; provides ergonomic transaction orchestration with zero unsafe code. Cons: Incurs a single heap allocation (`Box::pin`) for the closure future during structural mutation orchestration.
* **Decision Taken & Rationale:** Adopted Option B. The minimal heap allocation of `Box::pin` during structural mutations (which already incur database roundtrips and global advisory locking) is negligible, while providing compile-time lifetime safety and ergonomic transaction management.
* **Upstream Impact & Target Document:** `docs/vision/architecture.md` §5.2 Concurrency Model should document the higher-ranked `BoxFuture` closure contract for `execute_structural_mutation`.
* **Status:** Implemented

### DEC-2.3: Upward Lineage and Active Ancestor Edge Mirroring for Normative Draft Proposals

* **Work Package:** WP-2.2
* **Category:** Specification Gap
* **Context & Problem:** Decision D-57 and Invariant INV-2 mandate that normative edits to active `REQUIREMENT` and `SPECIFICATION` nodes must be intercepted, creating candidate entities in `lifecycle_state = 'DRAFT'` with `attributes->'replaces_node_id' = target_id`. However, the specification does not explicitly detail what upward edges must be created for the new candidate draft node so that it both satisfies Invariant INV-1 ancestor validation and preserves the target node's architectural lineage without premature active edge collisions.
* **Options Considered:**
  * *Option A:* Create only a single `DERIVED_FROM` edge from the candidate draft node directly to the target active node (`draft_id -DERIVED_FROM-> target_id`). Pros: Simple; guarantees child points to an active requirement root. Cons: Does not mirror existing upward parent edges if target is a `SPECIFICATION` that fulfills an active requirement or constraint, losing parent context if target is superseded.
  * *Option B:* Mirror all of the target node's active upward edges (`FULFILLS`, `CONSTRAINED_BY`, `DERIVED_FROM`) with `lifecycle_state = 'DRAFT'`, and additionally establish a `DERIVED_FROM` candidate edge from draft to target (`draft_id -DERIVED_FROM-> target_id`). Pros: Preserves both the target node's structural lineage to governing requirements and an explicit edge linking the draft to the specific node being superseded; guarantees full context envelope assembly for drafts; dual edges ensure clean batch promotion. Cons: Generates an extra draft edge in `graph_edges`.
* **Decision Taken & Rationale:** Adopted Option B. In `src/storage/mutation.rs::propose_normative_draft`, candidate drafts mirror active parent edges as `DRAFT` edges and add a `DERIVED_FROM` draft edge to the target node. This guarantees full ancestor path reachability (INV-1) and preserves lineage when the target node is superseded upon staging approval.
* **Upstream Impact & Target Document:** `docs/vision/architecture.md` §5.1 Disambiguated Mutation Pathways should record that candidate normative draft proposals mirror target upward edges in `DRAFT` state and establish a `DERIVED_FROM` draft edge to the target node.
* **Status:** Implemented

### DEC-2.4: Dual Client Signatures (&mut deadpool_postgres::Client and &mut tokio_postgres::Client) for Mutation Pathways

* **Work Package:** WP-2.2
* **Category:** API/Contract Elaboration
* **Context & Problem:** The WP-2.2 contract specifies public signatures taking `&mut deadpool_postgres::Client` (`elaborate_task`, `update_task_status`, `propose_normative_draft`, `mutate_draft_entity`). However, unit and lower-level integration tests or developer scripts frequently hold raw `tokio_postgres::Client` instances (from `db::connect`) rather than checking out pooled connections. Requiring only pooled clients in unit tests causes artificial test friction or requires re-pooling.
* **Options Considered:**
  * *Option A:* Expose only `&mut deadpool_postgres::Client` methods. Pros: Exact 1-to-1 match with the single contract line. Cons: Tests with direct `tokio_postgres::Client` cannot call mutation primitives without constructing a dummy pool.
  * *Option B:* Expose the specified `&mut deadpool_postgres::Client` signatures, backed by generic companion `&mut Client` functions (`elaborate_task_client`, `update_task_status_client`, etc.) and `StorageRepo` convenience methods. Pros: Satisfies 100% of the specified API contract, allows seamless pool and raw client usage across production gateway and test harnesses, and provides ergonomic repo methods. Cons: Adds thin companion forwarding functions.
* **Decision Taken & Rationale:** Adopted Option B. This provides complete compliance with the WP-2.2 interface contract while maximizing developer ergonomics and testing flexibility.
* **Upstream Impact & Target Document:** `docs/vision/architecture.md` §5.2 Concurrency Model and §6 Interfaces & Contracts should document both pooled and raw client interfaces.
* **Status:** Implemented

### DEC-2.5: Dedicated ERR_GOVERNANCE_LOCKED Error Code and Variant for Safety-Critical Locked Nodes

* **Work Package:** WP-2.2
* **Category:** Specification Gap
* **Context & Problem:** Invariant INV-5 and the WP-2.2 specification state: "`LOCKED` rejects all mutations with `ERR_GOVERNANCE_LOCKED`; `HUMAN_REVIEW_REQUIRED` forces normative proposals to `DRAFT`; `AUTONOMOUS_ELABORATION` allows direct `TASK` activation." The initial `MutationError` enum had only `GovernanceRejected(String)` mapping to `"ERR_GOVERNANCE_REJECTED"`. Using a generic rejection error obscures safety-critical locked node rejections from external callers and MCP clients.
* **Options Considered:**
  * *Option A:* Keep only `GovernanceRejected` and prefix the message string with `"ERR_GOVERNANCE_LOCKED"`. Pros: No changes to enum variants. Cons: Fragile string parsing; cannot programmatically distinguish policy violations from lock violations.
  * *Option B:* Introduce a first-class `MutationError::GovernanceLocked(String)` variant with error code `"ERR_GOVERNANCE_LOCKED"`, keeping `MutationError::GovernanceRejected(String)` with code `"ERR_GOVERNANCE_REJECTED"` for other policy mismatches. Pros: Strongly typed, unambiguous error taxonomy, directly satisfies the specification requirement. Cons: Adds one variant to `MutationError`.
* **Decision Taken & Rationale:** Adopted Option B. First-class typing ensures callers and MCP gateways can deterministically identify locked nodes without error string pattern matching.
* **Upstream Impact & Target Document:** `docs/vision/architecture.md` §5.1 Governance policy and §6 Interfaces & Contracts should document `ERR_GOVERNANCE_LOCKED`.
* **Status:** Implemented
