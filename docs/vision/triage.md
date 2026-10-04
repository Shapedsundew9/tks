# Triage Ledger: Reconciliation Iteration 10

## Triage Table

| ID | Severity | Bucket | Location |
| :--- | :--- | :--- | :--- |
| LD-1 | Blocker | Adopt | `architecture.md` §3 (INV-9), §4, §5.1, §6, §7, §9 (D-107); `strategic-planning-backlog.md` §1, §2 (Phase 4 Deliverable 4.4), §4; `technical-backlog.md` TB-12.1 |
| LD-2 | Blocker | Adopt | `architecture.md` §3 (INV-1), §4, §5.1, §6, §7, §9 (D-108); `strategic-planning-backlog.md` §1, §2 (Phase 4 Deliverable 4.4), §4; `technical-backlog.md` TB-12.2 |
| LD-3 | Major | Adopt | `architecture.md` §5.1, §6, §9 (D-109); `strategic-planning-backlog.md` §1, §2 (Phase 4 Deliverable 4.7), §4; `technical-backlog.md` TB-7.5 |
| LD-4 | Major | Adopt | `architecture.md` §4, §5.1, §5.2, §9 (D-110); `strategic-planning-backlog.md` §1, §2 (Phase 4 Deliverable 4.5); `technical-backlog.md` TB-11.2, TB-11.5 |
| LD-5 | Major | Adopt | `architecture.md` §4, §6, §7, §9 (D-111); `strategic-planning-backlog.md` §2 (Phase 5 Deliverable 5.1); `technical-backlog.md` TB-13.2 |
| LD-6 | Major | Adopt | `architecture.md` §2 (C-11), §4, §5.2, §9 (D-112); `strategic-planning-backlog.md` §2 (Phase 4 Deliverable 4.6b), §4; `technical-backlog.md` TB-2.9 |
| LD-7 | Major | Adopt | `architecture.md` §6, §7, §9 (D-113); `strategic-planning-backlog.md` §2 (Phase 4 Deliverable 4.4), §4; `technical-backlog.md` TB-12.3 |
| LD-8 | Major | Adopt | `architecture.md` §2 (C-22), §4, §5.2, §7, §9 (D-114); `technical-backlog.md` TB-6.1, TB-6.2 |
| LD-9 | Minor | Adopt | `architecture.md` §5.1 (Active Draft Staging Lifecycle and Embedding Ingestion) |
| LD-10 | Major (Simplification) | Adopt | `architecture.md` §3 (INV-7), §4, §5.2, §5.3, §6, §7, §8, §9 (D-105); `strategic-planning-backlog.md` §1; `technical-backlog.md` TB-1.7 |
| LD-11 | Major (Simplification) | Adopt | `architecture.md` §9 (D-115); `strategic-planning-backlog.md` §1, §2 (Phase 6 Deliverable 6.1, Phase N+) |
| LD-12 | Minor (Simplification) | Adopt | `architecture.md` §2 (C-26), §4, §5.1, §6, §7, §8, §9 (D-106); `strategic-planning-backlog.md` §1, §2 (Phase 6 Deliverables 6.2, 6.3), §4 |

## Upstream Issues

1. **Mechanical Sentence/Clause Atomization vs LLM Atomization (`vision.md` §3 / §4):**
   `vision.md` references downstream LLM decomposition performing requirement atomization. To preserve exact 0-based byte offsets (`byte_start`, `byte_end` per Invariant INV-4) and strict token minimization (Constraint C-11: zero verbatim text echoing), sentence and clause atomization must be executed mechanically during Stage 1 CommonMark AST parsing (`unicode-segmentation`), assigning discrete ordinal aliases (`c1`, `c2`, ...). Downstream Stage 2 semantic evaluation then classifies modality and confidence over pre-atomized spans without text echoing. The Project Initiator should update `vision.md` to reflect this two-tier atomization model.

2. **Retirement of Enterprise ALM Protocols (`vision.md` §3 / §6):**
   `vision.md` lists enterprise ALM synchronization protocols (OMG ReqIF and OASIS OSLC) in later phases. These protocols have been formally retired per LD-11 and Decision D-115 in favor of open JSON-LD, REST, and SQL property graph dumps. This avoids hundreds of pages of XML/RDF schema mapping that distract from developer-first agent cognition and governance. The Project Initiator should update `vision.md` to retire ReqIF/OSLC.
