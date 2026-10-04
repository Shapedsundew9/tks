# Stakeholder Response: Vision Alignment Critique

**Reviewer role:** Stakeholder sub-agent (committed ally, realist)
**Inputs reviewed:** `docs/vision/vision.md`, `docs/vision/strategic-planning-backlog.md`, and the Project Initiator's additional context (economic focus, open-source positioning, enterprise scale).
**Date:** 2026-10-04

I support the core thesis. A version-governed intent graph with invalidation cascades, provenance, and bounded agent context is new and worth building. Code-graph tools and file-based SDD do not provide it. The feedback below covers places where the vision **contradicts the initiator's stated intent**, **measures the wrong thing**, or **reimplements things that already exist**. Items are ordered by how much they would change the vision. Tactical suggestions are kept to short "backlog implication" notes so this round stays at the vision level.

---

## Priority Summary

| # | Issue | Severity | Changes Vision? |
| --- | --- | --- | --- |
| 1 | Target adopter and scale contradict the initiator's enterprise scope | Critical | Yes |
| 2 | Success criteria leave out long-horizon token effectiveness and overweight cost arbitrage | Critical | Yes |
| 3 | The vision constrains the implementation layers the initiator wants left free ("governance altitude") | Critical | Yes |
| 4 | Benchmarks and kill conditions are set at a scale where TKS isn't meant to win (risk of false falsification) | High | Yes |
| 5 | Human approval for every node won't scale to millions of LOC | High | Yes |
| 6 | CRA positioning contradicts itself; supply-chain checks duplicate off-the-shelf tools | High | Yes |
| 7 | Open-source positioning is missing; commercial "arbitrage" language should go; enterprise-trust prerequisites are unstated | Medium | Yes |
| 8 | The vision document holds strategy, defect status, and protocol trivia | Medium | Yes (structural) |

---

## 1. Target Adopter & Scale Contradict the Initiator's Enterprise Scope

**What the documents say:** The Target Adopter Profile (§2) names "a small engineering team or high-velocity solo developer" as the primary adopter. Enterprise and multi-project governance are only reachable after "demonstrated viability" with that group. The non-goal "No Premature Multi-Tenant Enterprise SaaS" repeats the small-team, local-workflow focus.

**The conflict:** The initiator says TKS works at a "fundamentally different scale" from Spec Kit and `AGENTS.md`: millions of lines of code, EU CRA compliance, and governance of whole processes, procedures, and policies across large domains. As written, the vision targets the market where Spec Kit and `AGENTS.md` are already good enough. It postpones the market where the initiator says TKS is needed.

**Why it matters:** This changes the destination. Every downstream decision depends on whether the *project* being governed is large or small. That includes benchmark choice, kill thresholds, curation economics, and ingestion design.

**Recommendation (vision change):** Separate **project scale** from **team size**. With agents, a small team can run a very large codebase, and that may be the main use case. Suggested reframing:

> *Primary target: engineering efforts of large scale and long duration (large codebases, many interacting components, regulatory or institutional process obligations), operated by teams of any size that rely on AI agents for most implementation work. Small projects are supported but are not where TKS must show it is better than file-based approaches.*

Keep the "no multi-tenant SaaS" non-goal (it is about hosting, not scale). Rewrite its rationale so it no longer says the product is meant for small teams.

---

## 2. Success Criteria Leave Out Long-Horizon Token Effectiveness and Overweight Cost Arbitrage

**What the documents say:** Model downgrading is framed as **economic arbitrage** in many places: Key Capability 6, H-6, the Conceptual Grounding table, the Inside-Out/Outside-In table ("Vendor Decoupling & Model Arbitrage", "Commercial Model Arbitrage"), Kill Condition 1 ("core economic premise"), and CAL-DOWN-1/2. The main benchmark is a **single feature-extension trial**.

**The conflict:** The initiator says economic efficiency "should not be over-emphasized." The primary requirement is **effective token use on large projects over extended horizons**, achieved by keeping agents focused on their tasks and by using lower-cost agents for implementation. No benchmark, observable, or kill condition in the vision measures behaviour over an *extended horizon*. Every evaluation is one-off.

