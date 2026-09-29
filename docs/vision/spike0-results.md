# Spike 0 Empirical Trial Report: Hypothesis H-1 Directional Validation

## 1. Executive Summary & Gating Decision

- **Trial Objective:** Empirically evaluate Hypothesis H-1 (Governed Requirement Topologies vs. Ad-Hoc Agentic Retrieval) by comparing constraint violation rates across 20 controlled synthetic coding tasks.
- **Evaluation Condition A (Multi-Tool Agentic Baseline):** External coding agent provided with local file context, AST symbol signatures, and flat search, operating without requirement graph topology.
- **Evaluation Condition B (Topological Context Envelope):** External coding agent provided with a graph-bounded topological context envelope (target task + depth-2 ancestor requirements + sibling `CONSTRAINED_BY` rules).
- **Total Architectural Rubric Rules Evaluated:** 40
- **Condition A Violations:** 37 / 40 (92.5% violation rate)
- **Condition B Violations:** 6 / 40 (15.0% violation rate)
- **Constraint Violation Reduction:** **83.8%** (Formula: `(V_A - V_B) / V_A * 100%`)
- **CAL-H1 Directional Threshold:** $\ge 30.0$% (Phase 0 greenlight gate; full Phase 2 target is $\ge 40.0$%)
- **Gating Outcome:** **PASS: GREENLIGHT FOR PHASE 1 CONSTRUCTION (CAL-H1 Directional Validation Confirmed)**

## 2. Cross-Cutting Invariant Breakdown

| Invariant Key | Invariant Name / Focus | Rules Checked | Condition A Violations | Condition B Violations | Invariant Reduction |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `INV-AUDIT-05` | Non-Local Architectural Contract | 5 | 3 | 0 | 100.0% |
| `INV-AUTH-02` | Non-Local Architectural Contract | 8 | 8 | 0 | 100.0% |
| `INV-CONCUR-06` | Non-Local Architectural Contract | 3 | 3 | 0 | 100.0% |
| `INV-ERR-03` | Non-Local Architectural Contract | 8 | 8 | 0 | 100.0% |
| `INV-SEC-08` | Non-Local Architectural Contract | 5 | 5 | 4 | 20.0% |
| `INV-SPAN-07` | Non-Local Architectural Contract | 2 | 1 | 1 | 0.0% |
| `INV-STATE-04` | Non-Local Architectural Contract | 4 | 4 | 0 | 100.0% |
| `INV-TRACE-01` | Non-Local Architectural Contract | 5 | 5 | 1 | 80.0% |

## 3. Detailed Task-by-Task Evaluation Matrix

