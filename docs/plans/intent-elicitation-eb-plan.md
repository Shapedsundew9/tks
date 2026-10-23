# Implementation Plan: Intent Elicitation Track — E-B (Goal Extraction & Gap Analysis)

> **Audience:** an implementation agent working in this repository.
> **Execution scope right now:** **EB-0 then EB-1 only.** EB-2 … EB-8 are specified for context and
> sequencing; do **not** start them until the user approves EB-1 adjudication (see the §6.3 gate).
> **Status:** Approved plan. Not yet executed.

---

## 1. Problem Statement

Phase 4 ingestion (WP-4.1 … WP-4.6, complete) assumes documents arrive structured enough to atomize
into requirements (H1/H2 → `REQUIREMENT`, sentence atomization, RFC-2119 modality). The very first
stage of building a knowledge base is not like that. Real first-stage intent is a **fuzzy vision**:
either a negotiated back-and-forth conversation (eventually a transcript) or a loosely written
product-department vision document, containing at best *soft goals*.

Requirements cannot simply be "pulled out" of such material. Goals must be **elicited**,
**extracted**, **checked for gaps**, and **iterated** (gaps feed the next round of questioning).

Naive paragraph-by-paragraph LLM extraction fails in two characteristic ways:

1. **Context splintering** — ephemeral conversational stepping stones are extracted as permanent goals.
2. **Premature concretization** — analogies / UI metaphors ("like an Excel sheet") are operationalized
   into rigid features, narrowing scope far beyond the stakeholder's intent.

## 2. The Intent Elicitation Track

A new track, slotted **ahead of Phase 5** in the strategic roadmap. Phase 4 numbering is untouched.

