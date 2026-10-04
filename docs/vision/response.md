# Stakeholder Response: Vision Alignment Critique

**Reviewer role:** Stakeholder sub-agent (committed ally, constructive skeptic)
**Inputs reviewed:** `docs/vision/vision.md`, `docs/vision/strategic-planning-backlog.md`
**Date:** 2026-10-04

I support the direction. The core idea is sound and worth building: an authoritative, version-governed source of intent that agents can query, with mechanical work done first and LLMs used sparingly. The feedback below covers only points that would change the vision, add or reprioritize a backlog item, or need a real rebuttal. I have kept tactical detail out on purpose. Where a point touches the backlog, I flag it for the tactical-planning step rather than solving it here.

Provenance key: **[DOC]** = a claim made in the supplied documents; **[EXT]** = an outside finding, with a source; **[ASSESS]** = my own judgment.

---

## Priority Summary

| # | Issue | Type | Ask |
| --- | --- | --- | --- |
| 1 | Some gate statuses disagree with the Phase 4 experiment's evidence | Vision integrity | Re-baseline gate status; add a hypothesis evidence ledger |
| 2 | The economic thesis (H-2 / Kill Condition 1) is unmeasured, and the vision keeps adding new review queues | Vision success criteria | Add a steady-state curation-burden observable and kill threshold; schedule H-2 evidence earlier |
| 3 | The competitive framing leaves out spec-driven development tooling | Problem definition / build-vs-leverage | Add this category to §1; restate TKS's differentiator against it |
| 4 | Baselines for the kill conditions are inconsistent; Kill Condition 5 uses a weak comparator | Falsification criteria | Make one strong baseline apply to every kill condition |
| 5 | The Feature Extension Benchmark gates Phase 5 in the vision but sits in Phase N+ in the backlog | Vision vs. backlog contradiction | Pick one; schedule the evidence the kill condition depends on |
| 6 | The vision is pinned to specific model products (one already retired) and a fixed "10x–20x" ratio | Durability of the vision | State model tiers by relative cost and capability, not by product name |
| 7 | The vision statement promises "bidirectional coherence" with code, but nothing detects drift coming from the code side | North Star overreach | Narrow the claim, or name the drift signal as a principle |
| 8 | No trust boundary for content: ingested text becomes "authoritative" context for agents | Missing foundational principle | Add a content-trust and prompt-injection principle |
| 9 | Some invariants can't be enforced mechanically (I-1 procedural grounding, testing policy) | Invariant hygiene | Keep invariants mechanical; move semantic obligations to governance or advisory |
| 10 | Scope creep and a principle conflict (doc generation vs. "no ghostwriting"); no Non-Goals section | Scope discipline | Add Non-Goals; move or drop capabilities 12–14 and the extra §6 POCs |
| 11 | No adopter or success criterion beyond self-dogfooding | Success criteria | Add an external-adoption criterion |

---

## 1. Gate status disagrees with the evidence (highest priority)

**[DOC]** The vision marks Milestone 1 "Completed," including "semantic milestone heading classification (`REQUIREMENT`)" and "verified protocol conformance." It marks Gate 2 "Autonomous Self-Evolution" as passed. Kill Condition 4 says the program is falsified if the team "cannot dogfood the Knowledge Substrate to manage its own post-Phase-1 development."

**[DOC]** The backlog's own post-Phase-3 experiment found:

- Zero `REQUIREMENT` nodes were produced by decomposition.
- The MCP schema serialization was non-conformant, so the agent harness rejected TKS calls.
- The agent fell back to raw SQL and edited the database directly.

**[ASSESS]** These cannot all be true together. Either Milestone 1 and Gate 2 were met under weaker conditions than the vision describes, or the gates were not met. This matters at the vision level:

