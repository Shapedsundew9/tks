# Intent Elicitation Evaluation Corpus (Track E-B)

> **Schema Version:** `tks-transcript/v1` (transcripts) / `tks-elicitation-labels/v1` (labels)  
> **Normative Specifications:** [Transcript Format Specification](../../../docs/vision/elicitation-transcript-format.md), [Evaluation Framework](../../../docs/vision/elicitation-evaluation.md)

---

## 1. Corpus Index

This corpus contains 10 synthetic conversational transcripts across diverse domains, plus labels for the dogfood engineering vision document [docs/vision/vision.md](../../../docs/vision/vision.md).

| Session ID | File Name | Domain | Participants | Turns | Traps | Gaps | Cycle |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| **T01** | `T01-capacity.md` | Team Capacity Planning | Priya (stakeholder), Marcus (facilitator) | 20 | 4 | 2 | 1 |
| **T02** | `T02-clinic-scheduling.md` | Clinic Appointment Scheduling | Dr. Sarah (stakeholder), Alex (facilitator) | 20 | 4 | 2 | 1 |
| **T03** | `T03-community-library.md` | Community Tool & Book Library | Tom (stakeholder), Nina (facilitator) | 18 | 3 | 2 | 1 |
| **T04** | `T04-logistics-dispatch.md` | Refrigerated Freight Logistics | Elena (stakeholder), Marcus (stakeholder), Dan (facilitator) | 22 | 4 | 2 | 1 |
| **T05** | `T05-budgeting-app.md` | Household Multi-Currency Budgeting | Mei (stakeholder), Chris (facilitator) | 20 | 4 | 2 | 1 |
| **T06** | `T06-school-timetable.md` | High School Timetable Scheduling | Mr. Davis (stakeholder), Dr. Chen (stakeholder), Rachel (facilitator) | 22 | 4 | 2 | 1 |
| **T07** | `T07-field-maintenance.md` | Wind Turbine Field Maintenance | Lars (stakeholder), Karen (facilitator) | 20 | 4 | 2 | 1 |
| **T08** | `T08-museum-visitor.md` | Museum Visitor Experience & Audio | Maya (stakeholder), Jordan (facilitator) | 20 | 3 | 2 | 1 |
| **T09** | `T09-volunteer-network.md` | Disaster Relief Volunteer Network | Carlos (stakeholder), Sam (facilitator) | 20 | 4 | 2 | 1 |
| **T10** | `T10-capacity-c2.md` | Team Capacity Planning (Cycle 2) | Priya (stakeholder), Marcus (facilitator) | 18 | 3 | 1 | 2 |
| **Dogfood** | `vision.md` | Knowledge Substrate Technical Vision | Lead Architect (stakeholder) | N/A (Doc) | N/A | 3 | N/A |

---

## 2. Trap Matrix

Every synthetic transcript plants $\ge 3$ intentional linguistic and structural traps. Across the corpus, every trap type appears $\ge 2$ times:

| Session ID | `ANALOGY` | `UI_METAPHOR` | `RETRACTION` | `MULTI_TURN_COREFERENCE` | `VENTING_NO_GOAL` | `CROSS_TURN_CONTRADICTION` | `IMPLICIT_SOFTGOAL` | `EXPLICIT_ANTIGOAL` | `BOUNDARY_BY_NEGATION` | `MISSING_STAKEHOLDER` | `FACILITATOR_LEADING` | Total Traps |
| :--- | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :---: |
| **T01** | ✓ | | | ✓ | | | | | ✓ | | ✓ | 4 |
| **T02** | | ✓ | ✓ | | ✓ | | | ✓ | | | | 4 |
| **T03** | ✓ | | | | | | ✓ | | | ✓ | | 3 |
| **T04** | | ✓ | ✓ | | | ✓ | | | | | ✓ | 4 |
| **T05** | | | | ✓ | ✓ | | ✓ | | ✓ | | | 4 |
| **T06** | | | | | | ✓ | | ✓ | | ✓ | ✓ | 4 |
| **T07** | ✓ | ✓ | ✓ | | | | | ✓ | | | | 4 |
| **T08** | | | | | ✓ | | ✓ | | ✓ | | | 3 |
| **T09** | ✓ | | ✓ | ✓ | | | | | | ✓ | | 4 |
| **T10** | | ✓ | | | | | | | ✓ | | ✓ | 3 |
| **Total Occurrences** | **4** | **4** | **4** | **3** | **3** | **2** | **3** | **3** | **4** | **3** | **4** | **37** |

---

## 3. How to Add a New Fixture

To add a new transcript to the evaluation corpus:

1. **Create Transcript (`tests/fixtures/elicitation/transcripts/<SESSION_ID>-<slug>.md`):**
   * Use YAML frontmatter declaring `format: tks-transcript/v1`, `session_id`, `cycle`, `source`, and `participants`.
   * Ensure 15–40 turns. Contiguous identifiers `t1..tN` in format `**[tN] Name (role):** utterance`.
   * Plant $\ge 3$ traps matching defined trap taxonomies.
2. **Compute SHA-256 Digest:**

   ```bash
   sha256sum tests/fixtures/elicitation/transcripts/<SESSION_ID>-<slug>.md
   ```

3. **Create Labels (`tests/fixtures/elicitation/labels/<SESSION_ID>-<slug>.labels.json`):**
   * Reference the computed SHA-256 digest in `source.sha256`.
   * Set `adjudication.status` to `"DRAFT"`.
   * Define contiguous episode turn ranges covering `t1..tN`.
   * Extract goals with verbatim quotes copied from utterance texts.
   * Provide $\ge 4$ entailment test cases including $\ge 1$ phantom case.
4. **Validate Integrity:**

   ```bash
   cargo test --test elicitation_fixtures
   ```
