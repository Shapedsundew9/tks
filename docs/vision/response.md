# Stakeholder Response — Iteration 1

**Reviewer Role:** Stakeholder (Committed Ally, Realist, Pragmatist)
**Document Under Review:** [`vision.md`](file:///workspaces/tks/docs/vision/vision.md)
**Date:** 2026-09-28

---

## Overall Assessment

The vision is **compelling, well-structured, and defensible**. The problem
framing is sharp, the three failure modes are real and observable daily, and the
core thesis — that graph-structured traceability will outperform flat vector
retrieval for agentic workflows — is a bet worth making. The competitive
landscape is more populated with adjacent tools than might be assumed (code-graph
engines like Augment and Greptile; requirement-graph tools like SARA and
Proj-Theseus), but no existing tool combines requirement-level property graphs,
an agent-facing MCP gateway, and per-node governance into a unified system. The
niche is defensible.

That said, I have significant feedback on five fronts: a factual correction, a
missing foundational principle, strategic scoping aligned with the initiator's
commentary, a vision-vs-strategy cleanup directive, and a challenge to one
commitment that could become an early trap.

---

## Feedback Item 1 — CHALLENGE: SQL/PGQ Is a Phantom Dependency

**Section affected:** §2 Key Capabilities, §2 Conceptual Grounding, §3
Boundary Contracts

**Issue:** The document references "SQL/PGQ" as if it is an available PostgreSQL
capability (line 89: *"graph topologies (SQL/PGQ)"*). **SQL/PGQ does not exist
in any released PostgreSQL version.** While SQL/PGQ (SQL:2023 Part 16) was
briefly merged into the PostgreSQL 19 development branch, it was **officially
reverted and removed on September 7, 2026** prior to the PG19 release, due to
severe stability flaws: locking and dependency resolution bugs, backend crashes
and catalog corruption under concurrent DDL, and security vulnerabilities
reachable by low-privilege users. Core committers do not guarantee its inclusion
even in PostgreSQL 20. This is a factual error that could mislead strategy
planning.

**What exists today:**

- **Apache AGE** — an Apache top-level project adding openCypher graph query
  support on PostgreSQL 14–16 ([age.apache.org](https://age.apache.org)).
- **Recursive CTEs (`WITH RECURSIVE`)** — standard SQL, zero extension
  dependencies, battle-tested for hierarchical/DAG traversal in PostgreSQL.
- **pgvector** — mature and widely adopted; no concern here.

**Recommendation (Vision-level):** Replace the reference to "SQL/PGQ" with a
technology-neutral framing: *"graph topologies modeled within PostgreSQL"*. The
specific extension or query approach (AGE, recursive CTEs, or eventual PGQ) is a
strategy/architecture decision, not a vision commitment. The vision should commit
to *the property graph model*, not to a specific query dialect that doesn't exist
yet.

---

## Feedback Item 2 — ADD: Bootstrapping Self-Reference as a Foundational Principle

**Relates to:** Project Initiator Commentary — *"it can define itself... eat our
own dog food"*

**Issue:** The initiator explicitly identifies self-reference (using the system
to manage its own development) as a foundational design principle. This is not
merely a tactical preference — it is a **vision-level constraint** that should
shape the problem definition and success criteria. A system that claims to be
*the* substrate for agentic software engineering but cannot manage its own
requirements, specifications, and tasks has a credibility problem.

**Why this belongs in the vision, not just strategy:**

1. It defines a **success criterion**: the system must reach self-hosting
   capability, analogous to a self-hosting compiler.
2. It constrains the **minimum viable scope**: Phase 1 must deliver enough
   capability that Phase 2 planning can be ingested, decomposed, and tracked
   *within the substrate itself*.
3. It creates a **forcing function** against over-engineering: if the system's
   own development team can't use it productively, no external team will either.

**Recommendation:** Add a sixth Key Capability or a new Invariant:

> **Self-Referential Bootstrapping (Dogfooding Principle):** The Knowledge
> Substrate must be capable of managing its own development lifecycle. Each
> development phase shall produce sufficient capability that the subsequent
> phase's requirements, specifications, and tasks are tracked within the
> substrate itself. The system is its own first user.

**Bootstrap risk acknowledgment:** This carries the classic bootstrapping
chicken-and-egg problem (the system must exist to manage its own creation). The
vision should acknowledge this explicitly and define the initial bootstrap
boundary — e.g., *"Phase 0 planning is managed externally; from Phase 1
completion onward, the substrate manages itself."*

---

## Feedback Item 3 — ADD: Git as the Document Ledger Backend

**Relates to:** Project Initiator Commentary — *"Git might be a good backend
version storage, efficient version storage for text documents"*

**Section affected:** §2 Key Capabilities (item 2), §2 Conceptual Grounding
("Document Ledger"), §3 Boundary Contracts ("Document Immutability Guarantee")

**Issue:** The vision currently specifies a custom *"content-addressed,
cryptographically signed (SHA-256) artifact store"* for document storage. The
initiator proposes using **Git** as this store. This is not a minor tactical
detail — it is a **vision-level architectural decision** about whether the
document ledger is a bespoke component or delegates to proven infrastructure.

**The case for Git as document store (vision-level):**

- Git *is* a content-addressed, SHA-256 (or SHA-1, transitioning to SHA-256)
  object store. It already does what the vision describes building.
- Git provides diffing, branching, history traversal, and efficient delta
  compression for text documents *for free*.
- PostgreSQL stores metadata, graph topology, and references (commit hashes,
  file paths) to documents in Git — this is the coupling the initiator envisions.
- The REST API retrieves document content by resolving Git references stored in
  the database.
- This eliminates an entire bespoke subsystem from the build scope.

**The case for caution:**

- Git is optimized for source code, not arbitrary binary blobs. The vision should
  clarify that the document store is scoped to **text-based documents (Markdown,
  plain text, structured specifications)** — which is consistent with the stated
  scope.
- Character-span references into documents become fragile across Git revisions.
  The vision should acknowledge that span stability across document versions is a
  design challenge to be resolved at the strategy level.

**Recommendation:** Elevate this to the vision level by amending the Document
Ledger concept:

> **Document Ledger:** A Git-backed, content-addressed artifact store for
> text-based specification documents, with PostgreSQL maintaining the relational
> metadata, graph references, and decomposition mappings. Document identity is
> anchored to Git commit hashes and object SHAs.

This shifts the vision from "we build a custom content-addressed store" to "we
compose proven infrastructure (Git + PostgreSQL)" — a far more pragmatic and
credible foundation for a small team.

---

## Feedback Item 4 — CHALLENGE: Vision/Strategy/Tactics Separation Needed

**Relates to:** Project Initiator Commentary — *"a little bit of a mix of
tactics, strategy, and vision"*

**Issue:** The initiator is correct. The document intermixes three distinct
layers, and this creates risk: strategy-level details baked into the vision
become rigid commitments before they've been validated. Specific items that are
**strategy or tactics masquerading as vision**:

| Current Location | Content | Proper Layer |
| --- | --- | --- |
| §2 Conceptual Grounding | `SHA-256` as the specific hash algorithm | **Strategy** — the vision commits to cryptographic content-addressing; the algorithm choice is implementation |
| §2 Operational Timescales | `< 50ms` latency target | **Strategy** — the vision commits to real-time responsiveness; the specific SLA is a strategic target |
| §3 Boundary Contracts | "Single PostgreSQL instance" | **Vision/Strategy boundary** — the vision commits to operational simplicity and single-engine philosophy; "single instance" is a scaling strategy |
| §4 Communication Contracts | Detailed 9-step workflow | **Tactics** — this is an implementation specification, not a vision statement |
| §5 Hypotheses | Specific percentage thresholds (`≥ 70%`, `≥ 50%`, `≥ 95%`) | **Strategy** — the vision commits to measurable hypotheses; the specific thresholds are strategic targets to be calibrated |
| §6 Roadmap | Four-phase execution plan | **Strategy** — the vision defines success criteria and capability progression; the specific phase structure is strategic planning |
| §7 MVDs | Detailed demonstration scripts | **Tactics** — acceptance criteria belong in strategy/planning |

**Recommendation:** The vision document should be refactored into two tiers:

1. **Vision (this document):** Problem space, north star statement, key
   capabilities, foundational invariants, strategic hypotheses (without specific
   thresholds), and kill conditions.
2. **Strategic Planning Backlog (separate document):** Roadmap phases, MVD
   scripts, operational timescale targets, specific technology choices, and
   workflow specifications.

This is not a request to delete content — it's a request to *relocate* it so the
vision remains stable while strategy can iterate freely. The current document
tries to be both a constitution and an execution plan, and it will become
unwieldy as strategy evolves.

---

## Feedback Item 5 — CHALLENGE: Bitemporal Versioning Complexity vs. "Start Small"

**Section affected:** §2 Key Capabilities (item 4), §3 Boundary Contracts, §5
Invariant I-2

**Relates to:** Project Initiator Commentary — *"this needs to start small"*

**Issue:** The vision commits to *"append-only bitemporal lineage"* and
*"Invariant I-2: Destructive in-place updates... strictly prohibited"* as
foundational, non-negotiable properties. Bitemporal data modeling (tracking both
*valid time* and *transaction time* across all entities and edges) is a
well-understood but **notoriously complex** pattern to implement correctly.

**The tension:** The initiator wants to start small and iterate. Bitemporal
versioning across a full property graph is one of the hardest data modeling
problems in database engineering. Building it correctly from day one is a
multi-month effort that delays the first useful capability. Building it
incorrectly and retrofitting it later is even more expensive.

**Pragmatic recommendation:** The vision should distinguish between:

- **The principle** (vision-level): All state changes are auditable, reversible,
  and historically reconstructible. No data is silently destroyed.
- **The mechanism** (strategy-level): Whether this is achieved via full
  bitemporal modeling, simpler event-sourcing/append-only logs, or PostgreSQL
  temporal tables (available since PG13 via `system_time` versioning) is an
  implementation decision.

Reframe Invariant I-2 to commit to the *property* (audit trail, reversibility,
no silent data loss) without prescribing the specific data modeling pattern.
Let the strategy phase choose the simplest mechanism that satisfies the
invariant — which might be straightforward event-sourcing with snapshot
reconstruction, not full bitemporality.

---

## Items Reviewed and NOT Flagged

For completeness, the following aspects were evaluated and found sound at the
vision level:

- **Problem framing (§1):** The three failure modes are real, clearly
  articulated, and well-differentiated. No changes recommended.
- **Core vision statement (§2):** Crisp, appropriately scoped, agent-agnostic.
  No changes recommended.
- **MCP as integration protocol (§3):** MCP adoption is strong and growing
  (Claude, Cursor, Windsurf, VS Code/Copilot, JetBrains, OpenAI agents SDK all
  support or are adopting it). This is a well-founded bet.
  Source: [modelcontextprotocol.io](https://modelcontextprotocol.io/)
- **Zero in-database agent execution (Invariant I-3):** Correct and important
  boundary. No changes recommended.
- **Per-node governance (Invariant I-5):** Novel and well-motivated. No changes
  recommended.
- **Kill conditions (§7):** Well-constructed falsification criteria. These are a
  sign of intellectual honesty and should be preserved.
- **Competitive positioning:** The landscape is more populated with adjacent
  competitors than might be expected, though the vision's specific synthesis
  remains differentiated:
  - *Code-graph tools* (Augment Code, Greptile, Qodo, Sourcegraph SCIP,
    RepoGraph) build AST/call-graph/dependency structures for coding agents —
    but these operate at the *code* layer, not the *requirement-to-code
    traceability* layer.
  - *Requirement-graph tools* exist: **SARA** manages requirements as knowledge
    graphs using Git + Markdown (notably aligned with the initiator's Git
    suggestion); **Proj-Theseus** uses Neo4j for multi-level requirement
    traceability; **Graphiti** tracks temporal knowledge graph mutations.
  - *Traditional ALM* (IBM DOORS, Jama Connect) provides traceability but
    without agent-facing APIs.
  - **No tool found** combines all three of: requirement-level property graph +
    agent-facing MCP gateway + per-node governance. The vision's niche is
    defensible but not as empty as it may appear — and SARA in particular
    warrants close study as a potential prior-art signal for the
    Git-as-document-store concept.

---

## Summary of Recommended Vision Changes

| # | Type | Item | Action |
| --- | --- | --- | --- |
| 1 | **Correct** | SQL/PGQ reference | Replace with technology-neutral "graph topologies modeled within PostgreSQL" |
| 2 | **Add** | Self-referential bootstrapping principle | Add as Key Capability or Invariant; acknowledge bootstrap boundary |
| 3 | **Add** | Git as document ledger backend | Amend Document Ledger concept to specify Git-backed store coupled to PostgreSQL |
| 4 | **Restructure** | Vision/Strategy separation | Relocate roadmap, MVDs, workflow specs, and specific thresholds to a strategic planning document |
| 5 | **Reframe** | Bitemporal versioning | Commit to the audit/reversibility *property* at vision level; defer mechanism choice to strategy |