**Why it matters:** A single feature trial can't show the property that most separates TKS from the alternatives. The claim is that intent stays coherent and context stays focused as the codebase and history grow. File-based context gets worse over time as files multiply and go stale. The graph is supposed not to. That difference only appears across many sequential changes.

**Recommendation (vision change):**

1. Make the North Star success criterion something like: **"Sustained agent focus and intent fidelity over a long sequence of changes on a large codebase, measured as tokens consumed per accepted, verified change and the rate at which contract violations accumulate, compared with the Canonical Agentic Baseline. Both should stay roughly flat as the project grows."**
2. Reframe H-6 around **focus and token effectiveness**. Treat "a cheaper model is enough" as *evidence* of focus, not the goal. Drop or downgrade "arbitrage" wording throughout.
3. Add a **Long-Horizon Benchmark** to the vision's milestone benchmarks, next to or in place of the single feature-extension trial as the primary benchmark: N sequential, interdependent changes over weeks on a large external codebase.
4. Reword Kill Condition 1 from "core economic premise" to the **net supervisory and token-effectiveness premise**.

**Backlog implication (deferred):** The design details (number of steps, codebase selection, token accounting method) belong in tactical planning.

---

## 3. "Governance Altitude": The Vision Constrains the Layers the Initiator Wants Left Free

**What the documents say:**

- **Invariant I-1** requires every functional specification, implementation task, *and code artifact reference* to trace to an active requirement, and rejects orphan tasks at the database level.
- **Tactical Decision Capture** (Capability 5, I-5) records "parameter ranges, CLI flags, types" into the graph.
- **Level-Appropriate Verification** requires "unit tests for tactical choices."
- **Code-side drift detection** flags changes to linked file paths in commits not attributed to an active task.

**The conflict:** The initiator says plainly that TKS is *not* meant to constrain lower implementation layers. It is meant to structure and guide the **higher architectural and design layers** through explicit requirements. The items above push governance down to individual commits, files, and CLI flags.

**Why it matters:** This is a basic boundary of the vision. At millions of LOC, governing below the design layer has three costs:

- It creates curation noise. Every tactical capture and every unattributed file change becomes something for someone to triage, which works against H-2 and the vision's own Supervisory Queue Economy principle.
- It wastes agent tokens on bookkeeping instead of the task.
- It makes TKS look like a bureaucracy layer, which hurts adoption.

**Recommendation (vision change):** Add an explicit **Governance Altitude principle**:

> *TKS models and governs intent down to the architecture and design boundary: requirements, decisions, interface and service contracts, invariants, and process obligations. Below that boundary, implementation is unconstrained. It is observed only through verification signals (tests, CI, commits) linked to the lowest governed node, and is never modelled node-by-node.*

Then:

- Narrow I-1 so that *tasks* must trace to governed intent, but code artifacts are not graph-modelled entities.
- Make Tactical Decision Capture **optional or dropped** from the core capabilities. If kept, restrict it to decisions that cross an interface or contract.
- Restate drift detection as **contract-level drift**: a failing or stale verification of a governed contract. Drop file-level attribution policing from the vision.
- Change "unit tests for tactical choices" so that verification policy attaches to governed contracts. Unit-test policy for code is left to the project's own conventions.

---

## 4. Benchmarks and Kill Conditions Sit at a Scale Where TKS Isn't Meant to Win

**What the documents say:** Kill Conditions 2 and 5 halt or restructure the program if TKS shows no statistically significant advantage over the Canonical Agentic Baseline. The evaluations planned for that comparison are a bounded feature trial and dogfooding on TKS itself, which is a small codebase.

**The conflict:** By the initiator's own framing, file-based approaches are *adequate* for small initiatives. On a small codebase, the baseline (plain specs plus `AGENTS.md`) may match TKS. That would trigger a kill condition even though the thesis is about large-scale work.

**Why it matters:** Today the falsification criteria test a claim the project doesn't make, and they don't test the claim it does make.

**Recommendation (vision change):**