- The vision's credibility depends on the falsification framework being honest. If gates are marked "Completed" when the evidence says otherwise, every later gate claim loses value.
- The same pattern shows up in the metrics table **[DOC]**. CAL-H2 ("Phase 3 User Study"), CAL-H3 ("Phase 3 Stress Test"), CAL-H4 ("Phase 1 Ingestion Eval"), and CAL-TTFV ("Phase 1 Benchmark") are tied to phases marked complete, but no results are recorded. Only SLA-1 and SLA-2 say "Validated."

**Asks:**

- **Vision change:** Add a short *Evidence Status* table to §5 or §7. For each hypothesis H-1..H-7, record one of: Untested / Partial / Validated / Refuted, with a link to the artifact. Gates count as "passed" only against their stated criteria.
- **Vision change:** Restate Milestone 1 and Gate 2 as "passed with known defects, re-verified in Phase 4," or similar wording. Don't silently carry the "Completed" label forward.
- **Backlog item (add, high priority):** A re-verification pass for Gates 1–2 against their written criteria once the Phase 4 fixes land. Also record H-3 and H-4 results, or explicitly mark them untested.

## 2. The economic thesis is unmeasured, and supervisory load keeps growing

**[DOC]** Kill Condition 1 (ingestion friction) and H-2 (sublinear oversight) together form the program's economic premise. Kill Condition 1 measures effort only "from zero structured intent to the first usable context envelope." CAL-TTFV measures only the first value delivered.

**[ASSESS]** For a living knowledge substrate, the bigger risk is the ongoing cost of keeping the graph current, not the cold start. Counting the human review queues the vision now creates:

1. Ingestion staging approval (Tier 4).
2. Delta-reconciliation review on every document revision.
3. `NEEDS_REVERIFICATION` cascades.
4. `relationship_review_backlog` triage.
5. Governance-profile digest work packages.
6. Remediation-envelope approvals that need authorization (for example, ancestor promotion).
7. Tier 3 escalation decisions.

Each is defensible on its own. Together, they may simply move the "Human Supervisory Exhaustion" failure mode (§1, F3) from code diffs to graph queues. The vision names this risk itself ("cannot be resolved by simply displacing cognitive fatigue…") but measures none of it.

**Asks:**

- **Vision change:** Add a **Steady-State Curation Burden** observable to §7: human-minutes per week spent maintaining the graph, per active contributor or per merged change. Extend Kill Condition 1 to cover steady state, not only cold start.
- **Vision change:** Add a principle that every new human review queue must justify its supervisory cost against H-2. A queue with no measured yield gets merged into another or dropped.
- **Backlog item (reprioritize):** Pull H-2 / CAL-H2 evidence collection forward, so it runs no later than Phase 4 alongside closed-loop traceability. It is the hypothesis most likely to kill the program, and nothing is collecting evidence for it yet.

## 3. The competitive framing leaves out the closest alternative

**[DOC]** §1 "Build-vs-Leverage Imperative" frames the market as two extremes: thin code orchestration (Cursor, Claude Code…) and retrospective ALM (DOORS, Jama…). TKS is placed "in the critical gap between these two extremes."

**[EXT]** Since 2025, a third category has grown that targets this same gap: **spec-driven development (SDD)** tooling.

- GitHub's open-source **Spec Kit** structures agent work as Specify → Plan → Tasks → Implement, using living Markdown specs as the shared source of truth. It is agent-agnostic (Copilot, Claude Code, Gemini CLI). Source: <https://github.com/github/spec-kit>.
- AWS **Kiro** puts specs (requirements → design → tasks), persistent **steering** files for standards and conventions, and event-driven **hooks** for validation inside the agent loop. Source: <https://kiro.dev/docs/steering/>.
- Repository-level agent instruction files (`AGENTS.md`, `CLAUDE.md`, Cursor rules) are the cheapest off-the-shelf baseline for "institutional memory for the agent."

**[ASSESS]** These tools already address much of F2 (spec drift), F5 (planning desert), and part of F4 (procedures, via steering and hooks), at near-zero adoption cost. They do **not** provide what TKS uniquely offers:

