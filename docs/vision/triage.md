# Architectural Triage Ledger

This document records the triage classification for all findings (`LD-1` through `LD-13`) presented in `docs/vision/response.md`.

## Triage Table

| ID | Severity | Bucket (Adopt/Defer/Rebut) | Location |
| :--- | :--- | :--- | :--- |
| LD-1 | Blocker | Adopt | `architecture.md` §5.1, §9 (D-116); `technical-backlog.md` TB-13.2 |
| LD-2 | Major | Adopt | `strategic-planning-backlog.md` §1, §4, §7 |
| LD-3 | Major | Adopt | `architecture.md` §5.1, §9 (D-117); `technical-backlog.md` TB-11.5 |
| LD-4 | Major | Adopt | `architecture.md` §5.1, §6, §9 (D-118); `technical-backlog.md` TB-8 |
| LD-5 | Major | Adopt | `architecture.md` §4, §5.1, §6, §9 (D-119); `technical-backlog.md` TB-1 |
| LD-6 | Major | Adopt | `architecture.md` §6, §9 (D-120); `strategic-planning-backlog.md` §4; `technical-backlog.md` TB-12.3 |
| LD-7 | Major | Adopt | `architecture.md` §6, §9 (D-121); `technical-backlog.md` TB-7.3 |
| LD-8 | Major | Adopt | `architecture.md` §6, §9 (D-122); `technical-backlog.md` TB-13.2 |
| LD-9 | Minor | Defer | `technical-backlog.md` TB-7.6 |
| LD-10 | Minor | Adopt | `architecture.md` §4; `strategic-planning-backlog.md` §4, §7 |
| LD-11 | Major | Adopt | `architecture.md` §5.1, §9 (D-116); `technical-backlog.md` TB-13.2 |
| LD-12 | Major | Adopt | `architecture.md` §6, §7 |
| LD-13 | Major | Adopt | `architecture.md` §5.1, §5.2, §6, §9 (D-123); `technical-backlog.md` TB-6 |

## Upstream Issues

The following issues were identified in `docs/vision/vision.md` for resolution by the Project Initiator:

1. **Pre-Merge CI Verification and VCS Commit Lineage Inversion (`vision.md` §3, §5 Invariant I-9):**
   * *Problem:* `vision.md` §3 and §5 (Invariant I-9) specify creating `VERIFIED_BY` edges directly on `TASK` nodes carrying PR commit metadata, link commits via downward `IMPLEMENTED_BY` edges (`Task -IMPLEMENTED_BY-> Commit`), and reference provisional commit nodes.
   * *Alignment with Architecture:* `architecture.md` (Decisions D-107, D-108, Invariant INV-9) and `strategic-planning-backlog.md` record pre-merge CI test runs directly into `graph_nodes.attributes->'pre_merge_verification'` on active `TASK` entities without creating graph edges, and link canonical VCS merge commits upward via `CODE_COMMIT -IMPLEMENTS-> TASK`, preserving Invariant INV-1 upward ancestry.
   * *Recommended Action:* Update `vision.md` §3 and §5 (Invariant I-9) to reflect that pre-merge test runs are recorded in task attributes and merge commits link upward via `IMPLEMENTS`.

2. **Stale `relationship_review_backlog` Reference in Architecture Diagrams (`vision.md` §4):**
   * *Problem:* In `vision.md` §4, the component topology Mermaid diagram includes a `Backlog` node representing `relationship_review_backlog` (lines 370 and 390).
   * *Alignment with Architecture:* Decision D-106 explicitly retired the `relationship_review_backlog` table in favor of direct structured log/CLI output and existing anomaly triage (`--triage-anomalies` / `attributes->'extraction_metadata'`). References in `architecture.md` and `strategic-planning-backlog.md` have been scrubbed.
   * *Recommended Action:* Update `vision.md` §4 Mermaid diagrams to remove the `Backlog` node and direct relationship audit output to CLI and logs.