- **E-A — Conversational Elicitation & Transcription** *(planned separately; see
  [E-A plan](./intent-elicitation-ea-plan.md))*:
  web page + microphone → speech recognition (phonetic transcription) → language-model clean-up of
  disfluencies/ASR artifacts → LLM-driven Socratic questioning (including scoping by "what it is
  *not*") → transcript emitted in the E-B transcript format.
- **E-B — Goal Extraction & Gap Analysis** *(this plan)*: transcript **or** free-form vision
  document → grounded goal model (hard goals, soft goals, anti-goals) → gap report + grounded open
  questions that feed the next E-A cycle.

```text
 [E-A conversation] ──transcript──▶ ┌──────────────────── E-B ────────────────────┐
 [vision document]  ───────────────▶│ parse → episodes → extract → ground →        │──▶ GOAL drafts → staging review
        ▲                           │ verify (entailment) → gap analysis           │
        └──── open questions ◀──────│                                              │
                                    └──────────────────────────────────────────────┘
```

## 3. Governing Principles (non-negotiable)

1. **No LLM inside TKS — "bring your own cognition".** TKS supplies *rails*: protocol, schemas,
   canonical versioned pass instructions, mechanical verification, storage and review. The user's own
   MCP-capable agent CLI (e.g. Copilot CLI, Claude Code, Codex) performs the cognition and bears the
   token cost. The TKS CLI initialises a session and the agent drives the conversation through MCP.
2. **Optional local offload is a different tier.** A local model (devcontainer Ollama) is a cheap
   offload for high-volume, low-sophistication prompts. It also changes the interaction model: TKS can
   *initiate* a conversation instead of waiting for the commercial tool. It is optional, never
   required, and never linked into the `tks serve` core path (constraint C-25 stands).
3. **Not ghostwriting; not a spec-authoring UX.** E-B helps the *user* articulate *their own* vision.
   Every goal must be anchored to the user's verbatim words. TKS never invents intent.
4. **Mechanical where possible.** Turn/sentence spans, verbatim quote verification, schema validation,
   technology-term lint and structural gap detection are deterministic Rust. Only segmentation,
   abstraction and judgement are delegated to the user's agent.
5. **Technology-agnosticism rule.** A goal states a desired state of affairs in the domain, not the
   mechanism used to achieve it. Analogies and UI metaphors are evidence, not requirements.
6. **Every goal is grounded.** ≥ 1 verbatim quote mechanically verified against a source turn or
   sentence. Unanchorable goals are rejected, never staged.

## 4. Confirmed Design Decisions

These are recorded as architecture decisions **D-124 … D-129** in EB-0.

| Plan ID | ADR ID | Decision |
| --------- | -------- | ---------- |
| D-EB-1 | D-124 | LLM passes run in the user's agent via MCP; TKS has no LLM dependency. Optional local Ollama offload stays outside the `tks serve` core path. |
| D-EB-2 | D-125 | New node type `GOAL` with `attributes.goal_kind ∈ {ACHIEVE, MAINTAIN, AVOID, SOFTGOAL}` and new node type `SOURCE_EXCERPT`; new edge types `GROUNDED_IN`, `REFINES`, `CONFLICTS_WITH`, `CONTRIBUTES_TO`. INV-1 amended: an active `GOAL` is a legitimate lineage root, and `REQUIREMENT` nodes may derive from `GOAL` nodes. |
| D-EB-3 | D-126 | Naming: "Intent Elicitation track", sub-phases E-A (backlogged) and E-B (planned). Phase 4 numbering untouched. |
| D-EB-4 | D-127 | Evaluation corpus = hand-built synthetic transcripts with planted traps **plus** `docs/vision/vision.md` as a dogfood vision document. Labels drafted by the agent, adjudicated by the user. |
| D-EB-5 | D-128 | Input format = defined Markdown transcript (`tks-transcript/v1`, §6.2). Free-form Markdown vision documents accepted as-is. |
| D-EB-6 | D-129 | Entailment verification: spike both (a) a fresh-context agent judge pass and (b) a local, non-generative NLI cross-encoder (DeBERTa-v3 MNLI via ONNX); choose by agreement with adjudicated labels. |

Sub-decisions resolved later (with recommended default):

- **Grounding storage (EB-3):** `graph_edges` has no attributes column. Default: per-turn/per-sentence
  `SOURCE_EXCERPT` nodes carrying byte spans via existing `doc_path`/`byte_start`/`byte_end`;
  `GOAL -GROUNDED_IN-> SOURCE_EXCERPT` edges for traversal; exact quote sub-spans in
  `GOAL.attributes.grounding[]`. Alternative: add `attributes JSONB` to `graph_edges`.
- **MCP prompts (EB-4):** the MCP server does not implement `prompts/list` / `prompts/get`. Add it so
  every client runs identical versioned pass instructions.

## 5. E-B Extraction Pipeline (reference for EB-2 … EB-8)

| Pass | Performed by | What | TKS mechanical check |
| ------ | ------------- | ------ | ---------------------- |
| 0. Parse | TKS | Transcript → turns `t1..tn` (speaker, role, byte span). Vision doc → sentence chunks `c1..cn` (reuse Stage 1 atomizer). | Byte-exact spans; frontmatter validation. |
| 1. Episodes & coreference | Agent | Segment into contiguous topic episodes (problem → exploration → confirm/reject). Resolve references ("that", "the earlier idea") to explicit verbatim referent quotes. | Episodes contiguous and covering all turns; referent quotes verbatim. |
| 2. Schema-constrained extraction | Agent | Per episode: hard goals, soft goals, anti-goals with stakeholder, statement, status (`CONFIRMED`/`TENTATIVE`/`RETRACTED`), rationale, grounding quotes, candidate links. | JSON-schema validation; technology-term lint (mechanism nouns not required by the quote ⇒ premature-concretization warning). |
| 3. Bidirectional grounding | TKS | Every quote must be a verbatim (whitespace-normalised) substring of the cited turn; compute byte sub-spans. | Reject with remediation envelope; citations of retracted turns flagged. |
| 4. Entailment verification | Agent judge **or** local NLI | Forward: premise ⇒ goal (`ENTAILED` / `NEUTRAL` = scope creep / `CONTRADICTED`). Reverse: goal ⇒ premise (detects over-constraint). | Verdicts in `attributes.verification`; non-entailed goals routed to anomaly triage. |
| 5. Gap analysis | TKS + Agent | Mechanical gap detectors (§6.3 step 2 `gap.detector` list); agent phrases each gap as an open question. | Each question references a gap id. |
| 6. Stage | TKS | `GOAL` drafts in narrative order with verbatim excerpts and badges → existing staging approve/reject. | Amended INV-1; edge pre-validation. |

---

## 6. Work Packages to Execute Now

General rules for the implementation agent:

- Read `.github/copilot-instructions.md`, `.shared/ai/core/service-instructions.md` and
  `.shared/ai/core/common-instructions.md` first.
- Do **not** add any LLM SDK, HTTP LLM client code, or model dependency in EB-0/EB-1.
- Do **not** modify Rust source under `src/` or migrations in EB-0/EB-1. The only Rust added in EB-1
  is a fixture-integrity integration test (§6.3, step 7).
- Keep existing document style: numbered sections, tables for decisions/constraints, decision
  entries formatted exactly like existing `### D-123` entries in `docs/vision/architecture.md`
  (copy that entry's field set: Status / Origin / Context / Decision / Rationale / …).
- After any Markdown change run `npx markdownlint-cli2 --fix "**/*.md"`. Never edit
  `.markdownlint-cli2.jsonc`. Report any remaining lint issues instead of suppressing them.
- Commit EB-0 and EB-1 as separate commits (Conventional Commits style used in this repo), each with
  the trailer `Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>`.

### 6.1 EB-0 — Articulate the Track in the Vision, Architecture and Backlog (docs only)

**Goal:** the governing documents describe the Intent Elicitation track, the "bring your own
cognition" model, and the goal-model schema change, so that later code work is traceable to approved
intent. All schema/invariant text is written as **decided, pending implementation in EB-3**.

Steps:

1. **`docs/vision/vision.md`**
   1. §1 *Problem Space*: add a subsection **"The Vision Articulation Gap (Fuzzy Intent Before
      Requirements)"** directly after "The Specification Void & The Cold-Start Adoption Barrier".
      Content: first-stage intent is fuzzy (conversations, prose visions, soft goals); users are bad
      at articulating it, which pushes them towards ghostwritten specs; naive extraction causes
      context splintering and premature concretization; brownfield scaffolding does not cover this.
   2. §1 *Explicit Non-Goals*: **amend, don't delete**, the "No Spec-Authoring UX" and "No
      In-Substrate Generative Ghostwriting" bullets with one clarifying sentence each. Elicitation
      rails, meaning protocol, schemas and verification that help a user articulate their *own* intent,
      grounded in their verbatim words, are in scope. Generating intent on their behalf is not.
   3. §2 *Core Philosophy: Minimal LLM Reliance*: add a paragraph **"Bring Your Own Cognition"**.
      TKS contains no LLM. Commercial cognition is supplied by the user's MCP-capable agent CLI, which
      bears the cost; the TKS CLI initialises sessions and the agent drives them. Optional local
      offload is a separate low-cost tier for high-volume, low-sophistication prompts that also lets
      TKS initiate conversations.
   4. §2 *Key Capabilities*: append capability **12. "Intent Elicitation & Grounded Goal
      Modeling"**. It covers elicitation (E-A), goal/softgoal/anti-goal extraction with verbatim
      grounding and entailment verification (E-B), and the iterative gap → question loop.
   5. §5 *Strategic Hypotheses*: add hypothesis **H-ELICIT**: "Under TKS rails (episode
      segmentation, schema-constrained technology-agnostic extraction, mechanical verbatim grounding,
      entailment verification), a user-supplied agent extracts goals from fuzzy vision material with
      recall ≥ 0.80 and premature-concretization ≤ 0.10 against adjudicated labels." Add a matching
      row in the *Hypothesis Evidence & Validation Status Ledger* with status `UNTESTED` and evidence
      pointer `docs/plans/intent-elicitation-eb-plan.md` (EB-8). Match the existing ledger's column
      layout.
