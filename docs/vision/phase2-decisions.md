# Phase 2 Implementation Decision Record: Bounded Agent Mutation & Per-Node Governance

## 1. Decision Ledger

| Decision ID | Work Package | Title | Category | Target Upstream Document | Status |
| :--- | :--- | :--- | :--- | :--- | :--- |
| DEC-2.1 | WP-2.1 | DAG Cycle Detection Traversal Across Active and Candidate Edges | Specification Gap | architecture.md §5.2 | Implemented |
| DEC-2.2 | WP-2.1 | Higher-Ranked BoxFuture Transaction Wrapper for Structural Mutations | Technical Trade-off | architecture.md §5.2 | Implemented |

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