- A transactionally governed graph with relational integrity.
- Per-node authorization.
- Append-only audit and point-in-time reconstruction.
- Invalidation cascades across topology.
- Commit- and test-level traceability that can be queried.

That list is the real differentiator, and the vision should say so plainly. Without this category in the framing, the build-vs-leverage analysis is incomplete, and Hypothesis H-1 is benchmarked against the wrong competitor.

**Asks:**

- **Vision change:** Add SDD tooling as a third paradigm in §1. State what TKS does that file-based spec workflows structurally cannot (governance, integrity, audit, cascade, queryable traceability). State what TKS should *leverage rather than rebuild* (for example, consuming Spec Kit or Kiro-style spec artifacts through Brownfield Intent Scaffolding instead of competing on authoring UX).
- **Vision change:** Update H-1 and Kill Condition 2 so the baseline includes "a competent agent with the same specs available as plain files plus repo instruction files" (see §4 below).
- **Backlog item (add):** Brownfield adapters for SDD artifacts (Spec Kit / Kiro spec directories, `AGENTS.md`). This is cheap and directly lowers CAL-TTFV.

## 4. Kill-condition baselines are inconsistent, and one is weak

**[DOC]**

- H-1 and Kill Condition 2 compare against "best-available multi-tool retrieval or auto-generated code graphs." That is a strong baseline.
- Kill Condition 5 compares planning dossiers against "unguided single-node prompt envelopes," which is TKS's own earlier, admittedly deficient output.
- CAL-DOWN-1 compares a guided utility model against an "unguided frontier model."
- The Gate 4 protocol forbids the agent from reading `docs/vision/`.

**[ASSESS]** Kill Condition 5 as written can't fail in practice. Beating TKS's own 3-bullet summary isn't evidence of value. The question an adopter actually asks is: *"Does `get_elaboration_context` beat handing the same agent the source Markdown plus the repo?"* Forbidding `docs/vision/` access removes exactly that comparator.

**Asks:**

- **Vision change:** Define one **canonical baseline** that applies to every kill condition: the same model, the same task, and the same underlying knowledge available as plain files (specs + repo + instruction files) through a competent off-the-shelf harness. TKS has to beat *that*, not an impoverished version of itself.
- **Vision change:** Run H-6 and CAL-DOWN-1 as a 2×2 design (guided/unguided × utility/frontier), so the contribution of TKS is separated from the contribution of the model.

## 5. Vision and backlog disagree on when the Feature Extension Benchmark runs

**[DOC]**

- The vision places the Real-World Feature Extension Benchmark at "Phase 5 Gate 4" (Capability 6, H-6, Kill Condition 5, Milestone 4).
- CAL-DOWN-1 says "Phase 5 Evaluation."
- The backlog's Phase 5 MVD contains only task *elaboration* on TKS's own Phase 6. It has no feature implementation in an external codebase. The backlog lists the benchmark under **Phase N+**.

**[ASSESS]** As a result, the kill condition that tests the cognitive-rails thesis depends on evidence that isn't scheduled. Gate 4 is also fully self-referential: the agent plans TKS's own roadmap from a corpus written by the TKS authors in a structure TKS was tuned to parse. That risks overfitting.

**Asks:**

- **Vision change:** Pick one position. Either the benchmark gates Phase 5, or Kill Condition 5 is restated against what Phase 5 actually measures (Gate 4 elaboration quality versus the canonical baseline from §4).
- **Vision change:** Require at least one benchmark on a codebase and spec corpus **not authored by the TKS team**, to guard against self-referential overfitting.
- **Backlog item (reprioritize):** Move a minimal external Feature Extension trial (one codebase, a few features) into Phase 5 if it remains a kill-condition input.

## 6. Pinning the vision to model products makes it fragile

**[DOC]** The vision names "Claude 3.5 Haiku, Gemini Flash, GPT-4o-mini" and a fixed "10x–20x cost reduction" in Capability 6, H-6, Kill Condition 5, the Model Downgrading observable, and the glossary.

