# Evaluation Framework: Intent Elicitation & Grounded Goal Extraction (Track E-B)

> **Status:** Normative Evaluation Framework  
> **Target Sub-Phase:** E-B (Goal Extraction & Gap Analysis)  
> **Related Documents:** [Technical Vision](vision.md), [Strategic Planning Backlog](strategic-planning-backlog.md), [Transcript Format Specification](elicitation-transcript-format.md), [Implementation Plan](../../docs/plans/intent-elicitation-eb-plan.md)

---

## 1. Objectives & Evaluation Philosophy

Goal extraction from fuzzy stakeholder intent is inherently subjective and prone to two failure modes:

1. **Context Splintering:** Mistaking transient conversational exploratory ideas or retracted suggestions for permanent architecture goals.
2. **Premature Concretization:** Narrowing stakeholder aspirations into rigid implementation mechanisms (e.g., turning an analogy like "like a spreadsheet" into a hard requirement for an Excel grid).

This evaluation framework defines the metrics, matching algorithms, ground-truth label formats, and human adjudication protocol to validate that TKS extraction rails reliably extract goals while suppressing hallucination, concretization, and leakage.

---

## 2. Evaluation Metrics & Provisional Thresholds

The pipeline is evaluated against eleven quantitative metrics derived from human-adjudicated labels:

| Metric | Definition | Mathematical Formulation | Provisional Target |
| :--- | :--- | :--- | :--- |
| **Goal Recall** | Fraction of labelled non-retracted goals successfully matched by a staged candidate goal | $\frac{\|M_{\text{active}}\| + \|M_{\text{tentative}}\|}{\|L_{\text{active}}\| + \|L_{\text{tentative}}\|}$ | $\ge 0.80$ |
| **Goal Precision** | Fraction of staged candidate goals that match a valid non-retracted label | $\frac{\|M_{\text{staged}}\|}{\|S\|}$ | $\ge 0.75$ |
| **Kind Accuracy** | Correct classification of `goal_kind` (`ACHIEVE`, `MAINTAIN`, `AVOID`, `SOFTGOAL`) among matched goals | $\frac{\sum_{m \in M} \mathbb{I}(m.\text{kind} = l.\text{kind})}{\|M\|}$ | $\ge 0.80$ |
| **Grounding Validity** | Fraction of staged goals whose cited quotes are mechanically verified exact substrings of source turns | $\frac{\|S_{\text{verified\_grounding}}\|}{\|S\|}$ | $= 1.00$ |
| **Premature Concretization Rate** | Fraction of staged goals flagged as premature concretizations via forbidden terms or ungrounded mechanism nouns | $\frac{\|S_{\text{concretized}}\|}{\|S\|}$ | $\le 0.10$ |
| **Retraction Leakage** | Fraction of labelled `RETRACTED` ideas incorrectly staged as active goals | $\frac{\|M_{\text{retracted}}\|}{\|L_{\text{retracted}}\|}$ | $\le 0.05$ |
| **Venting False Positives** | Fraction of emotional venting turns (`VENTING_NO_GOAL` traps) that incorrectly produce a staged goal | $\frac{\|S \cap T_{\text{venting}}\|}{\|T_{\text{venting}}\|}$ | $\le 0.10$ |
| **Phantom Catch Rate** | Fraction of unstated "best practice" phantom entailment cases correctly rejected by Pass 4 | $\frac{\|E_{\text{phantom\_rejected}}\|}{\|E_{\text{phantom}}\|}$ | $\ge 0.80$ |
| **Gap Recall** | Fraction of labelled structural gaps identified by the mechanical detectors and phrased as open questions | $\frac{\|G_{\text{detected}}\|}{\|G_{\text{labelled}}\|}$ | $\ge 0.80$ |
| **Run-to-Run Stability** | Mean Jaccard similarity of matched label sets across $k=3$ independent non-deterministic runs | $\frac{1}{3} \sum_{i < j} \frac{\|M_i \cap M_j\|}{\|M_i \cup M_j\|}$ | $\ge 0.70$ |
| **Cost Efficiency** | Agent prompt and completion token expenditure per 1,000 source tokens across model tiers | Tokens / 1k source tokens | Baseline characterization |

