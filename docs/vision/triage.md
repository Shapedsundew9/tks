# Technical Alignment & Triage Ledger

This ledger records the architectural triage decisions for the Lead Developer technical review findings (`LD-1` through `LD-13` from `docs/vision/response.md`).

## Triage Table

| ID | Severity | Bucket | Location |
| :--- | :--- | :--- | :--- |
| `LD-1` | Blocker | Adopt | `architecture.md` §5.1, §6, §9 (D-93); `technical-backlog.md` TB-12; `strategic-planning-backlog.md` Phase 4 Deliverable 4.4 |
| `LD-2` | Blocker | Adopt | `architecture.md` §5.2, §9 (D-94); `strategic-planning-backlog.md` Phase 4 Deliverable 4.1 |
| `LD-3` | Major | Adopt | `architecture.md` §6, §7, §9 (D-95); `technical-backlog.md` TB-13; `strategic-planning-backlog.md` Phase 5 Deliverable 5.1 |
| `LD-4` | Major | Adopt | `architecture.md` §3 (INV-8), §6.2, §9 (D-96); `technical-backlog.md` TB-13 |
| `LD-5` | Major | Adopt | `architecture.md` §5.2, §6.2, §7, §9 (D-97); `technical-backlog.md` TB-13 |
| `LD-6` | Major | Adopt | `architecture.md` §4, §5.1, §5.2, §9 (D-98); `technical-backlog.md` TB-11; `strategic-planning-backlog.md` Phase 4 Deliverable 4.5 |
| `LD-7` | Major | Adopt | `architecture.md` §4, §5.1, §6, §8, §9 (D-99); `strategic-planning-backlog.md` Phase 6 Deliverables 6.2, 6.3 |
| `LD-8` | Major | Adopt | `architecture.md` §4, §7, §8, §9 (D-100); `strategic-planning-backlog.md` Phase 4 Deliverable 4.2 |
| `LD-9` | Major | Adopt | `architecture.md` §5.1, §5.2, §6, §9 (D-101); `strategic-planning-backlog.md` Phase 4 Deliverable 4.6 |
| `LD-10` | Major | Adopt | `architecture.md` §5.1, §9 (D-102); `technical-backlog.md` TB-7.5 |
| `LD-11` | Major | Adopt | `architecture.md` §4, §7, §9 (D-100); `technical-backlog.md` TB-12 |
| `LD-12` | Major | Adopt | `architecture.md` §5.1, §9 (D-93, D-103); `technical-backlog.md` TB-12; `strategic-planning-backlog.md` Phase 4 Deliverable 4.4 |
| `LD-13` | Major | Adopt | `architecture.md` §4, §7, §8, §9 (D-104); `strategic-planning-backlog.md` Phase 4 Deliverable 4.6 |

## Upstream Issues

### UI-1: Provisional Code Commit Graph Nodes vs. Direct Task Verification Binding

* **Document:** `docs/vision/vision.md` (§5 Invariant I-9 & Key Capability 1)
* **Description:** `vision.md` contains passing narrative references suggesting the creation of provisional `CODE_COMMIT` graph nodes for candidate pull request branches prior to merging into `main`.
* **Architectural Assessment:** As established in `LD-12` and adopted across `architecture.md` (§5.1, D-93, D-103), materializing provisional commit nodes for unmerged PR branches pollutes the property graph with ephemeral commit SHAs that must later be reconciled, superseded, or garbage-collected when PRs are squashed, rebased, or discarded. The architecture standardizes pre-merge CI verification strictly on binding `VERIFIED_BY` edge attributes (`attributes->'pr_commit_sha'`) directly to active `TASK` entities. Permanent `CODE_COMMIT` nodes are materialized exclusively upon canonical merge to `main`.
* **Recommendation for Project Initiator:** In the next revision cycle of `docs/vision/vision.md`, update Invariant I-9 and Key Capability 1 descriptions to clarify that pre-merge verification status attaches directly to `TASK` nodes via `VERIFIED_BY` edges carrying the PR commit SHA attribute, reserving `CODE_COMMIT` graph nodes strictly for canonical merges to `main`.