**[EXT]** Anthropic retired Claude 3.5 Haiku (`claude-3-5-haiku-20241022`) from its API on 2026-02-19. Requests to that model ID now fail. Sources: Anthropic model deprecations page, <https://docs.claude.com/en/docs/about-claude/model-deprecations>; <https://endoflife.date/claude>.

**[ASSESS]** A vision should outlast model release cycles. Two problems:

- The named models are already partly unavailable.
- The cost ratio between frontier and utility tiers moves with every pricing change. The H-6 thesis could be "won" or "lost" for reasons unrelated to TKS. If frontier prices fall, the arbitrage disappears even if the rails work perfectly.

**Asks:**

- **Vision change:** Define tiers abstractly. For example: "a utility-tier model at ≤ 1/N the per-task cost of the contemporaneous frontier model." Leave specific model IDs and the value of N to the backlog, re-picked at each evaluation.
- **Vision change:** Restate H-6's value claim as *guided vs. unguided at the same tier*, measured in task success and cost per successful task. Treat model arbitrage as a secondary benefit, not the thesis.

## 7. "Bidirectional coherence" with code isn't backed by any mechanism

**[DOC]** The Core Vision Statement promises "end-to-end traceability and **bidirectional coherence** across … executable code." The boundary contracts also rule out code-level analysis: "avoiding speculative reverse-engineering of raw code syntax."

**[ASSESS]** Every coherence mechanism described runs intent → code: task links, commit links, `VERIFIED_BY`. Nothing detects code changing out from under requirements, such as:

- Commits made outside TKS-tracked tasks.
- Refactors that silently break an invariant.
- Tests that are deleted.

That is the "Specification Drift Trap" (§1, F2) seen from the other side, and the vision currently has no answer for it. Choose one of two honest positions:

- **(a) Narrow the claim:** "end-to-end traceability, with coherence maintained from intent to code," and list code-originated drift as a known limitation or non-goal; or
- **(b) Name the drift signal as a principle**, without reverse-engineering code. For example: "Code-side drift is detected through verification evidence: failing, missing, or stale `VERIFIED_BY` links, and changes to linked paths in commits not attributed to any task."

**Ask:** **Vision change.** Adopt (a) or (b). I recommend (b): it stays mechanical, uses existing CI, and keeps the North Star statement honest.

## 8. Missing principle: content trust and prompt injection

**[DOC]** The metaphor makes the substrate "authoritative." Agents consult it as the organization's playbook. Governance profiles, SOPs, and ingested documents flow straight into agent context envelopes. Invariant I-7 covers *identity* attribution, but there is no principle about *content* trust.

**[EXT]** Indirect prompt injection and MCP "tool poisoning" are well-documented risks. Instructions embedded in retrieved content or tool metadata get treated as authoritative by agents. Sources: OWASP Top 10 for LLM Applications, LLM01 Prompt Injection, <https://genai.owasp.org/llmrisk/llm01-prompt-injection/>; Invariant Labs, MCP tool-poisoning disclosure, <https://invariantlabs.ai/blog/mcp-security-notification-tool-poisoning-attacks>.

**[ASSESS]** A substrate built to make content *more* authoritative to agents raises the impact of any poisoned node. Today that could be a malicious ADR in a brownfield repo, an imported governance pack, or a compromised agent's draft that gets rubber-stamped. This belongs in the vision's foundational principles because it shapes trust tiers and the ingestion boundary, not just implementation.

**Asks:**

- **Vision change:** Add a boundary contract: *"Ingested content is data, never instructions. Context envelopes must mark provenance and trust level (human-approved / agent-drafted / externally imported). Content from untrusted or imported sources cannot reach authoritative status without human approval."*
- **Backlog item (add):** A threat-model spike covering injection via ingested documents and imported governance profiles.

## 9. Some invariants can't be enforced mechanically

