# Stakeholder Response — Iteration 3

**Role:** Stakeholder (Committed Ally / Constructive Skeptic)
**Date:** 2026-09-28

---

## Executive Summary

The vision and backlog are in strong shape. The problem framing is disciplined,
the kill conditions are honest, and the phased bootstrapping path is pragmatic.
After careful review and external research, I am raising **four items** that I
believe meet the threshold of significant feedback. Two are corrections to
factual claims, one surfaces a latent strategic risk that deserves explicit
acknowledgment, and one identifies a missing environmental constraint.

---

## Item 1 — SQL/PGQ Factual Correction (Backlog §5, Spike 1)

### The Claim (Backlog, Line 240)

> "SQL/PGQ (SQL:2023 Part 16) was removed from PostgreSQL 19 development
> branches in September 2026 due to catalog stability, concurrency, and
> security concerns."

### The Problem

This is partially incorrect and risks anchoring Spike 1 decisions on a stale
premise. Current public evidence (PostgreSQL Beta 4 release notes, September 24,
2026; neon.com coverage) shows that SQL/PGQ was **reverted** from the PostgreSQL
19 release cycle, but multiple sources indicate it is **expected to reappear in
PostgreSQL 20** after further community design work. Separately, Apache AGE now
supports PostgreSQL 11–18, with PG 19 support pending the stable release.