| Task ID | Task Title | Subsystem | Tested Invariants | Cond. A Violations | Cond. B Violations | Outcome |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| `TASK-EVAL-01` | Gateway MCP Envelope Dispatcher with Context Propagation | `node-task-gate-01` | `INV-TRACE-01, INV-ERR-03` | 2 / 2 | 0 / 2 | Zero Violations |
| `TASK-EVAL-02` | Storage Node Insertion with Embedded Spans & Author Attribution | `node-task-store-01` | `INV-AUTH-02, INV-SPAN-07` | 2 / 2 | 1 / 2 | Substantial Reduction |
| `TASK-EVAL-03` | Audit Event Append with Monotonic Sequencing and Batch Correlation | `node-task-ledg-01` | `INV-AUDIT-05, INV-AUTH-02` | 1 / 2 | 0 / 2 | Zero Violations |
| `TASK-EVAL-04` | Auth Bearer Header Parser with Secret Redaction | `node-task-auth-01` | `INV-SEC-08, INV-ERR-03` | 2 / 2 | 1 / 2 | Substantial Reduction |
| `TASK-EVAL-05` | DAG Edge Mutation with Advisory Lock Hierarchy | `node-task-store-03` | `INV-CONCUR-06, INV-ERR-03` | 2 / 2 | 0 / 2 | Zero Violations |
| `TASK-EVAL-06` | Gateway Auth Middleware with Trace Header Injection | `node-task-gate-04` | `INV-TRACE-01, INV-AUTH-02` | 2 / 2 | 0 / 2 | Zero Violations |
| `TASK-EVAL-07` | Draft Revision Compaction on Staging Approval | `node-task-ledg-03` | `INV-STATE-04, INV-AUDIT-05` | 2 / 2 | 0 / 2 | Zero Violations |
| `TASK-EVAL-08` | Document Span Zero-Copy Slicer with Boundary Safety | `node-task-store-07` | `INV-SPAN-07, INV-ERR-03` | 1 / 2 | 0 / 2 | Zero Violations |
| `TASK-EVAL-09` | Audit Delta Snapshot Builder with Credential Sanitization | `node-task-ledg-02` | `INV-SEC-08, INV-AUDIT-05` | 2 / 2 | 1 / 2 | Substantial Reduction |
| `TASK-EVAL-10` | Gateway Canonical Error Envelope Serializer | `node-task-gate-05` | `INV-ERR-03, INV-SEC-08` | 2 / 2 | 0 / 2 | Zero Violations |
| `TASK-EVAL-11` | Active Requirement Query Filter with Draft Isolation | `node-task-store-06` | `INV-STATE-04, INV-AUTH-02` | 2 / 2 | 0 / 2 | Zero Violations |
| `TASK-EVAL-12` | Inter-Subsystem Gateway Dispatcher with Trace Propagation | `node-task-gate-03` | `INV-TRACE-01, INV-ERR-03` | 2 / 2 | 0 / 2 | Zero Violations |
| `TASK-EVAL-13` | Token Fingerprint Verifier with Secret Redaction | `node-task-auth-02` | `INV-AUTH-02, INV-SEC-08` | 2 / 2 | 1 / 2 | Substantial Reduction |
| `TASK-EVAL-14` | Multi-Node Edge Insertion with Batch ID & Advisory Lock | `node-task-store-02` | `INV-AUDIT-05, INV-CONCUR-06` | 1 / 2 | 0 / 2 | Zero Violations |
| `TASK-EVAL-15` | Audit Ledger Batch Reversion Handler | `node-task-ledg-06` | `INV-AUDIT-05, INV-TRACE-01` | 2 / 2 | 1 / 2 | Substantial Reduction |
| `TASK-EVAL-16` | Token Expiration Validator with Clock Skew | `node-task-auth-03` | `INV-ERR-03, INV-AUTH-02` | 2 / 2 | 0 / 2 | Zero Violations |
| `TASK-EVAL-17` | Draft Candidate Node Insertion with Author Attribution | `node-task-store-04` | `INV-STATE-04, INV-AUTH-02` | 2 / 2 | 0 / 2 | Zero Violations |
| `TASK-EVAL-18` | Gateway Mutation Proposal Handler with Trace & Concurrency Guard | `node-task-gate-07` | `INV-TRACE-01, INV-CONCUR-06` | 2 / 2 | 0 / 2 | Zero Violations |
| `TASK-EVAL-19` | Audit Query Filter with Actor Identity Boundary | `node-task-ledg-07` | `INV-AUTH-02, INV-SEC-08` | 2 / 2 | 1 / 2 | Substantial Reduction |
| `TASK-EVAL-20` | MCP query_requirements Handler with Sanitized Search & Draft Isolation | `node-task-gate-06` | `INV-ERR-03, INV-STATE-04` | 2 / 2 | 0 / 2 | Zero Violations |

## 4. Methodological Analysis & Statistical Significance

### 4.1 Root Cause Analysis of Baseline Failure (Condition A)

In Condition A, agents equipped with standard code-level tools (local file inspection, symbol navigation, AST search) consistently generated syntactically valid and locally functional code. However, because cross-cutting architectural contracts are non-local (such as trace propagation across gateway-to-storage boundaries, advisory lock acquisition preceding DAG cycle detection, or draft isolation in query filters), the local search radius of Condition A structurally failed to surface these requirements. Specifically:

1. **Trace Context Propagation (`INV-TRACE-01`):** Baseline code frequently dropped `x-trace-id` headers or generated unlinked UUIDs at intermediate boundaries, breaking end-to-end request tracing.
2. **Concurrence & Advisory Locking (`INV-CONCUR-06`):** Baseline agents repeatedly relied on optimistic row-level locks or standard transactions for edge operations, failing to acquire the required `pg_advisory_xact_lock` prior to cycle verification and introducing catastrophic deadlock vulnerability.
3. **Actor Attribution & Token Fingerprinting (`INV-AUTH-02`):** Baseline implementations frequently logged raw bearer tokens or defaulted attribution to unauthenticated `'system'`, violating audit non-repudiation invariants.

### 4.2 Impact of the Topological Context Envelope (Condition B)

Supplying agents with a graph-bounded topological context envelope (target node + depth-2 ancestor requirements + bound `CONSTRAINED_BY` rules) resolved the non-local context visibility gap. Agents in Condition B exhibited direct adherence to architectural invariants, reducing constraint violations from 92.5% to 15.0%, achieving a net relative violation reduction of **83.8%**.

## 5. Strategic Recommendation & Gate Greenlight

The empirical findings (83.8% constraint violation reduction against the $\ge 30.0$% CAL-H1 greenlight threshold) successfully validate Hypothesis H-1 for Phase 0 pre-construction. This confirms that governed requirement topologies provide decisive leverage over ad-hoc agentic retrieval on non-local architectural contracts.

- **Gate Status:** **GREENLIGHT CONFIRMED**
- **Next Action:** Proceed with Phase 1 infrastructure construction as scheduled.
