# Stakeholder Review & Response: Requirement Quality Assessment, Extraction Normalization, and Pipeline Sequencing

**To:** Project Initiator & System Architect  
**From:** Stakeholder Sub-Agent  
**Date:** October 2026  
**Subject:** Evaluation of Phase 4.6 Multi-Tier Ingestion Analysis, Requirement Atomization, and Contradiction Sequencing  

---

## 1. Executive Stance: Constructive Alignment & Practical Grounding

The Project Initiator’s observation cuts directly to one of the most critical real-world failure modes in automated requirements engineering: **the false equivalence between raw document text spans and discrete, testable requirement entities**.

In unstructured engineering prose, real-world authors routinely bundle multiple architectural constraints, operational considerations, and non-functional goals into dense, compound paragraphs. Expecting a mechanical CommonMark AST parser to extract text blocks that cleanly map 1:1 to atomic, verifiable requirement nodes is unrealistic. Furthermore, attempting to evaluate cross-graph contradiction risk against approved nodes *before* or *simultaneously with* decomposing and normalizing what an extracted passage actually asserts is a fundamental architectural inversion.

As a committed ally invested in the enterprise’s success, I **strongly endorse the core conceptual insight**:

1. A **Source Span** (immutable provenance anchor in Git) is physically and semantically distinct from a **Requirement Node** (a discrete, normalized engineering contract in the property graph). One span may spawn $1 \dots N$ atomic candidate requirements.
2. Semantic ingestion analysis must be decoupled into two sequential stages: **Intra-Node Extraction Quality & Normalization** (atomization, modality classification, and confidence scoring) must precede **Inter-Node Topological Contradiction & Relational Binding**.

However, to keep our initiative grounded, economically viable, and focused on its primary milestones, this insight must be introduced with strict guardrails to prevent scope creep, prohibitive token consumption, and supervisory exhaustion.

---

## 2. Vision-Level Implications (Destination & Foundational Principles)

The Initiator's proposal touches foundational vision definitions in `docs/vision/vision.md`. These require explicit calibration rather than open-ended expansion:

### 2.1 Decoupling Provenance Anchors from Requirement Entities (Invariant I-4 Fidelity)

* **The Insight:** Invariant I-4 mandates that every requirement derived via decomposition must maintain a persistent cryptographic reference (`doc_path`, `doc_hash`, `byte_start`, `byte_end`) to the original document artifact. This invariant must *not* be interpreted as requiring the requirement text to be a verbatim substring match of the document block.
* **The Principle:** The Git-backed document ledger stores the immutable, authoritative source text. The PostgreSQL property graph stores normalized, atomic requirement entities. A single document section or paragraph span may serve as the provenance anchor for multiple child requirement nodes (e.g., separating a UI latency mandate from a backend data-retention rule co-located in the same paragraph).
* **The Guardrail:** While the requirement entity's statement is normalized for clarity and testability, it must retain strict bidirectional traceability to its exact enclosing source span. Candidate nodes must preserve both the normalized statement and the verbatim source excerpt for human inspection.

### 2.2 Correcting the "Contradiction-First" Sequencing Inversion

* **The Insight:** `vision.md` (§2, Key Capability 2 and §4, Loop 1) currently states that candidate nodes are evaluated by a Quality Index that places *"Contradiction Risk against existing approved nodes first, followed by ambiguity and testability scores."*
* **The Reality:** This is logically unworkable. If an AST parser extracts an un-atomized, compound narrative paragraph, running vector similarity and contradiction prompts against approved graph nodes produces massive semantic noise and false positives. You cannot reliably determine if a passage contradicts existing invariants until you have isolated its discrete assertions.
* **The Correction:** The Vision must clarify the two-stage semantic progression:
  1. **Stage A (Intra-Node Extraction Quality & Normalization):** Assess atomicity, identify compound clauses, classify normative modality (`SHALL` vs. `SHOULD` vs. `MAY`), flag semantic ambiguity, and assign an extraction confidence score.
  2. **Stage B (Inter-Node Topological & Contradiction Risk):** Compare the isolated, normalized candidate assertions against the active approved graph topology to detect conflicts, redundancies, and layer-binding gaps.

### 2.3 Defending the "Minimal LLM Reliance" Philosophy (The Anti-Ghostwriter Directives)

* **The Threat:** The Initiator suggests that the LLM's role is to ask: *"Hey, can this be reworded as a shall with high probability or a should with high probability? Or should it be broken down into four requirements instead of one?"* While well-intentioned, this risks turning TKS into an expansive, generative "AI ghostwriter" that paraphrases user documents into synthetic NASA-style requirements.
* **The Core Philosophy:** `vision.md` explicitly mandates *minimal LLM reliance and maximum reliance on mechanical processes*, strictly minimizing costly generative output tokens. If an LLM rewrites large bodies of engineering documentation:
  * Generative hallucination will subtly alter author intent or invent constraints not present in the source text.
  * Token costs and latency on ingestion will surge.
  * Human engineers will suffer cognitive exhaustion trying to diff what they wrote against what the LLM invented.
* **The Principle:** The local or commercial LLM must function as an **analytical classifier and structured extractor**, not a creative rewriter. The model should emit compact structured metadata (identifying clause boundaries, suggested modal classifications, and confidence ratings) rather than generating ungrounded, synthetic prose.

---

## 3. Grounded Pragmatic Critique & Risk Mitigations

### 3.1 Local 8B Model Reality Check & The Asynchronous Boundary

The Initiator suggests using local LLMs (such as Ollama Qwen 8B) to perform this quality assessment, translation, and confidence ranking, followed by escalation to commercial models and humans.

