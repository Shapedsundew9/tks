# Stakeholder Response — Iteration 4

---

## Summary Disposition

The vision is structurally mature, disciplined in scope, and honest about
its risks. The phased roadmap with dogfooding gates, the graduated
response framework, and the kill conditions all reflect the kind of
intellectual honesty that gives a constrained-resource initiative a real
chance of succeeding. I remain a committed supporter of the direction.

Two concerns below rise to the level of significant feedback because they
affect the **validity of the problem framing** and the
**meaningfulness of the foundational hypothesis**, respectively. Neither
requires tearing anything down — both can be addressed by tightening
existing sections.

---

## Concern 1: The Competitive Positioning in §1 Argues Against the Weakest Opponent

**Location:** §1, "The Build-vs-Leverage Imperative"

### The Gap

The fourth failure-mode subsection explicitly positions TKS against
legacy ALM platforms (IBM DOORS Next, Jama Connect, Siemens Polarion),
arguing they treat traceability as "a retrospective reporting and
compliance concern." That argument is correct but incomplete. It defeats
the weakest competitor in the room while ignoring the strongest.

By late 2026, the most relevant competitive pressure comes from the
**AI-native development tool ecosystem** — not legacy ALM:

- **AI-native IDEs** (Cursor, Windsurf/Devin Desktop) embed agentic
  context assembly directly into the editing loop, using AST analysis,
  type-system navigation, and multi-tool agentic retrieval (ripgrep,
  ast-grep, LSP) to assemble context dynamically without a separate
  knowledge substrate.
- **Terminal-first agents** (Claude Code, OpenAI Codex CLI) perform
  agentic retrieval loops — iteratively reading files, grepping symbols,
  and exploring dependency graphs — rather than relying on flat vector
  search.
- **MCP-native integrations** are already bridging agents to ticketing
  systems, ADRs, and documentation stores, providing ad-hoc traceability
  without a purpose-built graph engine.

These tools are attacking the same three failure modes (context collapse,
specification drift, unverifiable output) from a "thin orchestration
layer over existing code artifacts" direction. The vision's problem
framing implicitly assumes the baseline is "flat semantic search over raw
codebases" (§1, first paragraph), but the industry baseline has moved
beyond that.

### Why This Is Vision-Level

The "Build-vs-Leverage Imperative" is part of the problem definition, not
strategy. If the problem definition only argues against legacy tools, it
cannot survive contact with the obvious rebuttal: *"We already have
Cursor + Claude Code + MCP. Why do we need a separate knowledge
substrate?"*

### What I Would Add

The vision's genuine, defensible differentiation is the **intent and
requirement layer** — the "why" behind the code. AI-native IDEs
can tell an agent what the code *does* (via AST/types/tests); they
cannot tell an agent what the code is *supposed to do*, why a
design decision was made, or which upstream business constraint governs
a module. No amount of code-level context assembly reconstructs
the requirement provenance that TKS provides.

**Recommended action:** Revise §1's "Build-vs-Leverage Imperative" to
acknowledge AI-native development tools as the primary contemporary
competition, then explicitly articulate TKS's differentiation as the
**requirement-intent provenance layer** that code-level tooling
structurally cannot provide. This strengthens the problem framing by
showing the vision survives the strongest objection, not just the
weakest.

---

## Concern 2: Hypothesis H-1's Baseline Is a Strawman

**Location:** §5, Hypothesis H-1; Backlog §5, Spike 0

### The Gap

H-1 states:

> *"We hypothesize that supplying agents with graph-bounded context
> envelopes [...] significantly reduces downstream architectural contract
> violations compared to standard top-k flat semantic vector retrieval."*

And Spike 0's Condition A (baseline) is defined as:

> *"Standard top-k semantic vector similarity over chunked specification
> text."*

In 2026, **flat top-k vector similarity is not how competent agents
retrieve context**. Modern agents use multi-step agentic retrieval
combining lexical search, AST-guided structural analysis, type-system
navigation, and iterative file exploration. Benchmarks (GraphRAG-Bench,
practitioner consensus from agentic coding tool evaluations) consistently
show that hybrid agentic retrieval significantly outperforms naive vector
search.

If H-1 is validated against a strawman baseline, a positive result proves
only that *structured retrieval beats the weakest available retrieval
method*. That is not a useful scientific signal and would provide false
confidence for the Phase 1 infrastructure investment.

### Why This Is Vision-Level

H-1 is declared as a "Strategic Hypothesis (Scientific Bet to De-Risk)"
in the vision document, and the entire program's go/no-go decision
depends on its validation. If the hypothesis is poorly formulated, the
kill conditions that depend on it (Kill #2, "Graph RAG Inefficacy
Falsification") are also weakened. The integrity of the falsification
framework requires that H-1 tests against a credible baseline.

### What I Would Change

**In the vision (§5, H-1):** Restate the comparison baseline as
"best-available agentic context assembly" rather than "standard top-k
flat semantic vector retrieval." The specific baseline configuration
(which tools, which retrieval strategies) belongs in the backlog's
Spike 0, but the vision-level hypothesis should not lock in a baseline
that is already obsolete.

**In the backlog (§5, Spike 0, Condition A):** Redefine the baseline
condition to represent a competent agentic retrieval setup — e.g., an
agent with access to file reading, grep, AST-based code search, and
standard semantic search, operating without graph-structured requirement
context. This tests whether the *requirement graph* adds value beyond
what code-level tooling already provides, which is the actual bet TKS is
making.

**Impact on decision thresholds:** If the baseline is strengthened,
the ≥40% violation reduction target for "strong directional greenlight"
may need recalibration. A smaller margin against a strong baseline is
more meaningful than a large margin against a strawman. Consider
whether 20-30% reduction against a competent baseline should qualify
for directional greenlight. This recalibration is a backlog concern
and should be tracked there.

---

## Items Considered and NOT Raised

For transparency, the following were evaluated and judged to not
constitute significant feedback:

| Topic | Disposition |
| --- | --- |
| **Node lifecycle states** (ACTIVE, SUPERSEDED, ARCHIVED) | Could improve context envelope precision over time, but the vision's JSONB extension mechanism and progressive layering approach provide an adequate strategic path. Suggest adding as a backlog consideration for Phase 2+, not a vision change. |
| **Semantic validation of autonomous mutations** | The governance model is structural, not semantic. This is a real limitation but the vision's progressive delegation model (AUTONOMOUS_ELABORATION only on low-risk nodes) adequately scopes the risk. Not a vision gap. |
| **MCP protocol maturity risk** | Real but mitigated by the REST fallback. The vision's agent-agnostic claim is reasonable given the dual-protocol approach. |
| **Git-only document ingestion** | Intentional and documented scope boundary. Expanding to Confluence/Notion/etc. is a Phase 3+ concern at most. |

---

## Recommended Backlog Additions

If the vision changes above are accepted, the following backlog items
should be tracked:

1. **Spike 0 baseline redesign:** Redefine Condition A to represent
   competent multi-tool agentic retrieval (file reading + grep +
   AST search + semantic search, without graph-structured requirements).
   Recalibrate CAL-H1 decision thresholds against the stronger baseline.
2. **Node lifecycle management (Phase 2+ consideration):** Evaluate
   adding lifecycle state attributes (ACTIVE, SUPERSEDED, ARCHIVED) to
   prevent context envelope degradation from graph growth over time.