2. **`docs/vision/architecture.md`**
   1. §3 *Architectural Invariants*: amend **INV-1** so an active `GOAL` node is an accepted lineage
      root, alongside an active requirement node. Add a parenthetical
      "(GOAL roots: D-125; implementation EB-3)". Do not change the other invariants.
   2. §5 *Data & State Model*: add a subsection **"Goal Model (Intent Elicitation)"** describing
      `GOAL` (`goal_kind`, `status`, `stakeholder`, `grounding[]`, `verification`),
      `SOURCE_EXCERPT`, and the edges `GROUNDED_IN` (GOAL → SOURCE_EXCERPT), `REFINES` (GOAL → GOAL,
      REQUIREMENT → GOAL), `CONFLICTS_WITH` (GOAL ↔ GOAL, stored once with lower UUID as `from`),
      and `CONTRIBUTES_TO` (GOAL/REQUIREMENT → SOFTGOAL-kind GOAL). Note that the `chk_node_type`
      expansion lands in migration `V7__goal_model.sql` (EB-3).
   3. §9 *Decisions*: append **D-124 … D-129** using the §4 table, after `D-123`.
   4. §2.2 *Derived Constraints*: add a constraint row **C-27** (next free number; verify first).
      Elicitation and extraction cognition is external to TKS. TKS may only verify, store and stage
      agent submissions, and every staged `GOAL` must carry ≥ 1 mechanically verified verbatim
      grounding span. Origin: D-124/D-125.
