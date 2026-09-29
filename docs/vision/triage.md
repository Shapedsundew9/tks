# Triage Ledger

This triage ledger tracks the architectural disposition of critique findings LD-1 through LD-12 from `docs/vision/response.md` (iteration 8). Every finding has been evaluated from first principles and sorted into Adopt, Defer, or Rebut.

## Findings Triage Table

| ID | Severity | Bucket | Location |
| :--- | :--- | :--- | :--- |
| LD-1 | Blocker | Adopt | `architecture.md` §2.2 (C-4), §3 (INV-2), §5.1 (Document Revision Reconciliation), §6 (Staging Approval), §9 (D-74); `technical-backlog.md` TB-7; `strategic-planning-backlog.md` §1, §4, §7 |
| LD-2 | Major | Adopt | `architecture.md` §2.2 (C-13, C-21), §3 (INV-7), §5.1 (Entity Typing and Relational Constraints), §5.2 (Context Envelope Traversal Guardrails), §6 (`get_context_envelope`, `propose_node_mutation`), §9 (D-75); `strategic-planning-backlog.md` §1, §2, §4 |
| LD-3 | Major | Adopt | `architecture.md` §3 (INV-1), §4, §5.1 (Reverification Interface & Operational Unblocking), §5.2 (Degraded Node Inspection), §6 (`reverify_node`, `get_context_envelope`), §7, §8, §9 (D-76); `strategic-planning-backlog.md` §1, §2, §4 |
| LD-4 | Major | Adopt | `architecture.md` §2.2 (C-22), §5.1 (State Ownership, Entity Typing DDL), §5.2 (Asynchronous Out-of-Band Embedding Generation), §7, §9 (D-77), §11 (Q-4); `strategic-planning-backlog.md` §2 |
| LD-5 | Major | Adopt | `architecture.md` §2.2 (C-21), §4, §5.1 (Entity Typing DDL), §5.2 (Context Envelope Traversal Guardrails), §9 (D-78); `strategic-planning-backlog.md` §2 |
| LD-6 | Major | Adopt | `architecture.md` §5.1 (State Ownership), §5.2 (Two-Stage Mechanical Ingestion), §8 (Failure & Recovery), §9 (D-79); `technical-backlog.md` TB-7; `strategic-planning-backlog.md` §1, §7 |
| LD-7 | Minor | Adopt | `architecture.md` §3 (INV-2), §5.1 (Rollback Cascade Mechanics), §6 (`revert_mutations`), §7, §8, §9 (D-80); `strategic-planning-backlog.md` §1, §2, §4 |
| LD-8 | Minor | Defer | `technical-backlog.md` TB-2 (CommonMark AST Document Decomposition Pipeline), TB-7 (AST Revision Diffing & 3-Tier Reconciliation) |
| LD-9 | Minor | Adopt | `architecture.md` §3 (INV-5), §5.1 (Disambiguated Mutation Pathways), §6 (`propose_node_mutation`), §9 (D-81); `technical-backlog.md` TB-7; `strategic-planning-backlog.md` §1, §2 |
| LD-10 | Minor | Adopt | `architecture.md` §10 (R-1); `strategic-planning-backlog.md` §6 (Quantitative Operational Targets: CAL-H1) |
| LD-11 | Major | Adopt | `architecture.md` §2.2 (C-17), §4 (Topology diagram and Component table), §5.1 (State Ownership, Consolidated DDL), §5.2 (Worker polling), §6, §7, §8, §9 (D-82), §11 (Q-8); `technical-backlog.md` TB-6; `strategic-planning-backlog.md` §1, §2, §7 |
| LD-12 | Minor | Adopt | `architecture.md` §4, §5.1, §6 (Staging Approval and Staging Rejection), §8, §9 (D-83) |

## Upstream Issues

None