* **Pragmatic Assessment:** Edge commodity 8B models frequently struggle with reliable JSON formatting, multi-clause linguistic decomposition, and subtle modal distinction on dense technical documentation. If Phase 4 makes local LLM decomposition a synchronous, mandatory prerequisite for document ingestion, it will introduce latency, brittle failure modes, and user friction.
* **Mitigation:**
  1. **Tier 1 Mechanical AST Extraction Remains Baseline:** The structural heading convention (promoting `#` and `##` structural containers to `REQUIREMENT` anchors at zero token cost) must remain fully functional and independently usable out-of-the-box.
  2. **Tier 2 Local Pre-Analysis Remains Asynchronous & Optional:** Local LLM extraction quality evaluation must run strictly *"if available and requested"*, enqueuing recommendations into the staging draft metadata. If no local LLM is configured, the system must stage the mechanically parsed sections cleanly without blocking.
  3. **Confidence-Gated Escalation:** The local model should output an explicit `extraction_confidence` score (0.0 to 1.0). High-confidence extractions are presented directly to the human reviewer; low-confidence extractions trigger an automated advisory recommendation for Tier 3 commercial escalation or targeted human intervention.

### 3.2 Guarding the Phase 4 Focus: Closed-Loop Traceability Must Not Be Derailed

Following Phase 3, the empirical dogfooding experiment established that TKS had four critical mechanical gaps: MCP camelCase serialization, context deserts, missing requirement taxonomy, and lack of relational edge pre-validation. Phase 4 was explicitly reprioritized to fix these foundational blockers and establish **closed-loop traceability from requirements to code commits and test runs**.

* **Scope Warning:** We must not allow Phase 4.6 (multi-tier ingestion analysis) to mushroom into an open-ended research program on automated NLP requirements engineering.
* **Boundary Defense:** In Phase 4, the mechanical 80/20 parsing, edge pre-validation, VCS commit linking, and pre-merge CI test-run mapping are the existential deliverables. The multi-tier extraction analysis should be delivered as a clean, bounded enhancement within Phase 4.6, rather than an expansive prerequisite that blocks Milestone 3 verification.

---

## 4. Incidental Capture for Strategic Planning Backlog

In keeping with our process boundaries, detailed implementation workflows belong in the Strategic Planning Backlog for upcoming tactical planning. The following items should be captured:

### 4.1 Structuring Deliverable 4.6 Sub-Bullets

Update Backlog Deliverable 4.6 ("Multi-Tier Ingestion Analysis & Narrative-Preserving Staging Review") to reflect the distinct operational stages:
* **4.6a: Baseline Mechanical AST Extraction & Scaffolding (Tier 1):** Zero-token structural decomposition (`pulldown-cmark`) mapping top-level containers and headings to initial candidate anchors.
* **4.6b: Extraction Quality Assessment & Requirement Atomization (Tier 2/3):** Optional local/commercial LLM pre-analysis analyzing candidate text blocks for atomicity, splitting compound clauses into discrete child candidate requirements, classifying modal intent (`SHALL` / `SHOULD` / `MAY`), and assigning an `extraction_confidence` score.
* **4.6c: Topological Relational Binding & Contradiction Risk (Tier 2/3):** Evaluating isolated, normalized candidate requirements against active approved graph nodes to detect semantic conflicts, duplicate intent, and missing cross-layer edges.
* **4.6d: Narrative-Preserving Staging & Supervisory Review (Tier 4):** Presenting candidate requirements in source document context, displaying both the verbatim source span and the normalized candidate statements, with inline visual badges for modality, confidence, and contradiction risk.

### 4.2 Sequence Diagram & Interaction Flow Update

Update the sequence diagram in Backlog §7:
* After Step 423 (Mechanical AST parse), split Step 424 into:
  * `Worker ->> LocalLLM: Assess extraction quality (atomize compound clauses, classify modality SHALL/SHOULD, score confidence)`
  * `Worker ->> LocalLLM: Evaluate cross-node contradiction risk against approved active requirements`
* Ensure model provenance and confidence attributes are stored in `graph_nodes.attributes->'extraction_metadata'`.

### 4.3 Ingestion Benchmarking Spike (CAL-Q2 Calibration)

Capture an evaluation spike in Backlog §6 to empirically validate model performance without holding up Phase 4 delivery:
* **CAL-Q2 (Requirement Atomization & Extraction Benchmark):** Benchmark the precision, recall, and formatting reliability of local commodity 8B models (e.g. Qwen 8B) versus commercial utility models (Haiku / Flash) on decomposing compound technical prose into atomic, testable requirement tuples.

---

## 5. Summary of Recommended Adjustments

| Document | Section | Proposed Adjustment |
| :--- | :--- | :--- |
| **`vision.md`** | §2 (Key Capability 2) & §4 (Conceptual Loop 1) | Reorder semantic evaluation: replace "Contradiction-First" with a two-stage evaluation model (**Requirement Quality & Atomization First, Cross-Graph Contradiction Second**). Formalize that requirement entities are discrete engineering contracts derived from, but not identical to, source spans. |
| **`vision.md`** | §2 (Core Philosophy) & §5 (Invariant I-4) | Clarify that extraction normalization must emit compact relational tuples and structured modal classifications without open-ended generative rewriting, preserving cryptographic span anchors. |
| **`strategic-planning-backlog.md`** | §2 (Phase 4 Deliverable 6) | Subdivide Phase 4.6 into 4.6a (Mechanical AST), 4.6b (Extraction Quality & Atomization), 4.6c (Topological Contradiction Risk), and 4.6d (Narrative Staging Review). |
| **`strategic-planning-backlog.md`** | §6 & §7 | Update interaction sequence diagram to reflect the two-stage evaluation, and add the CAL-Q2 extraction quality calibration benchmark. |

This maintains architectural rigor, eliminates semantic sequencing paradoxes, and respects our core economic and resource constraints.