- State in the vision that **H-1, H-2, and H-6 are claims about large-scale, long-horizon work**. Benchmarks used for kill decisions must run at representative scale: a large external codebase with a real body of specifications, ADRs, and policies.
- Add an honest **"where TKS should not be expected to win"** statement: small, short-lived, single-spec projects. This also strengthens the Build-vs-Leverage section.
- Kill Condition 4 (external adoption) should require the adopting project to be **representative of the target scale**, not just "non-trivial."
- Restate H-3 (single-engine scalability) in terms of the target workload, not just node count. Example: graph size and query mix for a project of millions of LOC with full process and policy coverage. 10⁶ nodes may or may not be the right proxy.

---

## 5. Human Approval for Every Node Won't Scale to Millions of LOC

**What the documents say:** Tier 4 human review is required before candidates become `ACTIVE`. The Content Trust Boundary says brownfield or externally imported content "cannot attain authoritative status ... without explicit human supervisory approval."

**The conflict:** Brownfield scaffolding of an enterprise estate (hundreds of OpenAPI/Protobuf contracts, years of ADRs, BDD suites) produces tens of thousands of candidate nodes. Narrative-ordered batching helps, but it doesn't change the order of magnitude. This is the cold-start barrier the vision is trying to remove, now moved into the staging queue.

**Unstated assumption:** Many of these artifacts have *already been through human governance*. An OpenAPI contract merged to `main` through a reviewed PR, or an ADR with "Accepted" status, has had human approval. Reviewing it again in TKS duplicates work that was already done.

**Recommendation (vision change):** Add an **authority-inheritance principle** to the trust boundary:

> *Content can inherit authority from the governance of its source (for example, merged and reviewed contracts on a protected branch, or ADRs with accepted status). Human review is required in proportion to risk and novelty: items that are new, ambiguous, contradictory, or come from untrusted sources. It is not required uniformly for every item.*

This keeps the prompt-injection defence for untrusted content. It makes H-2 plausible at enterprise scale and directly supports CAL-TTFV.

---

## 6. CRA Positioning Contradicts Itself; Supply-Chain Checks Duplicate Off-the-Shelf Tools

**What the documents say:** EU CRA compliance is named in governance profiles (Capability 8, H-7). The non-goal "No Regulated-Industry ALM Compliance Certification" dismisses "bureaucratic compliance sign-off" in favour of "operational agentic agility." The dependency-onboarding profile describes TKS evaluating licensing, commit frequency, maintenance health, and CVE history.

