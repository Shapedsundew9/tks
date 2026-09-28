# Lead Developer Review Triage Ledger (Iteration 1)

| ID | Severity | Bucket | Location |
| :--- | :--- | :--- | :--- |
| LD-1 | Blocker | Adopt | ## 4. Component Topology |
| LD-2 | Major | Adopt | ## 3. Architectural Invariants |
| LD-3 | Major | Adopt | ## 5. Data & State Model |
| LD-4 | Major | Adopt | ## 5. Data & State Model |
| LD-5 | Major | Adopt | ## 5. Data & State Model |
| LD-6 | Major | Adopt | ## 6. Interfaces & Contracts |
| LD-7 | Major | Adopt | ## 3. Architectural Invariants |
| LD-8 | Major | Defer | TB-1 |
| LD-9 | Major | Adopt | ## 5. Data & State Model |
| LD-10 | Major | Adopt | ## 4. Component Topology |
| LD-11 | Major | Rebut | ### Rebuttal: LD-11 |
| LD-12 | Major | Adopt | ## 4. Component Topology |

## Upstream Issues

The following architectural tensions and specification gaps were identified during review of `vision.md` and `strategic-planning-backlog.md` for the Project Initiator:

1. **Authentication Protocol Transport Gap (vision.md §5 Invariant I-7 vs. strategic-planning-backlog.md §5 Spike 6):**
   Invariant I-7 mandates non-negotiable identity attribution on every mutation, while Spike 6 targets OAuth 2.1 with PKCE and Agent Identity Working Group DPoP federation. However, standard local agent harnesses (Claude Code, Cursor, Windsurf) connect over Model Context Protocol (MCP) using `stdio` process pipes, which lack native HTTP authorization headers. Mandating external OAuth 2.1 + DPoP token validation for Phase 1 and Phase 2 imposes prohibitive infrastructure complexity on solo developers or small teams (violating C-9). The governing backlog should formally recognize the two-tier authentication strategy adopted in D-11: pre-shared cryptographic API keys or HMAC bearer tokens mapped to an in-database `agent_identities` table for Phase 1–2, with external OAuth 2.1 / DPoP workload federation deferred to Phase 4.

2. **Hot-Path Traversal Penalty from Untyped Metadata (strategic-planning-backlog.md §2 Phase 2 & §5 Spike 7):**
   Spike 7 and Phase 2 backlog items specify storing node lifecycle states (`ACTIVE`, `SUPERSEDED`, `ARCHIVED`) and governance policies within untyped JSONB metadata attributes. Because recursive Common Table Expression (CTE) traversals must evaluate `lifecycle_state` at every recursion step to exclude superseded entities and meet the sub-50ms SLA-1 target, extracting JSONB fields dynamically incurs significant row-level parsing overhead and defeats standard PostgreSQL B-tree index scans. The backlog should be updated to model `lifecycle_state`, `governance_policy`, and `node_type` as first-class, strongly typed SQL columns with database `CHECK` constraints on `graph_nodes`.

3. **Synchronous Ingestion Timeout and Task Durability (strategic-planning-backlog.md §4 Phase 1 MVD & §7 Sequence Diagram):**
   The operational workflow in §7 depicts document ingestion executing LLM-assisted decomposition inline within the HTTP request/response cycle. LLM document decomposition and deterministic character span re-anchoring across large technical specifications routinely require 15–90 seconds, inducing HTTP gateway timeouts. Furthermore, spawning background async tasks (`tokio::spawn`) without persistent state risks silent data loss upon process restarts. The sequence diagram and backlog should be updated to reflect an asynchronous job queue model (`ingestion_jobs`), where `POST /api/v1/documents/ingest` commits the Git artifact, enqueues the job, and immediately returns `202 Accepted` with a `job_id` for status polling via `GET /api/v1/documents/ingest/{job_id}`.

4. **Relational Ordering Deadlock on Invariant I-1 (vision.md §5 Invariant I-1 & strategic-planning-backlog.md §2 Phase 1):**
   Invariant I-1 mandates that orphan execution tasks must be rejected at the database constraint level. In a normalized relational schema where graph nodes and directed edges occupy separate tables (`graph_nodes` and `graph_edges`), an inserted node must physically exist before an edge referencing its foreign key can be inserted. If database constraints or triggers validate ancestor requirement paths immediately on node insertion, valid tasks cannot be created. The strategic backlog should clarify that database-level enforcement of Invariant I-1 relies on PostgreSQL `DEFERRABLE INITIALLY DEFERRED` constraint triggers evaluated at transaction `COMMIT` time or atomic transactional insertion CTEs.
