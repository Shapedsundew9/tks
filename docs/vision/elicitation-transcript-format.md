# Specification: Elicitation Transcript Format (`tks-transcript/v1`)

> **Status:** Normative Specification  
> **Schema Version:** `tks-transcript/v1`  
> **Related Documents:** [Technical Vision](vision.md), [Strategic Planning Backlog](strategic-planning-backlog.md), [E-A Implementation Plan](../../docs/plans/intent-elicitation-ea-plan.md), [E-B Implementation Plan](../../docs/plans/intent-elicitation-eb-plan.md)

---

## 1. Overview & Purpose

Phase 4 of The Knowledge Substrate (TKS) established mechanical ingestion and closed-loop traceability for structured engineering specifications, Architecture Decision Records (ADRs), and API schemas. However, initial software intent rarely originates in structured documents. Intent begins as conversational negotiation—unstructured dialogue, stakeholder interviews, and exploratory discussions.

The `tks-transcript/v1` format provides a deterministic, machine-verifiable, and Git-friendly Markdown representation for conversational transcripts. It serves as the intake contract for the Intent Elicitation track (Sub-phase E-B: Goal Extraction & Gap Analysis).

---

## 2. Normative Specification

A valid `tks-transcript/v1` document is a UTF-8 encoded text file with the `.md` extension conforming to the following rules:

### 2.1 YAML Frontmatter

Every transcript file MUST begin with a YAML frontmatter block delimited by `---` lines at the top of the file.

The frontmatter MUST include the following fields:

| Field | Type | Description | Constraints |
| :--- | :--- | :--- | :--- |
| `format` | string | Format specification identifier | MUST be exactly `"tks-transcript/v1"` |
| `title` | string | Descriptive title of the conversation | Non-empty string |
| `session_id` | string | Unique identifier within the corpus or project | Alphanumeric slug (e.g. `"T01"`) |
| `cycle` | integer | Elicitation iteration number | Positive integer ($\ge 1$) |
| `date` | string | Date of recording or synthesis | ISO 8601 date (`"YYYY-MM-DD"`) |
| `language` | string | Primary spoken language code | ISO 639-1 code (e.g. `"en"`) |
| `source` | string | Provenance of the transcript | MUST be one of: `"synthetic"`, `"manual"`, `"asr"` |
| `participants` | array | List of participants in the dialogue | MUST contain $\ge 1$ participant |

Each entry in `participants` MUST be a mapping with:

* `name` (string): Unique participant display name.
* `role` (string): Allowed roles are strictly:
  * `"stakeholder"`: Domain expert, end user, or sponsor providing domain intent.
  * `"facilitator"`: Interviewer or Socratic guide eliciting goals and boundaries.
  * `"agent"`: Autonomous or assistive agent participating in dialogue.

### 2.2 Turn Structure

Following the YAML frontmatter, the document body consists strictly of dialogue turns:

1. **Paragraph Isolation:** Every turn MUST be formatted as exactly one Markdown paragraph separated from preceding and subsequent turns by one or more blank lines.
2. **Turn Header:** Each paragraph MUST begin with an emphatic turn prefix:
   `**[tN] Name (role):**`
   * `N` is an integer starting strictly at `1` for the first turn and incrementing contiguously by `1` for each subsequent turn (`t1`, `t2`, `t3`, ...). Gaps, out-of-order turns, or duplicate identifiers are invalid.
   * `Name` MUST exactly match the `name` of a participant declared in the frontmatter.
   * `role` MUST exactly match the declared `role` of that participant.
3. **Optional Timestamp:** A turn prefix MAY be followed immediately by a single space and an italicized timestamp in parentheses:
   `_(HH:MM:SS)_`
4. **Utterance Text:** The utterance text begins after the speaker tag (and timestamp, if present), separated by a space. The utterance text spans the remainder of the paragraph.
5. **No Extraneous Block Elements:** Transcripts MUST NOT contain headings (`#`), horizontal rules (`---`), blockquotes (`>`), lists (`-` or `1.`), code fences (` ``` `), or tables between turns.
6. **No Inline Markdown in Synthetic Fixtures:** Utterances in synthetic evaluation fixtures MUST NOT contain bold, italics, or code formatting within the speech text itself, ensuring that speech-to-text realism is maintained.
7. **Verbatim Grounding Boundary:** The utterance text (with whitespace collapsed) defines the exact candidate text boundary against which downstream extraction passes mechanically verify quotes.
8. **Turn Count:** A transcript MUST contain at least one turn. The format imposes no maximum turn count. The 15–40 turn range applies only to the synthetic evaluation corpus as a benchmarking constraint, not as a format rule.

### 2.3 Free-Form Vision Documents

Free-form vision documents (such as [vision.md](vision.md) or product vision memos) are unstructured narrative Markdown artifacts rather than transcripts. They:

* Do NOT require YAML frontmatter or turn prefixes.
* Are parsed during Pass 0 into atomic sentence chunks using CommonMark AST parsers.
* In evaluation label sets, are grounded via `{ "section": "<heading text>", "quote": "<verbatim>" }` rather than turn identifiers.

---

## 3. Worked Examples

### 3.1 Example 1: Valid Transcript (`T01-capacity.md`)

```markdown
---
format: tks-transcript/v1
title: "Team Capacity Planning Vision"
session_id: "T01"
cycle: 1
date: "2026-10-12"
language: "en"
source: "synthetic"
participants:
  - { name: "Priya", role: "stakeholder" }
  - { name: "Marcus", role: "facilitator" }
---

**[t1] Marcus (facilitator):** _(00:00:02)_ Thanks for joining, Priya. What core problem are you experiencing with engineering allocation today?

**[t2] Priya (stakeholder):** _(00:00:10)_ Honestly, I spend half my Monday chasing tech leads on Slack just to know who has bandwidth for incoming support escalations.

**[t3] Marcus (facilitator):** _(00:00:22)_ So the primary pain point is lack of visible availability?

**[t4] Priya (stakeholder):** _(00:00:29)_ Yes. Project managers should be able to independently check team availability without interrupting developers during deep work.
```

### 3.2 Example 2: Invalid Transcript (Non-Contiguous Turn Identifiers)

```markdown
---
format: tks-transcript/v1
title: "Invalid Transcript Example"
session_id: "T-ERR"
cycle: 1
date: "2026-10-12"
language: "en"
source: "synthetic"
participants:
  - { name: "Alice", role: "stakeholder" }
  - { name: "Bob", role: "facilitator" }
---

**[t1] Bob (facilitator):** Where are files stored?

**[t3] Alice (stakeholder):** On local drives.
```

*Validation Error:* Turn `t3` immediately follows `t1`. Turn indices must be strictly contiguous without omissions (`expected turn t2, found t3`).

### 3.3 Example 3: Free-Form Vision Document Snippet

```markdown
# Product Vision: Next-Generation Clinical Dispatch

## Operational Resilience

Clinical coordinators require real-time visibility into active ambulatory units. The dispatch engine must maintain zero data loss during regional network partitions while preventing duplicate vehicle dispatches.
```

*Note:* Vision documents are segmented into sentence spans by the Stage 1 atomizer, with grounding anchors bound to section headings (e.g. `section: "Operational Resilience"`) and exact substrings.
