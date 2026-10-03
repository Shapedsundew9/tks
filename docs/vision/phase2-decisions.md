# Phase 2 Implementation Decision Record: Bounded Agent Mutation & Per-Node Governance

## 1. Decision Ledger

| Decision ID | Work Package | Title | Category | Target Upstream Document | Status |
| :--- | :--- | :--- | :--- | :--- | :--- |
| DEC-2.1 | WP-2.1 | DAG Cycle Detection Traversal Across Active and Candidate Edges | Specification Gap | architecture.md §5.2 | Implemented |
| DEC-2.2 | WP-2.1 | Higher-Ranked BoxFuture Transaction Wrapper for Structural Mutations | Technical Trade-off | architecture.md §5.2 | Implemented |
| DEC-2.3 | WP-2.2 | Upward Lineage and Active Ancestor Edge Mirroring for Normative Draft Proposals | Specification Gap | architecture.md §5.1 | Implemented |
| DEC-2.4 | WP-2.2 | Dual Client Signatures (&mut deadpool_postgres::Client and &mut tokio_postgres::Client) for Mutation Pathways | API/Contract Elaboration | architecture.md §5.2 | Implemented |
| DEC-2.5 | WP-2.2 | Dedicated ERR_GOVERNANCE_LOCKED Error Code and Variant for Safety-Critical Locked Nodes | Specification Gap | architecture.md §5.1 | Implemented |
| DEC-2.6 | WP-2.3 | Structured MutationError::ConfirmationRequired(`Box<RevertPreview>`) and Error Code Mapping for Rollback Safety Aborts | Specification Gap | architecture.md §5.1 | Implemented |
| DEC-2.7 | WP-2.3 | Selective State Preservation vs Edge Reversion in Administrative Rollback | Specification Gap | architecture.md §5.1 | Implemented |
| DEC-2.8 | WP-2.3 | Dedicated ERR_DEPENDENCY_INACTIVE Error and Re-Parenting Operational Unblocking in Reverification Interface | API/Contract Elaboration | architecture.md §5.1 | Implemented |

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

### DEC-2.6: Structured MutationError::ConfirmationRequired(`Box<RevertPreview>`) and Error Code Mapping for Rollback Safety Aborts

* **Work Package:** WP-2.3
* **Category:** Specification Gap
* **Context & Problem:** Decision D-80 and the WP-2.3 specification require that when `dry_run = false`, if cross-agent dependent tasks exist and `force != true`, the engine halts with `ERR_CONFIRMATION_REQUIRED` returning the preview payload. The method signature returns `Result<RevertExecutionResult, MutationError>`. If the abort payload is serialized into a string message or returned as an `Ok` variant, downstream API and MCP layers lose structured blast-radius type information or violate standardized error taxonomy.
* **Options Considered:**
  * *Option A:* Return `Ok(RevertExecutionResult::ConfirmationRequired(RevertPreview))`. Pros: Encapsulated within `Ok`. Cons: Violates the specification mandate that the operation aborts with error code `ERR_CONFIRMATION_REQUIRED`, creating impedance mismatch with REST 409 Conflict / MCP JSON-RPC error protocols.
  * *Option B:* Introduce `MutationError::ConfirmationRequired(Box<RevertPreview>)` with error code `"ERR_CONFIRMATION_REQUIRED"` and typed accessor `MutationError::preview(&self) -> Option<&RevertPreview>`. Pros: Preserves error semantics, satisfies contract error taxonomy, provides zero-cost typed access to blast-radius preview without JSON re-parsing, and keeps `MutationError` enum size compact via heap indirection. Cons: Slightly expands `MutationError` variants.
* **Decision Taken & Rationale:** Adopted Option B. Boxing `RevertPreview` within `MutationError::ConfirmationRequired` provides clean typed error propagation to Axum and MCP layers while satisfying clippy error size lints and exact error contract compliance.
* **Upstream Impact & Target Document:** `docs/vision/architecture.md` §5.1 Rollback Cascade Mechanics and §6 Interfaces & Contracts should document `MutationError::ConfirmationRequired` and error code `ERR_CONFIRMATION_REQUIRED`.
* **Status:** Implemented

### DEC-2.7: Selective State Preservation vs Edge Reversion in Administrative Rollback