3. **`docs/vision/strategic-planning-backlog.md`**
   1. §2 roadmap Mermaid: insert node
      `PE["Intent Elicitation Track (E-A / E-B)<br/><i>(Conversational Elicitation + Grounded Goal Extraction & Gap Loop)</i>"]:::secondary`
      and re-wire `P4 --> PE --> P5`. Keep the existing theme block unchanged.
   2. Add a section **"Intent Elicitation Track: Vision Articulation & Grounded Goal Extraction"**
      between the Phase 4 and Phase 5 sections, in the same style (Primary Objective, Core
      Deliverables). It needs two sub-entries:
      - **E-A (Backlogged — not yet planned):** browser page with microphone capture; speech
        recognition model for English transcription; language-model clean-up of ASR artifacts; agent-
        or locally-driven Socratic elaboration (including boundary scoping by negation); output in
        `tks-transcript/v1`. Open spike: MCP sampling vs. local-offload for TKS-initiated
        conversations.
      - **E-B (Planned):** summary of the §5 pipeline, a link to this plan
        (`docs/plans/intent-elicitation-eb-plan.md`), and the work-package list EB-0 … EB-8.
   3. §4 *MVD Acceptance Test Scripts*: add an **"Intent Elicitation E-B MVD"** script (Preconditions,
      Steps, Expected Results), derived from §7.3 of this plan (dogfood run on `vision.md`,
      cycle-2 gap closure).
   4. §6 *Quantitative Operational Targets*: add calibration rows `CAL-EB-RECALL`,
      `CAL-EB-PRECISION`, `CAL-EB-CONCRETIZATION`, `CAL-EB-GROUNDING`, `CAL-EB-GAP-RECALL` with the
      provisional targets in §7.2. Mark them "provisional — calibrate after first baseline".
4. **`docs/vision/technical-backlog.md`**: add **TB-14**, "MCP `prompts/*` capability for versioned
   pass instructions (needed by EB-4)", following the existing TB entry format.
5. **`README.md`**: only if it lists roadmap phases or vision docs, add a one-line pointer to the
   Intent Elicitation track. Otherwise leave it unchanged.

**EB-0 acceptance criteria:**

- [ ] All five vision.md edits present; no existing non-goal removed.
- [ ] INV-1 amended; D-124 … D-129 present and numbered contiguously after D-123; C-27 (or the next
      free number) added.
- [ ] Roadmap Mermaid is valid (validate the block with a Mermaid validator if available) and shows
      `P4 --> PE --> P5`.
- [ ] Backlog contains E-A (backlogged) and E-B (planned) entries, an MVD script, and the CAL-EB rows.
- [ ] TB-14 added.
- [ ] `npx markdownlint-cli2 --fix "**/*.md"` leaves no new issues in changed files.
- [ ] Commit `docs(elicitation): EB-0 articulate intent elicitation track` created.

### 6.2 Transcript Format `tks-transcript/v1` (specified here; documented in EB-1)

A transcript is a UTF-8 Markdown file:

```markdown
---
format: tks-transcript/v1
title: "Team capacity planning vision"
session_id: "T01"          # unique within corpus / project
cycle: 1                   # elicitation iteration number (1-based)
date: "2026-10-12"
language: "en"
source: "synthetic"        # synthetic | manual | asr
participants:
  - { name: "Priya", role: "stakeholder" }
  - { name: "Interviewer", role: "facilitator" }
---

**[t1] Interviewer (facilitator):** _(00:00:05)_ What problem are you trying to solve?

**[t2] Priya (stakeholder):** Honestly, I spend half my Monday chasing people on Slack.
```

Rules (the EB-1 fixture test and later the EB-2 parser enforce these):

1. YAML frontmatter is mandatory, and `format` must be exactly `tks-transcript/v1`.
2. Every turn is **exactly one Markdown paragraph** (blank-line separated) beginning with
   `**[tN] Name (role):**`. `N` starts at 1 and increments by 1 without gaps. `Name` must appear in
   `participants`, and `role` must match that participant's role.
3. Allowed roles: `stakeholder`, `facilitator`, `agent`.
4. Optional timestamp immediately after the speaker tag: `_(HH:MM:SS)_`.
5. The **utterance text** is everything after the speaker tag (and timestamp, if present), trimmed.
   Grounding quotes are matched against the utterance text only.
