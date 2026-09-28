# Architectural Rebuttals (Iteration 1)

This document records formal architectural rebuttals to Lead Developer critique findings from `response.md` that compromise structural integrity, violate core invariants, or conflict with the governing vision.

## Iteration 1 Rebuttals

### Rebuttal: LD-11

* **Finding Reference:** LD-11
* **Severity:** Major
* **Proposed Alternative from Lead Developer:** Consolidate the staging queue directly into `GraphTopo` by utilizing node lifecycle statuses (`status: PENDING_STAGING`, `PENDING_REVIEW`, `ACTIVE`, `SUPERSEDED`, `ARCHIVED`). Write draft nodes and edges directly into `graph_nodes` and `graph_edges` with pending statuses, exclude them from context envelopes by default (`WHERE status = 'ACTIVE'`), and execute approval as an in-place status transition (`UPDATE ... SET status = 'ACTIVE'`).
* **Architectural Disposition:** Respectfully Rebutted.

#### First-Principles Architectural Rationale

While the motivation to reduce dual-schema translation code is acknowledged, eliminating `StagingTable` (`staging_queue`) and injecting unverified drafts directly into the primary graph tables introduces four fatal structural liabilities:

1. **Direct Violation of Invariant INV-2 (Prohibition of In-Place Updates):**
   Invariant INV-2 explicitly states: *"Destructive in-place updates (`UPDATE`, `DELETE`) on requirements, specifications, and topological edges must not occur. All state transitions must be recorded as discrete, reversible audit events enabling full point-in-time reconstruction."*
   Transitioning a candidate entity from pending to active via an in-place `UPDATE ... SET status = 'ACTIVE'` bypasses the event ledger materialization pipeline and violates the invariant requiring every authoritative graph entity to originate from an immutable, attributed audit event.

2. **Entity Identity and Primary Key Collision on Candidate Mutations to Existing Nodes:**
   When an external agent proposes a revision to an existing active requirement node (e.g., modifying the constraint text of `REQ-005` where `governance_policy = HUMAN_REVIEW_REQUIRED`), the candidate modification cannot be inserted directly into `graph_nodes` under the primary key identifier `REQ-005` without triggering a primary key uniqueness violation (`id = 'REQ-005'`). Overwriting the active row in-place destroys or conceals the authoritative specification from operational context queries while review is pending. If surrogate identifiers are assigned to candidate versions, existing topological edges referencing `REQ-005` become ambiguous or disjoint. A decoupled staging table storing candidate patch payloads (`target_node_id`, `mutation_type`, `proposed_payload`, `proposing_actor_id`) is structurally required to hold pending mutations for existing entities without mutating active state.

3. **Canonical Topology Pollution and Hot-Path Query Degradation (SLA-1):**
   Automated LLM decomposition across technical documents inherently generates noisy extractions, sub-atomic fragments, and invalid candidate entities that human supervisors reject during review. If all raw candidates and candidate edges are inserted directly into `graph_nodes` and `graph_edges`, rejected drafts must either be physically deleted (directly violating INV-2) or retained indefinitely as dead rows (`status = 'REJECTED'`). Over repeated ingestion iterations, accumulating rejected candidates bloats the core adjacency tables and indexes, degrading B-tree index density and slowing recursive CTE joins on the hot path, directly threatening the sub-50ms micro-reflex traversal target (SLA-1).

4. **Relational Contamination and Violation of Invariant INV-1:**
   Writing unapproved edges directly to `graph_edges` creates real topological paths before human authorization. If deferred foreign key constraints or cycle detection triggers operate over `graph_edges`, unverified drafts can participate in ancestor paths, causing subsequent valid mutations to establish dependencies on candidates that may ultimately be rejected.

5. **Violation of Governing Vision Boundary Contracts (vision.md §2, §3, §4):**
   The North Star vision and environmental boundary contracts explicitly isolate unverified external data from the authoritative substrate:
   * §2 Key Capabilities (2): mandates human review gates before structured requirement nodes are committed to verified lineage.
   * §3 System Topology: explicitly depicts the human review gate standing between the decomposition pipeline and the commit to `AuditTrail` / `GraphStore`.
   * §4 Information Flow: specifies that unverified drafts do not enter the live property graph until human review commits them.

#### Conclusion & Architectural Action

The architectural separation between `StagingTable` (`staging_queue`) and `GraphTopo` (`graph_nodes`, `graph_edges`) will be strictly maintained. `staging_queue` serves as the necessary transactional quarantine zone for unverified candidate extractions and agent mutation proposals pending human approval. Once approved, items are atomically committed to `audit_ledger` and materialized into `graph_nodes` with full provenance attribution. This decision is permanently recorded as Rejected decision D-14 in `architecture.md`.
