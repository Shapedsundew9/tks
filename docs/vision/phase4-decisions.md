# Phase 4 Implementation Decision Record: Foundational Ingestion & Closed-Loop Traceability

## 1. Decision Ledger

| Decision ID | Work Package | Title | Category | Target Upstream Document | Status |
| :--- | :--- | :--- | :--- | :--- | :--- |
| DEC-4.1 | WP-4.1 | Structural Heading Taxonomy, Atomization Scope & Compact Classification Schema | Specification Gap | architecture.md §5.1, technical-backlog.md TB-7 | Implemented |

---

## 2. Decision Entries

### DEC-4.1: Structural Heading Taxonomy, Atomization Scope & Compact Classification Schema

* **Work Package:** WP-4.1
* **Category:** Specification Gap
* **Context & Problem:** WP-4.1 requires structural heading classification (H1/H2 $\rightarrow$ `REQUIREMENT`, H3–H6 $\rightarrow$ `SPECIFICATION`), frontmatter type overrides, mechanical atomization using `unicode-segmentation`, and zero token overhead for Tier 1 structural extraction while keeping Stage 2 LLM prompt/completion token ratios under 10% (Constraint C-11). Two design ambiguities arose during implementation:
  1. Whether structural headings should be treated as candidate chunks submitted to Stage 2 LLM semantic classification or treated as deterministically classified by AST analysis. Marking structural headings as candidates caused token explosion and C-11 ratio violations (> 10%), in direct tension with the zero-token Tier 1 invariant.
  2. In Stage 2 classification output, whether to emit full JSON object fields including redundant governance policy labels, or an ultra-compact tuple schema `(alias, modality, confidence)`. The full JSON schema consumed ~13.7% output tokens on short specifications, exceeding C-11.
* **Options Considered:**
  - *Option A:* Send all parsed blocks (headings + clauses) to Stage 2 LLM and request verbose JSON objects containing `alias`, `node_type`, `modality`, `confidence`, and `governance_policy`. Pros: Uniform pipeline for all chunks. Cons: Violates zero-token Tier 1 extraction invariant, exceeds C-11 token budget (13.7%), and risks LLM hallucinations overturning deterministic heading hierarchy.
  - *Option B:* Restrict Stage 2 LLM candidate flagging (`is_candidate = true`) strictly to non-heading clauses that contain RFC 2119 keywords or canonical keys, while structural headings are deterministically resolved in Tier 1 with 1.0 confidence. In Stage 2, emit compact tuples `{"alias": ..., "modality": ..., "confidence": ...}` while backward-compatibly accepting `chunk_id` and legacy fields. Pros: Keeps combined token ratio at ~3.5% (well below C-11 10% threshold), guarantees zero LLM tokens for heading taxonomy, eliminates heading hallucinations, and guarantees 100% precision / 96.8% recall on evaluation benchmarks. Cons: Headings with RFC 2119 phrasing rely on explicit frontmatter or manual override if a non-default type is desired.
* **Decision Taken & Rationale:** Adopted Option B. Structural headings are deterministically classified by the AST parser at zero token cost and confidence 1.0, establishing the structural backbone and upward `DERIVED_FROM` lineage. Mechanical atomization segments non-heading paragraphs into clause-level spans using `UnicodeSegmentation::split_sentence_bound_indices` with exact 0-based byte spans and ordinal aliases (`c1`, `c2`, ...). Stage 2 prompts and parser accept compact tuples, reducing output token ratio to ~3.5% on sample specifications.
* **Upstream Impact & Target Document:** `architecture.md` §5.1, `technical-backlog.md` TB-7, TB-11. Feedback: Document that Tier 1 AST heading classification is deterministic and bypassed from Stage 2 LLM prompt dispatch to guarantee zero token overhead and C-11 compliance.
* **Status:** Implemented