6. No other block content (headings, lists, code) is allowed between turns. Inline Markdown inside
   an utterance is discouraged; synthetic fixtures must not use it.
7. Free-form vision documents are **not** transcripts and need no frontmatter. They are addressed
   by sentence chunks in later WPs and by verbatim quotes plus a section heading in labels.

### 6.3 EB-1 — Ground-Truth Corpus, Label Schema & Metrics (test-first)

**Goal:** before any extraction code exists, produce the evaluation corpus, the label format, the
metric definitions and an automated integrity check, then stop for user adjudication.

Files to create:

```text
docs/vision/elicitation-transcript-format.md     # normative spec of §6.2 (+ rationale, examples)
docs/vision/elicitation-evaluation.md            # metrics, matching protocol, thresholds, adjudication process
tests/fixtures/elicitation/README.md             # corpus index + trap matrix + how to add a fixture
tests/fixtures/elicitation/labels.schema.json    # JSON Schema (draft 2020-12) for *.labels.json
tests/fixtures/elicitation/transcripts/T01-<slug>.md … T10-<slug>.md
tests/fixtures/elicitation/labels/T01-<slug>.labels.json … T10-<slug>.labels.json
tests/fixtures/elicitation/labels/vision.labels.json   # labels for docs/vision/vision.md
tests/elicitation_fixtures.rs                    # fixture-integrity test (no DB, no LLM)
```

Steps:

1. **Write `docs/vision/elicitation-transcript-format.md`** from §6.2, adding three worked
   examples: a valid transcript, an invalid one (gap in turn ids), and a free-form vision doc
   snippet.
2. **Write the label schema** `labels.schema.json`. Each labels file is one JSON object:

   ```json
   {
     "schema_version": "tks-elicitation-labels/v1",
     "source": { "kind": "transcript", "path": "tests/fixtures/elicitation/transcripts/T01-capacity.md", "sha256": "<hex of file bytes>" },
     "adjudication": { "status": "DRAFT", "labeled_by": "agent", "adjudicated_by": null, "notes": "" },
     "episodes": [ { "id": "E1", "turns": ["t1", "t6"], "topic": "status visibility pain" } ],
     "goals": [
       {
         "id": "G1",
         "kind": "ACHIEVE",
         "statement": "Project managers can independently assess sprint progress.",
         "stakeholder": "Priya",
         "status": "CONFIRMED",
         "episode": "E1",
         "grounding": [ { "turn": "t2", "quote": "I spend half my Monday chasing people" } ],
         "acceptable_paraphrases": ["Managers can see progress without asking engineers."],
         "forbidden_concretizations": ["dashboard", "Slack integration"],
         "links": [ { "type": "CONTRIBUTES_TO", "target": "G3" } ]
       }
     ],
     "traps": [
       { "id": "TR1", "type": "ANALOGY", "turns": ["t4"], "expected": "NO_MECHANISM_GOAL", "note": "\"like an Excel sheet\" is a metaphor" }
     ],
     "gaps": [ { "id": "GP1", "detector": "SOFTGOAL_WITHOUT_CONTRIBUTOR", "refs": ["G3"], "description": "…" } ],
     "entailment_cases": [
       { "id": "EC1", "premise_turns": ["t2", "t3"], "hypothesis": "…", "forward": "ENTAILED", "reverse_overconstrained": false, "phantom": false }
     ]
   }
   ```

   `episodes[].turns` is an inclusive `[first, last]` range. Enumerations:
   - `goal.kind`: `ACHIEVE | MAINTAIN | AVOID | SOFTGOAL`
   - `goal.status`: `CONFIRMED | TENTATIVE | RETRACTED`. Retracted ideas are labelled so leakage can be
     measured; they must **not** be staged by a correct extractor.
   - `link.type`: `REFINES | CONFLICTS_WITH | CONTRIBUTES_TO`
   - `trap.type`: `ANALOGY | UI_METAPHOR | RETRACTION | MULTI_TURN_COREFERENCE | VENTING_NO_GOAL |
     CROSS_TURN_CONTRADICTION | IMPLICIT_SOFTGOAL | EXPLICIT_ANTIGOAL | BOUNDARY_BY_NEGATION |
     MISSING_STAKEHOLDER | FACILITATOR_LEADING`
   - `trap.expected`: `NO_GOAL | NO_MECHANISM_GOAL | ABSTRACT_GOAL | RETRACTED_NOT_STAGED |
     RESOLVED_REFERENT | CONFLICT_LINK | ANTIGOAL | GAP_REPORTED | FACILITATOR_NOT_SOURCE`
   - `gap.detector`: `SOFTGOAL_WITHOUT_CONTRIBUTOR | GOAL_WITHOUT_STAKEHOLDER | EPISODE_WITHOUT_GOAL |
     UNRESOLVED_CONFLICT | TENTATIVE_NEVER_CONFIRMED | ANTIGOAL_WITHOUT_RATIONALE |
     SILENT_STAKEHOLDER | NO_BOUNDARY_STATEMENT`
   - `entailment.forward`: `ENTAILED | NEUTRAL | CONTRADICTED`
   - `adjudication.status`: `DRAFT | ADJUDICATED`

   For `vision.labels.json`, `source.kind = "document"`, `source.path = "docs/vision/vision.md"`,
   and grounding entries use `{ "section": "<heading text>", "quote": "<verbatim>" }` instead of
   `turn`. Use JSON Schema `oneOf` to allow both grounding shapes.