**[DOC]** Invariant I-1 requires that tasks "involving institutional procedures (e.g., introducing third-party dependencies…)" carry edges to procedure nodes, and that "ungoverned procedural actions are rejected at the database constraint level." I-1 also embeds a testing policy ("unit tests for tactical choices, integration tests for architectural decisions").

**[ASSESS]** Deciding whether a task "involves introducing a dependency" is a semantic judgment, not a relational constraint. A database cannot reject what it cannot detect. This conflicts with the vision's own principle that structural verification is "enforced entirely by deterministic relational constraints." Embedding a testing policy in an invariant also makes a governance choice look like a structural law.

**Ask:** **Vision change.** Keep invariants strictly mechanical:

- I-1 stays as orphan-path rejection only.
- Procedural grounding becomes a governance-profile rule. It is enforced mechanically where a trigger is mechanically detectable (for example, a changed `Cargo.toml` or `package.json` in a linked commit) and is advisory everywhere else.
- Level-appropriate testing moves to Capability 14 / governance profiles.

## 10. Scope creep, a principle conflict, and no Non-Goals section

**[DOC]** The vision lists 14 Key Capabilities. §6 adds four POCs not in the backlog: Open-Source Tool Reproduction, Compiler / Safety-Critical Formal Standards, and Dual Mode A/B. Capability 12 (Reverse Document Projection) "orchestrat[es] an LLM to assemble graph nodes … into cohesive Markdown."

**[ASSESS]**

- **Principle conflict:** Capability 12 is generative ghostwriting by definition. That directly contradicts "Analytical Classification vs. Generative Ghostwriting" and sits awkwardly with I-3. If kept, it should be framed as *an external agent's use of TKS through MCP*, not a TKS capability. Another option is a deterministic template projection with no LLM involved, which is cheap and mechanical.
- **Scope creep:** Capabilities 12–14 and the extra §6 POCs dilute focus under the vision's own "Resource-Constrained Execution Model." Capability 14 (NASA/INCOSE recursive standards) and the compiler/safety-critical POC point toward the regulated-ALM market the vision explicitly says it is *not* targeting.
- **Missing section:** The vision has boundaries but no explicit **Non-Goals** list. With a solo or small team, saying "no" in writing is the most effective scope control available.

**Asks:**

- **Vision change:** Add a **Non-Goals** section. Candidates:
  - Authoring UX that competes with SDD tools.
  - Code-level reverse-engineering.
  - Regulated-industry ALM certification.
  - Generative doc authoring inside TKS.
  - Multi-tenant enterprise SaaS before external adoption is proven.
- **Vision change:** Move capabilities 12–14 and the three extra §6 POCs to a clearly labeled "Long-Horizon Options (non-committal)" appendix, or drop them. Keep the core focused on: governed graph, ingestion, context and planning rails, closed-loop traceability, and governance profiles.

## 11. No success criterion beyond self-dogfooding

**[DOC]** All gates and kill conditions are measured on TKS managing itself. Kill Condition 4 is only about the TKS team's ability to dogfood. Governance profiles target "large multi-project repositories."

**[ASSESS]** Self-hosting is necessary but not sufficient, and it is biased: the authors know the corpus, the schema, and the workarounds. The vision doesn't say who the first non-author adopter is (a solo developer? a small team? a platform group?), or what "worth it" looks like for them. Without that, Kill Condition 1 has no realistic subject.

**Ask:** **Vision change.** Name the primary target adopter, and add a success criterion of the form "at least one project not authored by the TKS team adopts TKS and passes the steady-state curation-burden threshold from §2." Defer multi-project and enterprise governance claims until that is met.

---

## Explicitly Out of Scope for This Review

I noticed, but did not raise, tactical items in the backlog: lock hierarchy, Git actor concurrency, `tsvector` sanitization, webhook shapes, and MCP spec-version pinning. Those belong to the tactical-planning step. One note for that step: the backlog pins "MCP 2024-11-05 spec." Newer MCP revisions exist, so version targeting should be decided during planning, not fixed in the vision.