**The conflict:** The initiator names CRA compliance as a defining enterprise-scale need. CRA is evidence-heavy by design. Reporting obligations for actively exploited vulnerabilities apply from **11 September 2026**, i.e. now. Full application, including Annex I vulnerability-handling requirements (a machine-readable SBOM covering at least top-level dependencies) and technical documentation for conformity assessment, applies from **11 December 2027** ([Regulation (EU) 2024/2847, EUR-Lex](https://eur-lex.europa.eu/eli/reg/2024/2847/oj)). The vision can't both champion CRA and treat compliance work as bureaucracy to avoid.

**Recommendation (vision change):**

1. Reword the non-goal: TKS **does not certify** compliance or replace conformity assessment. It **is** a system of record for **traceable compliance evidence**: requirement → decision → contract → verification → release, linked to process obligations. This is a core, distinctive value, not a side effect.
2. Add a **"link evidence, don't reimplement checks"** principle to Build-vs-Leverage. Mature, maintained tools already cover dependency health, vulnerability data, SBOM generation, and licence policy, for example [OpenSSF Scorecard](https://github.com/ossf/scorecard) (maintenance and security posture), [OSV](https://osv.dev) (vulnerability data), [CycloneDX](https://cyclonedx.org) / [SPDX](https://spdx.dev) (SBOM formats), and ecosystem licence/advisory checkers such as `cargo-deny`. These are external findings, not claims from the supplied documents. TKS's novelty is **binding those tools' outputs to governed intent and process obligations**, not rebuilding the checks. The vision already applies this logic to markdown and syntax linting. Extend it explicitly to supply-chain and security checks.

---

## 7. Open-Source Positioning Is Missing; Enterprise-Trust Prerequisites Are Unstated

**What the documents say:** The vision reads in places like a venture pitch: "Strategic & Economic Leverage," "Commercial Model Arbitrage," "the enterprise can adopt...". It never says TKS is non-monetized open source.

**The conflict:** The initiator says there is no commercial goal. TKS is open source, for anyone who finds it useful.

**Why it matters:** Open-source status changes what success means (adoption and outside contribution, not revenue). It also creates **unstated dependencies** that enterprise adopters will check before trusting a substrate with their governance data:

- **Interface stability:** versioned, backward-compatible MCP/REST contracts and graph schema. Adopters' agents and CI integrations depend on these.
- **Data portability and exit guarantee:** the graph must be exportable in an open, documented format. An authoritative system of record with no way out won't be adopted at enterprise scale.
- **Coexistence with existing systems of record:** large organizations already run ALM, requirements tools, and issue trackers. The vision treats legacy ALM as a competing paradigm. At enterprise scale TKS will more often need to **interoperate** with these tools than replace them. Open interchange standards exist, such as [OMG ReqIF](https://www.omg.org/spec/ReqIF/) and [OASIS OSLC](https://open-services.net/) (external finding).

**Recommendation (vision change):**

- Add a short **Open-Source Positioning** statement: non-commercial, adoption-driven, and success measured by real-world use at target scale.
- Retitle the Inside-Out/Outside-In table's right column away from "economic leverage" and remove the "arbitrage" language.
- Add **interface stability, data portability, and interoperability (not displacement)** as foundational principles.

**Backlog implication (deferred):** Choosing interchange formats and adapters is tactical. Don't plan it now.

---

## 8. The Vision Document Holds Strategy, Defect Status, and Protocol Trivia

**What the documents say:** The vision includes:

- phase numbers and gate statuses ("Provisional / Remediating in Phase 4");
- specific defect histories (snake_case `input_schema`);
- a protocol serialization detail promoted to **Key Capability 3** and a **Boundary Contract** ("camelCase `inputSchema`");
- CLI command names (`tks graph audit`);
- a pinned "MCP 2024-11-05" reference (in the backlog);
- hypothesis-ledger statuses.

**Why it matters:**

- **Durability:** a vision that changes with every sprint is no longer a stable reference to align against. MCP alone has had several dated revisions since 2024-11-05, including 2025-06-18 and 2025-11-25 ([MCP specification](https://modelcontextprotocol.io/specification)). Pinning protocol details in the vision guarantees it goes stale.
- **Scope defence:** conformance to an open protocol is a *principle*. Getting one field's casing right is a *bug fix*. Listing the latter as a core capability weakens the list.
- **Self-consistency:** the vision is about 90 KB of Markdown and the backlog about 60 KB, which is the "governance in large Markdown files" burden the initiator describes. That suggests the vision should be short and stable, with detail living in the substrate and the backlog.

**Recommendation (vision change):** Strip phase status, defect narratives, gate progress, command names, and protocol field details out of the vision and into the backlog (or into TKS itself). Keep Capability 3 as "Open-protocol, agent-agnostic integration with strict standards conformance." Keep the Hypothesis Evidence Ledger as a separate, living artifact that the vision links to.

**Backlog implication (deferred):** The casing defect comes from implementing the protocol by hand. When tactics are planned, consider a "use the official SDK" leverage decision, e.g. the official [MCP Rust SDK](https://github.com/modelcontextprotocol/rust-sdk) (external finding). This is noted for later, not for this round.

---

## Items Considered and Deliberately Not Raised

- **Single-engine PostgreSQL, Git document ledger, and the mechanical-first / LLM-as-classifier philosophy:** sound, and consistent with both token discipline and the initiator's intent. No change.
- **Self-referential bootstrapping:** worth keeping as a dogfooding practice. The overfitting risk is already handled by the external-corpus mandate. Item 4's scale requirement covers what's left.
- **Detailed tiering, locking, and schema choices in the backlog:** tactical. Out of scope for this round.
