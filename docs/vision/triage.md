# Architectural Triage & Upstream Issues (Iteration 2)

## 1. Triage Decisions

The following table summarizes the triage of all Lead Developer findings from [Lead Developer Architectural Review (Iteration 2)](file:///workspaces/tks/docs/vision/response.md). Every finding has been categorized into exactly one bucket (*Adopt into Architecture*, *Defer to Technical Backlog*, or *Respectfully Rebut*):

| ID | Severity | Bucket | Location |
| :--- | :--- | :--- | :--- |
| **LD-1** | Major | Adopt | `architecture.md` §1 (DR-11), §2.2 (C-11), §4 (Component Responsibilities - Decomposition Pipeline & Worker), §5.2 (Concurrency Model), §6 (Document Ingestion), §7 (Technology Stack), §9 (D-15); `strategic-planning-backlog.md` §5 (Spike 4); `technical-backlog.md` TB-2 |
| **LD-2** | Major | Adopt | `architecture.md` §1 (DR-12), §2.2 (C-13), §3 (INV-2), §5.1 (Data & State Model - Draft Lifecycle & Event Compaction), §9 (D-16 superseding D-14); `strategic-planning-backlog.md` §5 (Spike 7) |
| **LD-3** | Major | Adopt | `architecture.md` §4 (Storage Repository Layer), §5.2 (Context Envelope Traversal Guardrails), §6 (`get_context_envelope`), §9 (D-17); `strategic-planning-backlog.md` §5 (Spike 5) |
| **LD-4** | Major | Adopt | `architecture.md` §4 (Component Topology - Embedding Worker & Queue), §5.1 (State Ownership), §5.2 (Asynchronous Out-of-Band Embedding Generation), §9 (D-18), §11 (Q-8 resolved) |
| **LD-5** | Major | Defer | `technical-backlog.md` TB-1 (Bare Git Repository Direct ODB Ingestion, Permanent Reference Namespaces, and Volume Backup Synchronization) |
| **LD-6** | Major | Adopt | `architecture.md` §2.2 (C-12), §4 (CLI Stdio Adapter, Deployment & Process Model), §7 (CLI & Stdio integration), §8 (Build & Deploy), §9 (D-19 updating D-7) |
| **LD-7** | Major | Adopt | `architecture.md` §5.2 (Topological Mutations & Global Advisory Locking), §9 (D-20 updating D-8) |
| **LD-8** | Minor | Defer | `technical-backlog.md` TB-3 (Schema Versioning and StagedPayload Enum Mapping for Staging Queue) |
| **LD-9** | Minor | Adopt | `architecture.md` §4 (Component Topology & Responsibilities - Storage Repository Layer), §5.2 (Concurrency Model), §7 (Technology Stack) |
| **LD-10** | Minor | Adopt | `architecture.md` §4 (Storage Substrate), §5.1 (Relational Ingestion Unification), §6 (Ingestion Job Polling) |
| **LD-11** | Minor | Adopt | `architecture.md` §4 (CLI Stdio Adapter), §7 (CLI & Stdio integration), §9 (D-19 updating D-7) |

---

## 2. Upstream Issues in `vision.md`

The following potential discrepancies and clarifications in [Technical Vision](file:///workspaces/tks/docs/vision/vision.md) are raised for the Project Initiator's consideration:

1. **Mechanical Pre-Parsing Philosophy Missing from Ingestion Topology (`vision.md` §4):**
   - *Observation:* The information flow diagram and pipeline description in `vision.md` §4 describe raw specifications being fed directly from the Git Document Ledger to the "Assisted Decomposition Assistant (LLM Parser via API)".
   - *Recommendation:* To reflect the Project Initiator's new directive emphasizing mechanical 80/20 extraction over LLM generation to minimize output tokens, `vision.md` §4 could be updated in an upcoming revision to explicitly depict mechanical CommonMark AST decomposition prior to LLM semantic classification.
2. **Clarification of Invariant I-2 for Draft Lifecycle Compaction (`vision.md` §5):**
   - *Observation:* Invariant I-2 currently states that "Destructive in-place updates (`UPDATE` or `DELETE`) on requirements, specifications, and topological edges are strictly prohibited. State transitions must be recorded such that every state mutation is fully auditable, reversible, and point-in-time reconstructible without data loss."
   - *Recommendation:* To align with the Project Initiator's mandate that intermediate drafting must not generate excessive version control churn or audit history bloat, `vision.md` Invariant I-2 should clarify that the strict immutability and audit-event logging mandate applies to approved/active requirements, while draft entities are subject to lifecycle compaction (squash on approval) into a single canonical audit event with summary metadata.