More importantly, the Spike 1 text describes SQL/PGQ removal as a settled, final
outcome. It is more accurate to call it a *deferral*. The architectural
implication is the same for Phase 1 (don't depend on SQL/PGQ now), but the
forward-looking text should acknowledge that native SQL/PGQ may arrive in PG 20
and could eventually simplify or replace the chosen graph query strategy.

### Recommended Action

**Backlog correction (minor).** Reword Spike 1 context to:

> "SQL/PGQ (SQL:2023 Part 16) was reverted from the PostgreSQL 19 release cycle
> due to design and stability concerns. Native SQL/PGQ support may appear in a
> future major release (earliest PG 20). The project requires an alternative
> graph query approach for initial phases."

Add a forward note:

> "Re-evaluate native SQL/PGQ feasibility when PG 20 reaches beta."

This is a factual correction, not a vision change, but it affects a backlog
spike's framing and should be fixed.

---

## Item 2 — Character-Span Extraction Risk Is Under-Weighted (Vision §2.4 / Backlog Spike 4)

### The Concern

Invariant I-4 (Cryptographic Source Anchoring) mandates that every derived
requirement stores "source span coordinates pointing to the original document
artifact." The entire provenance guarantee chain hangs on the accuracy of
character offsets produced by the assisted decomposition pipeline (Hypothesis
H-4).

External research (LLMStructBench 2026; industry practitioner consensus)
confirms what the backlog's Spike 4 already suspects: **LLMs remain
fundamentally challenged by character-level precision** due to tokenization
boundaries. Models are reliable at schema-level structured output (valid JSON,
correct field types) but demonstrably unreliable at returning *exact character
offsets* into source documents, especially with:

- UTF-8 multibyte sequences
- Markdown formatting artifacts (link syntax, fenced blocks, nested lists)
- Whitespace normalization differences between the model's internal
  representation and the raw byte stream

The backlog's CAL-H4 target (≥ 95% precision/recall on atomic requirement spans)
is ambitious. Industry evidence suggests that achieving 95% on *span boundary
accuracy* (as opposed to entity-level F1) is at the upper end of what
state-of-the-art models achieve even with constrained generation, and that this
accuracy degrades on structurally complex Markdown.

### Why This Is Significant

If span extraction proves unreliable, the project faces a fork:

1. **Relax Invariant I-4** to allow approximate spans (e.g., paragraph-level or
   section-level anchoring), which weakens the provenance guarantee.
2. **Supplement LLM extraction with deterministic post-processing** (e.g.,
   fuzzy-match the extracted text against the source document to re-derive exact
   offsets), which adds pipeline complexity but preserves the invariant.

Neither path is covered in the current vision or backlog.

### Recommended Action

**Add a backlog item** (Spike 4 addendum or standalone) to explicitly plan for
deterministic span re-anchoring as a fallback:

> "If LLM-produced character offsets fall below the CAL-H4 threshold, implement
> a deterministic post-processing step that fuzzy-matches extracted requirement
> text against the source document byte stream to re-derive verified span
> coordinates. This preserves Invariant I-4 without requiring the LLM itself to
> produce exact offsets."

This is a low-cost architectural hedge that should be identified now, not
discovered during Phase 1 integration.

---

## Item 3 — Competitive Positioning / Build-vs-Leverage Gap

### The Concern

The vision document (§1) defines the problem space convincingly but contains
**no explicit acknowledgment of the existing competitive landscape**. There are
mature, commercially supported tools in this space:

- **Enterprise ALM platforms** (Jama Connect, IBM DOORS Next, PTC Codebeamer,
  Siemens Polarion) provide end-to-end requirements traceability with audit
  trails, albeit designed for human-centric workflows, not agentic ones.
- **Modern DevOps integrations** (Modern Requirements / MR4DevOps, Qase) provide
  traceability matrices that plug into existing issue trackers.
- **Lightweight graph-native approaches** (ReqView + Neo4j export, SARA-style
  Git/Markdown knowledge graphs) overlap directly with TKS's niche.

The vision's differentiation is the *agent-agnostic MCP gateway* and the
*graph-bounded context envelope* thesis. These are genuinely novel. But the
document never states why an enterprise team should build TKS instead of
extending an existing ALM tool with an MCP adapter layer, or why a solo
developer should build a PostgreSQL substrate instead of using ReqView + Neo4j.

### Why This Is Significant

This is not a gap in the technical architecture — it is a gap in the *strategic
justification*. Without an explicit "why not just extend X?" section, the vision
is vulnerable to the objection: "You are building a requirements management
system from scratch when dozens exist. The only novel part is the MCP context
envelope, which could be a plugin for an existing tool."

### Recommended Action

**Vision addition (§1 or §2).** Add a brief (3–5 sentence) positioning
statement that explicitly addresses why TKS must be a purpose-built substrate
rather than an extension layer on existing ALM tools. The core argument should be
that existing tools treat traceability as a *reporting* concern (backward-looking
audit), whereas TKS treats it as an *operational* concern (forward-looking agent
context assembly), and that this distinction requires the graph topology to be
the primary storage model, not a secondary export.

This is a vision-level addition — it clarifies the problem definition and
success criteria by contrast.

---

## Item 4 — Missing Resource / Team-Size Constraint Acknowledgment

### The Concern

The vision and backlog implicitly assume a multi-person engineering team (e.g.,
references to "engineering leadership," "human engineering teams," "development
team"). The Cargo.toml and repository structure suggest this is currently a
**single-developer project** (or very small team). The four-phase roadmap
through to closed-loop lifecycle verification (Phase 4) is substantial — it
encompasses a Git-backed document store, a PostgreSQL graph engine, an MCP
server, a REST API, an LLM-assisted decomposition pipeline, a governance policy
engine, a web supervisory portal, CI/CD webhook integration, and an audit ledger.

There is no explicit statement anywhere in the vision or backlog about the
resource constraint under which this project operates, or about how the phased
plan maps to a realistic execution timeline given the available effort.

### Why This Is Significant

This matters because the bootstrapping strategy (§3) — "use the tool to build
the tool" — is the project's primary risk mitigation for the scope/effort
problem. If the project never reaches Phase 1 completion because Phase 0 + Phase
1 scope is too large for the available effort, the self-referential feedback loop
never engages, and the entire acceleration premise fails.

The kill condition for this (Kill #4, Bootstrapping Failure Falsification)
triggers *after* Phase 1, but the actual risk is *reaching* Phase 1.

### Recommended Action

**Vision addition (§3 or §6).** Add an explicit acknowledgment:

> "The project operates under a constrained resource model. The phased roadmap
> is designed so that each phase delivers standalone value and can be evaluated
> independently. Phase 0 and Phase 1 are scoped to be achievable by a small team
> (or individual developer) using commodity infrastructure. Phases 3 and 4 are
> aspirational targets contingent on the demonstrated viability of earlier
> phases."

This is not a change to the North Star — it is an environmental boundary
contract (§3 scope) that is currently unstated. Stating it explicitly protects
against scope creep in Phase 0/1 planning and makes the "start small" directive
concrete rather than aspirational.

---

## Items Considered and Not Raised

For transparency, I evaluated and chose *not* to raise the following:

- **Mermaid diagram styling consistency:** Purely stylistic; no impact on
  vision clarity.
- **The Phase 3 Web Portal as scope creep risk:** Already implicitly mitigated
  by the phased gating model; raising it would be a strategic/tactical
  discussion, not a vision concern.
- **Embedding model selection specifics:** Correctly deferred to the backlog.
  No vision-level concern.
- **The "single PostgreSQL instance" constraint (H-3):** The kill condition
  and graduated response model already cover this adequately. The pgvector
  benchmarks I reviewed (5–30ms p50 at 1M rows for vector search; sub-second
  recursive CTEs at millions of edges with proper indexing) suggest the
  hypothesis is plausible at the scale targets specified in SLA-1/SLA-2. No
  reason to escalate.

---

## Summary of Recommended Actions

| # | Type | Scope | Action |
| --- | ------ | ------- | -------- |
| 1 | Factual correction | Backlog Spike 1 | Reword SQL/PGQ context from "removed" to "reverted/deferred"; add PG 20 re-evaluation note |
| 2 | Risk mitigation | Backlog Spike 4 / new item | Add deterministic span re-anchoring fallback plan for when LLM offsets miss CAL-H4 |
| 3 | Strategic clarification | Vision §1 or §2 | Add competitive positioning statement ("why not extend existing ALM tools?") |
| 4 | Environmental constraint | Vision §3 or §6 | Acknowledge resource/team-size constraint; tie "start small" to concrete effort model |
