# Stakeholder Response — Iteration 2

---

## Preamble: Overall Assessment

The vision is well-constructed. The problem statement is sharp and grounded in
real, observable failure modes. The invariants are precise. The kill conditions
are intellectually honest and show disciplined risk management. The
single-engine PostgreSQL bet is pragmatically sound for early phases, and the
self-referential bootstrapping strategy is a powerful forcing function. The
scope boundary between vision and strategic backlog is clean.

I have **three significant concerns**, each of which either proposes a change
to the vision document or would materially reprioritize a backlog item.

---

## Concern 1: The Progression Model Should Front-Load De-Risking of Hypothesis H-1

**Affects:** Vision §6 (Multi-Axis Progression Model), Backlog §2 (Phased
Execution Roadmap)

### The Issue

Hypothesis H-1 — that graph-bounded context envelopes significantly
outperform flat vector retrieval for preserving architectural invariants — is
the foundational scientific bet of the entire enterprise. Every other
capability (governance, audit ledger, decomposition pipeline, MCP gateway)
derives its value from this premise. If H-1 is false, the project's
differentiating value proposition collapses regardless of how well the
infrastructure is built.

The current progression model places H-1 validation at **Phase 2** (CAL-H1:
"≥70% fewer architectural violations vs. flat-file vector search, Phase 2
Controlled Trial"). This means the team will design and build the *entire*
Phase 1 infrastructure — Git document store, PostgreSQL graph schema, assisted
decomposition pipeline, read-only MCP server, and dogfooding gate — before
receiving any empirical signal on whether the core thesis holds.

### What Prior Evidence Tells Us

Microsoft's GraphRAG research (2024) demonstrated that graph-based retrieval
outperforms flat vector RAG specifically for **global summarization** and
**multi-hop reasoning** across document corpora. However, those results were
measured on text comprehension tasks, not on code synthesis fidelity or
architectural constraint preservation. The TKS hypothesis is in a
**different domain** — whether graph topology helps agents respect structural
invariants during code generation — and remains genuinely untested.

Critically, the GraphRAG literature also established that graph-based retrieval
can *degrade* below flat-vector performance when graph construction quality is
poor (e.g., entity deduplication failures). The quality of TKS's decomposition
pipeline directly affects H-1 outcomes, creating a compounding dependency.

*(Source: Microsoft Research, "From Local to Global: A Graph RAG Approach to
Query-Focused Summarization," 2024; industry evaluations summarized at
[VentureBeat](https://venturebeat.com), [TigerGraph](https://tigergraph.com),
[Flur.ee](https://flur.ee))*

### Recommendation

**Add a "Spike 0" to Phase 0** (before Phase 1 construction begins): a
lightweight, throwaway evaluation that tests graph-bounded context retrieval
vs. flat-vector retrieval on a controlled agentic coding task. This does not
require the full substrate — it can use an in-memory graph, a small
hand-curated requirement tree (~50-100 nodes), and a standard embedding model.
The goal is directional signal, not production benchmarking.

This is not a change to the vision's *destination* — H-1 remains a hypothesis
to validate — but a change to the *progression model's risk ordering*. The
vision should state that the highest-risk hypotheses are tested at the
earliest possible phase, not deferred until substantial infrastructure exists.

**Proposed vision change:** Add a sentence to §6 "The Bootstrapping
Progression Strategy" under Phase 0 Baseline:

> *Phase 0 additionally includes lightweight evaluation spikes to generate
> early directional signal on the highest-risk strategic hypotheses (H-1,
> H-4), reducing the probability of significant infrastructure investment on
> unvalidated premises.*

**Proposed backlog addition:** Add "Spike 0: Graph-Bounded vs. Flat-Vector
Retrieval Directional Evaluation" before Spike 1, scoped to a throwaway
prototype testable in days, not weeks.

---

## Concern 2: Agent Identity and Trust Requires a Vision-Level Invariant

**Affects:** Vision §3 (Boundary Contracts), Vision §5 (Invariants)

### The Issue

The vision defines five invariants and six boundary contracts. Per-node
governance (I-5) controls *authorization* — what an agent is permitted to do
at each node. The stateless gateway boundary contract specifies
"authenticated, isolated transactions." But no invariant or boundary contract
addresses:

1. **How agents are identified.** What credential does an agent present? How
   is an agent distinguished from a human user?
2. **How agent identity is audited.** The audit ledger records mutations, but
   does it record *which specific agent instance* made the mutation, with
   verifiable identity?
3. **Defense against policy-compliant semantic corruption.** Per-node
   governance prevents unauthorized *structural* mutations, but a compromised
   or hallucinating agent operating under `AUTONOMOUS_ELABORATION` can submit
   mutations that are structurally valid but semantically destructive — subtly
   rewriting requirement text, introducing contradictory sub-specifications,
   or creating circular dependency chains. The governance model assumes agents
   are well-intentioned but need guardrails; it does not address adversarial
   or corrupted input.

### Why This Is Vision-Level, Not Strategic

The MCP specification itself is actively evolving its security model. The
July 2026 MCP specification moved to stateless request/response architecture
and standardized on OAuth 2.1 with PKCE for authentication. An Agent Identity
Working Group is driving formalization of workload identity federation and
proof-of-possession tokens (DPoP) for autonomous agent authentication. The
"confused deputy" problem — where agents are manipulated into performing
unauthorized actions downstream — is a recognized open challenge in the MCP
ecosystem.

*(Source: MCP specification updates 2025-2026; Agent Identity Working Group
roadmap at [modelcontextprotocol.io](https://modelcontextprotocol.io))*

TKS's vision explicitly positions itself as an "agent-agnostic" substrate
accepting mutations from arbitrary external agents. This makes agent identity
and trust a *foundational architectural concern*, not an implementation detail.
If the audit ledger cannot attributably trace mutations to verified agent
identities, Invariant I-2 (auditability) is structurally weakened — you can
audit *what* changed, but not reliably *who* changed it.

### Recommendation

**Add Invariant I-7 (Agent Identity Attribution):**

> *Every mutation submitted through the Integration Gateway must be
> attributable to a verified external identity (human user or autonomous agent
> instance). Agent identity credentials must be recorded in the audit ledger
> alongside mutation events. The specific authentication mechanism is resolved
> at the strategic planning level, but identity attribution is a
> non-negotiable audit requirement.*

**Add a boundary contract clause** to "Stateless Gateway Boundary":

> *The gateway must validate caller identity on every request. No mutation may
> be committed to the audit ledger without a verified identity reference.*

This keeps the vision appropriately abstract (no OAuth/PKCE/DPoP
implementation details) while establishing identity attribution as a
foundational principle alongside traceability and auditability.

---

## Concern 3: Kill Conditions Are Binary — The Vision Should Acknowledge Graduated Outcomes

**Affects:** Vision §7 (Falsification & Termination Criteria)

### The Issue

The four kill conditions are framed as binary pass/fail gates:

- Kill #2: If graph retrieval shows "comparable rates" to flat-vector, "the
  graph-native thesis is falsified."
- Kill #3: If PostgreSQL can't maintain latency, "the single-engine
  architectural boundary must be abandoned."

But the calibration targets in the backlog define *specific thresholds* on a
continuous scale (CAL-H1: ≥70% reduction; CAL-H3: <100ms at 10⁶ nodes). What
happens when results fall in the middle?

- **Scenario A:** Graph retrieval shows 35% fewer architectural violations
  (not 70%). Is the project dead? The graph provides measurable value but
  doesn't meet the aspirational target.
- **Scenario B:** PostgreSQL handles 10⁵ nodes at <100ms but degrades at
  5×10⁵. The vision says "enterprise scale," but many real projects never
  exceed 10⁵ nodes.

The current framing creates a false dichotomy: either the hypothesis is
triumphantly validated or the project is terminated. In practice, partial
validation is the most likely outcome for any novel system, and the
appropriate response is *scope adjustment*, not termination.

### Recommendation

**Add a "Graduated Response" clause** to §7, after the kill conditions:

> *If empirical results partially validate a strategic hypothesis — delivering
> measurable but below-target improvements — the appropriate response is scope
> adjustment rather than program termination. Partial validation may warrant
> narrowing the target domain (e.g., focusing on projects with specific
> structural characteristics where graph retrieval demonstrably excels),
> adjusting calibration targets, or hybridizing approaches. Kill conditions
> are triggered only when results show no statistically significant
> improvement over the baseline, or when the overhead of the substrate
> demonstrably exceeds the value it provides.*

This preserves the intellectual honesty of the kill conditions while
acknowledging that the most probable outcome is a spectrum, not a binary.

---

## Summary Table

| # | Concern | Vision Section Affected | Action Type |
| --- | --------- | ------------------------ | ------------- |
| 1 | H-1 de-risking must precede Phase 1 construction | §6 Progression Model | Modify vision + add backlog item |
| 2 | Agent identity/trust is a foundational invariant | §3 Boundary Contracts, §5 Invariants | Add invariant I-7 + boundary clause |
| 3 | Kill conditions need graduated response model | §7 Falsification Criteria | Add graduated response clause |