3. **Write the 10 synthetic transcripts.** Requirements:
   - 15–40 turns each, realistic disfluent speech (fillers, self-corrections) but no inline Markdown.
   - Vary the domains. At most **two** may be about software-engineering tooling; the rest come from
     other domains such as clinic scheduling, community library, logistics, a household budgeting app,
     school timetabling, field maintenance, a museum visitor experience, or a volunteer network.
   - Each transcript plants **≥ 3 traps**. Across the corpus every trap type appears **≥ 2 times**.
     Record the matrix in `tests/fixtures/elicitation/README.md` (rows = transcripts,
     columns = trap types).
   - At least two transcripts contain an explicit "what it is **not**" boundary discussion, and at
     least one contains none (so `NO_BOUNDARY_STATEMENT` fires).
   - At least two transcripts have ≥ 2 stakeholders with a cross-stakeholder contradiction.
   - Include at least one `cycle: 2` transcript (e.g. `T10`) that continues `T01`. It answers some of
     `T01`'s gaps and leaves others open. Note the pairing in both label files' `adjudication.notes`.
   - Facilitator turns may be *leading* ("so you want a dashboard, right?"). A stakeholder's mere
     acquiescence must not be labelled as a confirmed mechanism goal (`FACILITATOR_LEADING` trap).
4. **Label each transcript** with `episodes`, `goals` (including `RETRACTED` and `TENTATIVE` ones),
   `traps`, `gaps`, and `entailment_cases`. Rules for labelling:
   - Goal statements must follow the technology-agnosticism rule (§3, principle 5). List mechanism
     words the stakeholder used as analogy in `forbidden_concretizations`.
   - Every grounding quote must be copied **verbatim** from the utterance text of the cited turn, and
     must come from a `stakeholder` turn unless the stakeholder explicitly confirms a facilitator's
     wording. In that case, ground on both turns.
   - Each transcript gets ≥ 4 `entailment_cases`, including ≥ 1 `NEUTRAL` **phantom** (a plausible
     "best practice" goal that was never stated, `phantom: true`) and ≥ 1 with
     `reverse_overconstrained: true`.
5. **Label `docs/vision/vision.md`** in `vision.labels.json`. Do this **after EB-0 is committed**, and
   record the `sha256` of the file at that commit. Aim for 20–40 goals across all four kinds, at least
   5 soft goals and 3 anti-goals (the "Explicit Non-Goals" section is a rich anti-goal source). Skip
   traps and entailment cases for this document. Add `gaps` you genuinely observe.
6. **Write `docs/vision/elicitation-evaluation.md`** containing:
   - Metric definitions and provisional thresholds exactly as in §7.2.
   - **Matching protocol:** a staged goal matches a label when (i) it shares ≥ 1 grounding turn or
     overlapping quote span with the label, **and** (ii) its statement is semantically equivalent to
     the label `statement` or an `acceptable_paraphrases` entry. Pre-filter (ii) with fastembed cosine
     similarity ≥ 0.75. Borderline cases between 0.60 and 0.75 go to adjudication, or to the judge
     selected in EB-5. Matching is one-to-one (greedy by similarity).
   - **Premature concretization:** a staged goal is concretized if its statement contains any
     `forbidden_concretizations` term of its matched label, or a mechanism term from the
     technology lexicon that is absent from its grounding quotes.
   - **Adjudication process:** the agent drafts (`status: DRAFT`); the user reviews each file and
     edits it, then sets `status: ADJUDICATED` with `adjudicated_by`. Inter-rater agreement: the user
     independently labels goals for 2 transcripts *before* viewing the agent draft. Report Cohen's κ
     on goal kind and F1 on goal sets in this document.
   - Kill/pivot criteria from §7.4.