---

## 3. Matching & Scoring Protocol

### 3.1 Candidate-to-Label Matching Algorithm

Matching between an agent-staged candidate goal $s \in S$ and a ground-truth label $g \in G$ is 1-to-1 and greedy, evaluated in two stages:

1. **Grounding Alignment Filter (Condition I):**
   * For transcripts: Candidate $s$ must share $\ge 1$ cited turn identifier with label $g$, OR have an overlapping quote character span with label $g$'s grounding quote.
   * For vision documents: Candidate $s$ must cite the same section heading and overlap with the verbatim quote span.
2. **Semantic Equivalence (Condition II):**
   * Pre-filter semantic similarity using cosine distance over dense vector embeddings (`fastembed-rs` `all-MiniLM-L6-v2`):
     $$\cos(\mathbf{e}_s, \mathbf{e}_g) \ge 0.75$$
     evaluated against label statement $g.\text{statement}$ and all entries in $g.\text{acceptable\_paraphrases}$.
   * **Borderline Triage:** If cosine similarity falls in $[0.60, 0.75)$, the match is routed to human adjudication or the verified entailment judge (EB-5). Pairs with similarity $< 0.60$ are non-matches.
3. **Greedy Assignment:**
   * Candidate pairs satisfying Conditions I and II are ranked by descending cosine similarity and assigned greedily (each label and staged goal matched at most once).

### 3.2 Premature Concretization Flagging

A staged candidate goal $s$ matching label $g$ is classified as **prematurely concretized** if:

1. Its statement contains any lowercase token or lemma present in $g.\text{forbidden\_concretizations}$ (e.g. `"dashboard"`, `"excel"`, `"slack"`), **OR**
2. Its statement contains a mechanism noun from the global technology lexicon (e.g. `"database"`, `"websocket"`, `"redis"`, `"microservice"`, `"react"`, `"cron"`) that is **completely absent** from its cited verbatim grounding quotes.

---

## 4. Human Adjudication & Inter-Rater Agreement

### 4.1 Adjudication Workflow

1. **Agent Draft:** During corpus initialization (EB-1), the agent produces draft label files (`tests/fixtures/elicitation/labels/*.labels.json`) with `adjudication.status = "DRAFT"` and `adjudication.labeled_by = "agent"`.
2. **Independent Human Blind Labelling:** To prevent bias, the human supervisor independently labels goals and kinds for two transcripts **prior to inspecting the agent drafts**.
3. **Inter-Rater Reliability Reporting:**
   * Compute Cohen's kappa ($\kappa$) on `goal_kind` classifications across overlapping goals.
   * Compute unweighted F1 score between human-labelled goal sets and agent-drafted goal sets.
4. **Adjudication Confirmation:**
   * The human supervisor reviews each labels file, resolves discrepancies, edits statements, and updates frontmatter:

     ```json
     "adjudication": {
       "status": "ADJUDICATED",
       "labeled_by": "agent",
       "adjudicated_by": "username",
       "notes": "Reviewed and confirmed trap matrix and goal boundaries."
     }
     ```

---

## 5. Strategic Kill & Pivot Criteria

The Intent Elicitation track adheres to the following falsification boundaries:

1. **Extraction Inefficacy Kill Condition:**
   If prompt and pipeline iterations fail to achieve Goal Recall $\ge 0.60$ OR Premature Concretization Rate $\le 0.25$ using commercial utility-tier models, the single-shot pipeline is falsified. Pivot: drop episode segmentation or switch to an interactive turn-by-turn elicitation protocol.
2. **Entailment Verification Falsification:**
   If neither the fresh-context agent judge nor the local ONNX NLI cross-encoder achieves $\ge 0.70$ agreement with adjudicated entailment labels (or fails to achieve $\ge 0.80$ phantom catch rate), automated NLI verification is downgraded to advisory-only, and human review carries scope protection.
3. **Label Subjectivity Falsification:**
   If human/agent inter-rater agreement on goal sets falls below $F_1 < 0.60$, goal definitions are judged too subjective for deterministic automated benchmarking. The labelling guide and taxonomies must be tightened before evaluating later work packages.