* **Work Package:** WP-2.3
* **Category:** Specification Gap
* **Context & Problem:** Decision D-73 and WP-2.3 specify administrative rollback via `revert_mutations`, transitioning live nodes to `SUPERSEDED` and edges to `REVERTED`. However, mutations on existing nodes (such as Pathway 2 `TASK_STATUS_UPDATE` or `LEAF_ATTRIBUTE_UPDATE`) modify attributes in place on nodes that were created in earlier batches. If all targeted nodes and their outgoing edges were indiscriminately marked `SUPERSEDED` and `REVERTED`, reverting an in-place task status update would erroneously sever the task's structural parent edge to its requirement root, violating Invariant INV-1 and breaking reverification loops.
* **Options Considered:**
  * *Option A:* Treat all targeted nodes identically by transitioning them to `SUPERSEDED` and marking all outgoing edges `REVERTED`. Pros: Uniform handling across all audit event types. Cons: Destroys active tasks when only a leaf attribute or status update was meant to be reverted; breaks Invariant INV-1 traceability for intact nodes.
  * *Option B:* Differentiate entity creation events (`TASK_ELABORATED`, `NODE_INSERTED`, `STAGING_APPROVED`) from leaf/status update events (`TASK_STATUS_UPDATE`, `LEAF_ATTRIBUTE_UPDATE`). For creation events, transition the node to `SUPERSEDED` and its outgoing edges to `REVERTED`. For leaf/status update events, restore prior attributes/status from event `delta`/`snapshot`, preserve the node's `ACTIVE` state, and keep its structural edges intact while cascading dependent child tasks to `NEEDS_REVERIFICATION`. Pros: Correctly preserves graph topology, faithfully restores prior attribute states, and allows dependent child tasks to be successfully reverified against the restored active parent. Cons: Requires event-type branch logic in the rollback engine.
* **Decision Taken & Rationale:** Adopted Option B. In `src/storage/rollback.rs`, entity creation events transition to `SUPERSEDED` and revert outgoing edges, while status and leaf attribute rollbacks restore previous state in-place without severing structural edges. This preserves Invariant INV-1, guarantees non-destructive reversible compensation, and enables the invalidation-and-reverification loop to function correctly.
* **Upstream Impact & Target Document:** `docs/vision/architecture.md` §5.1 Rollback Cascade Mechanics should document that in-place leaf/status mutation rollbacks restore attributes without severing structural edges or superseding nodes.
* **Status:** Implemented

### DEC-2.8: Dedicated ERR_DEPENDENCY_INACTIVE Error and Re-Parenting Operational Unblocking in Reverification Interface

* **Work Package:** WP-2.3
* **Category:** API/Contract Elaboration
* **Context & Problem:** Decision D-76 and `architecture.md` §6 specify that `reverify_node` validates that all direct upstream dependencies are in `ACTIVE` state (INV-1), returning error `ERR_DEPENDENCY_INACTIVE` if inactive. When a parent node creation has been reverted (transitioned to `SUPERSEDED`), child tasks that cascaded to `NEEDS_REVERIFICATION` cannot be reverified against the superseded parent. Without an explicit re-parenting mechanism, degraded child tasks would remain permanently stuck in operational deadlock unless manual SQL edge updates were performed.
* **Options Considered:**
  * *Option A:* Require a multi-step external workflow where callers must invoke a separate edge creation tool/endpoint before calling `reverify_node`. Pros: Minimal implementation in `reverify_node`. Cons: Requires non-atomic two-phase execution, risks leaving orphan nodes in invalid intermediate states, and creates circular dependencies between tools.
  * *Option B:* Support an optional re-parenting parameter in `updated_attributes` (accepting `reparent_to` or `parent_id` UUID or `node_key`), atomically updating active upward edges to the new active parent under `pg_advisory_xact_lock` before verifying upstream dependencies and promoting the node to `ACTIVE`. Pros: Enables atomic operational unblocking in a single call, verifies acyclicity via `assert_no_dag_cycle`, validates unbroken path to active requirement (INV-1), and returns dedicated `ERR_DEPENDENCY_INACTIVE` when upstream parents are inactive. Cons: Adds edge re-anchoring logic within the reverification transaction.
* **Decision Taken & Rationale:** Adopted Option B. In `src/storage/reverify.rs`, `reverify_node` supports optional `reparent_to` / `parent_id` in `updated_attributes`, asserting cycle prevention and atomically establishing active edges to the new parent before promoting the node to `ACTIVE`. This provides complete operational recovery for degraded subtrees following parent supersession, eliminating operational deadlocks while strictly preserving Invariant INV-1.
* **Upstream Impact & Target Document:** `docs/vision/architecture.md` §5.1 Reverification Interface and §6 Interfaces & Contracts should document `reparent_to` / `parent_id` support in `updated_attributes` and `ERR_DEPENDENCY_INACTIVE`.
* **Status:** Implemented