7. **Write `tests/elicitation_fixtures.rs`** (pure Rust, no DB, no network, no new dependencies).
   Use `serde_json`, `serde_yaml`, `regex` and `sha2`, which are already in `Cargo.toml`. For every
   transcript and labels file it must assert:
   - Frontmatter parses; `format == "tks-transcript/v1"`; every participant referenced exists, with a
     matching role.
   - Turn ids are `t1..tN`, contiguous, one per paragraph, and no non-turn blocks exist.
   - Labels file parses into strongly typed serde structs whose enums mirror §6.3 step 2, with
     `deny_unknown_fields`. Do not add a JSON-Schema validation crate.
   - `source.sha256` matches the actual file bytes. For `vision.labels.json` a mismatch must produce a
     clear failure message instructing the user to re-label or re-pin after `vision.md` edits.
   - Every grounding quote is a substring of the cited turn's utterance text, after collapsing runs
     of whitespace. For the vision document, the quote is matched within the named section.
   - Episode ranges are within bounds, ordered, non-overlapping, and cover every turn.
   - All goal/trap/gap/link/entailment references resolve to existing ids and turns.
   - Corpus-level: ≥ 10 transcripts; every trap type occurs ≥ 2 times; every transcript has ≥ 1
     phantom entailment case.
   - It must **not** fail merely because `adjudication.status == "DRAFT"`. Instead print a summary of
     non-adjudicated files with `eprintln!`.
8. **Validate:** `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`,
   `cargo test --test elicitation_fixtures`, `npx markdownlint-cli2 --fix "**/*.md"`.
9. **Commit** `test(elicitation): EB-1 ground-truth corpus, label schema and metrics`.
10. **STOP — adjudication gate.** Report to the user: the trap matrix, label counts per file, and the
    list of files awaiting adjudication. Ask the user to adjudicate (and to perform the blind
    double-labelling for 2 transcripts). Do **not** proceed to EB-2 until the user confirms.

**EB-1 acceptance criteria:**

- [ ] 10 transcripts and 11 label files (10 transcripts + vision) exist and pass
      `cargo test --test elicitation_fixtures`.
- [ ] The trap matrix in the fixtures README shows every trap type ≥ 2 times.
- [ ] Format spec and evaluation docs exist and are linked from the strategic backlog E-B entry.
- [ ] fmt, clippy and markdownlint are clean.
- [ ] The agent stopped at the adjudication gate.

---

## 7. Testing & "Did We Get It Right?" (applies through EB-8)

### 7.1 Deterministic (CI, every commit)

- Parser: byte-exact turn and sentence spans, where slicing the source by span equals the text;
  malformed transcripts produce actionable errors.
- Grounding verifier: 100 % of perturbed quotes rejected (altered word, two turns spliced, wrong
  turn cited, retracted turn cited); 0 false rejections on labelled true quotes.
- Protocol: invalid submissions rejected with remediation envelopes; MCP conformance (camelCase
  `inputSchema`, `prompts/*`).
- Schema / INV-1: GOAL roots satisfy ancestry; invalid edge targets → `ERR_INVALID_EDGE_TARGET`.
- Golden replay: recorded agent submissions for each corpus item reproduce the expected staged goal
  set and gap report exactly.

### 7.2 Extraction quality (opt-in live eval vs adjudicated labels; provisional thresholds)

| Metric | Definition | Provisional target |
| -------- | ----------- | -------------------- |
| Goal recall | Labelled non-retracted goals matched by a staged goal | ≥ 0.80 |
| Goal precision | Staged goals matching a non-retracted label | ≥ 0.75 |
| Kind accuracy | Correct `goal_kind` among matches | ≥ 0.80 |
| Grounding validity | Staged goals with mechanically verified verbatim quotes | = 1.00 |
| Premature-concretization rate | Staged goals flagged as concretized (§6.3 step 6) | ≤ 0.10 |
| Retraction leakage | `RETRACTED` labels matched by a staged goal | ≤ 0.05 |
| Venting false positives | `VENTING_NO_GOAL` traps producing a staged goal | ≤ 0.10 |
| Phantom catch rate | Phantom entailment cases rejected by Pass 4 | ≥ 0.80 |
| Gap recall | Labelled gaps reported by the gap analysis | ≥ 0.80 |
| Run-to-run stability | Mean Jaccard of matched-label sets over k = 3 runs | ≥ 0.70 |
| Cost | Agent tokens per 1k source tokens, per model tier | Baseline only |

### 7.3 Human acceptance

- **Dogfood:** run E-B on `docs/vision/vision.md`. The user reviews the output in staging. Targets:
  ≥ 70 % of goals approved unedited, zero goals judged "invented", and gap questions judged useful.
- **Iteration:** answering the gap questions in a cycle-2 transcript closes those gaps, and the UUIDs
  of unchanged goals are preserved.

### 7.4 Kill / pivot criteria

- Best-tier recall < 0.60 **or** premature concretization > 0.25 after prompt iteration ⇒ revisit
  the pipeline: drop or merge the episode pass, or move to interactive per-episode confirmation.
- Neither verifier option reaches ≥ 0.70 agreement with adjudicated entailment labels ⇒ Pass 4
  becomes advisory-only and human review carries scope protection.
- User/agent inter-rater agreement on goal sets is F1 < 0.6 ⇒ the labels are too subjective.
  Tighten the labelling guide before trusting any threshold.

---

## 8. Later Work Packages (do not start; for sequencing only)

| WP | Title | Summary | Depends on |
| ---- | ------- | --------- | ----------- |
| EB-2 | Transcript & vision-doc parser | `src/ingest/transcript.rs`: frontmatter, turns, byte spans, aliases; vision docs via existing atomizer; `SOURCE_EXCERPT` emission; byte-exact property tests. Reuse the EB-1 fixture rules. | EB-1 (adjudicated) |
| EB-3 | Schema & invariants | `V7__goal_model.sql` (`GOAL`, `SOURCE_EXCERPT` node types; edge types); envelope/INV-1 ancestry accepts GOAL roots; edge pre-validation rules; migration and envelope tests. | EB-2 |
| EB-4 | MCP extraction session protocol | Tools `open_extraction_session`, `submit_episodes`, `submit_goal_candidates`, `submit_verification_verdicts`, `get_gap_report`, `stage_goals`; session state in `ingestion_jobs.attributes`; grounding verifier; technology-term lint (configurable lexicon); MCP `prompts/*` with versioned pass prompts (TB-14). | EB-3 |
| EB-5 | Verification spike | (a) fresh-context agent judge prompt; (b) ONNX DeBERTa-v3-MNLI cross-encoder behind a Cargo feature (like `vector-spike`). Measure agreement with adjudicated entailment labels and phantom catch rate; record `docs/vision/spike-eb5-results.md`. | EB-1 (adjudicated) |
| EB-6 | Gap analysis & question loop | Mechanical detectors (`gap.detector` enum); gap report (JSON + Markdown); grounded question prompt; cycle-n+1 goal reconciliation preserving UUIDs (reuse Phase 4 delta-reconciliation concepts). | EB-4 |
| EB-7 | Goal staging review UX | `tks staging list` / Explorer: narrative order, verbatim quotes with click-through to turn, kind, verdict, lint and gap badges. | EB-4 |
| EB-8 | Evaluation harness & exit gate | Offline golden-replay CI suite; opt-in live harness in `scripts/elicitation_eval/` driving the MCP protocol via an OpenAI-compatible endpoint configured by env (Ollama `qwen3:8b` or commercial) — kept outside `tks serve`; dogfood run; exit gate against §7. | EB-4, EB-5, EB-6 |

## 9. Notes & Cautions

- The original research note (CMU "Generative Goal Modeling", RE-NLP toolkits, Pydantic/Instructor)
  is guidance, not gospel. Verify any external reference before citing it in docs. Pydantic/Instructor
  maps to serde plus JSON Schema `inputSchema` in this Rust repository.
- Keep `tks serve` free of LLM SDKs (C-25). Live evaluation drivers live under `scripts/`, like the
  H-1 spike.
- Real transcripts may contain personal data. Commit only synthetic transcripts as fixtures.
- Fuzzy-goal ground truth is inherently subjective. Always report inter-rater agreement alongside
  extraction metrics.
