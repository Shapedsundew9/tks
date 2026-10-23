# Implementation Plan: Intent Elicitation Track — E-A (Conversational Elicitation & Transcription)

> **Audience:** an implementation agent working in this repository.
> **Execution scope right now:** **EA-0 spike only.** Do not begin EA-1 onward until the user approves
> the EA-0 results.
> **Status:** Design decisions D-EA-1 … D-EA-5 accepted (architecture decisions D-130 … D-134,
> constraint C-28). Implementation not yet started.

---

## 1. Problem Statement

E-B provides deterministic rails for extracting grounded goals and open questions from transcripts
and vision documents. It does not help a stakeholder produce a useful transcript in the first place.
Initial intent is often spoken, incomplete, tentative, and discovered through conversation. Requiring
users to prepare a transcript manually creates friction and can lose the clarifications that emerge
while exploring a problem.

E-A is the upstream conversation and transcription experience. It should help a stakeholder articulate
their own intent, preserve uncertainty and corrections, establish scope boundaries, and emit a
reviewable transcript that E-B can ingest without special-case parsing.

The experience must not turn TKS into a hidden LLM host, send speech to a cloud service, silently
rewrite a stakeholder's words, or store microphone audio.

## 2. Scope and Relationship to E-B

E-A is the conversational intake half of the Intent Elicitation Track. Its only output is an ordinary
`tks-transcript/v1` Markdown document. Audio is never stored.

```text
 [stakeholder] ──speaks/types──▶ [E-A session]
                                      │
                 ┌────────────────────┼─────────────────────┐
                 ▼                    ▼                     ▼
       local in-process ASR    Socratic dialogue     user review/edit
       (audio in memory only)  (external cognition)
                 └────────────────────┼─────────────────────┘
                                      ▼
                         tks-transcript/v1 Markdown
                                      │
                                      ▼
                              E-B extraction
                  goals, grounding, gaps, next questions
```

E-A covers browser-based conversation capture, local transcription, transcript review, and export.
E-B continues to own transcript parsing, goal extraction, entailment checks, gap detection, and goal
staging. E-A does not create or stage `GOAL` graph nodes.

## 3. Governing Principles

1. **Bring your own cognition.** Follow D-124, C-25 and C-27: LLM cognition (Socratic questioning,
   clean-up suggestions) runs in the user's agent or in an optional local LLM service outside the
   `tks serve` core path. No LLM SDK or LLM inference may be added to TKS.
2. **Speech recognition is a local internal tool.** ASR is not an LLM. A permissively licensed,
   non-LLM speech recognition model may run inside TKS (D-131, C-28). No cloud speech service,
   including browser speech-recognition APIs that forward audio to a vendor, may be used.
3. **No audio storage.** Microphone audio exists only in volatile memory long enough to be
   transcribed, then is dropped. It is never written to disk, database, logs, caches, browser storage,
   or exports, and there is no "save audio" option (D-132, C-28).
4. **Stakeholder agency.** The stakeholder controls when to record, pause, resume, edit, export, or
   discard. The facilitator asks neutral questions and must not invent requirements.
5. **Visible provenance.** Use the transcript format's `source` field for transcript-level
   provenance (`manual` or `asr`). The current format does not encode provenance per turn; do not
   invent per-turn fields without an approved format change. Human-reviewed text is authoritative;
   do not silently replace a stakeholder's words with model-generated prose (D-133).
6. **E-B-compatible output.** Export only valid `tks-transcript/v1` Markdown: declared participants,
   contiguous `tN` turn IDs, allowed roles, and one paragraph per turn. Preserve cycle and session
   identity when continuing a prior elicitation (D-134).
7. **Graceful degradation.** Typed/manual transcript entry remains available if microphone access,
   the ASR model, MCP sampling, or local LLM inference is unsupported or unavailable.

## 4. Decisions

### 4.1 Constraints Already Established

| Constraint | Source | Implication for E-A |
| --------- | ------ | ------------------- |
| LLM cognition is external to `tks serve`; optional Ollama offload is outside the daemon. | D-124 / C-25 / C-27 | Socratic questioning uses an external agent/client or a separate, explicitly configured local LLM service. |
| Track placement and names are E-A and E-B, before Phase 5. | D-126 | Do not renumber Phase 4 or fold E-A into E-B. |
| Transcript output is `tks-transcript/v1`. | D-128 and [transcript specification](../vision/elicitation-transcript-format.md) | E-A must satisfy the normative frontmatter and turn rules. |
| E-B supplies grounded gap questions for a later cycle. | [E-B plan](./intent-elicitation-eb-plan.md) | E-A accepts prior gap questions as optional session context without treating them as established facts. |

### 4.2 Accepted Decisions

These are recorded as architecture decisions **D-130 … D-134** and constraint **C-28** in
[architecture.md](../vision/architecture.md). EA-0 gathers implementation evidence within these
decisions; it does not reopen them unless a decision's "Reopen If" condition is met.

| Plan ID | ADR ID | Decision | Evidence still required in EA-0 |
| ------- | ------ | -------- | ------------------------------- |
| D-EA-1 | D-130 | Socratic conversation cognition sits behind a small conversation-provider boundary supporting MCP sampling and out-of-process local Ollama offload. Ship whichever path EA-0 proves viable (both if practical). Typed/manual operation is always available. | Client support for MCP sampling, consent flow, latency, failure handling, setup cost. |
| D-EA-2 | D-131 | Speech recognition is local only, with no cloud services. Use the most accurate permissively licensed (Apache-2.0 or MIT), non-LLM English model from the Hugging Face Open ASR Leaderboard, running inside TKS behind an optional Cargo feature. Current selection: **Cohere Transcribe** (`CohereLabs/cohere-transcribe-03-2026`). Fallback: **Whisper large-v3-turbo** (`openai/whisper-large-v3-turbo`, MIT). | Rust inference runtime, hardware fit, latency, WER on a local English set, silence handling, gated-download handling. |
| D-EA-3 | D-132 | No audio storage of any kind. Audio is held in volatile memory only for transcription and then dropped; there is no option to save it. | Data-flow verification of every buffer, error path, and log. |
| D-EA-4 | D-133 | Human-reviewed utterance text is authoritative. ASR-derived transcripts declare `source: asr`; audio is never needed for E-B ingestion. | Round-trip test with the transcript parser and E-B grounding. |
| D-EA-5 | D-134 | Follow-up cycles keep a stable `session_id`, increment `cycle`, and show imported E-B gap questions as prompts rather than confirmed intent. | Cycle-2 transcript validation test. |

**ASR model selection rationale (D-131).** Eligibility requires all three of: an Apache-2.0 or MIT
licence, a dedicated speech architecture with no LLM decoder or backbone, and local execution.
Cohere Transcribe (2B parameters, Apache-2.0) is a Conformer encoder with a lightweight Transformer
decoder trained from scratch, so it is a dedicated speech recognizer rather than an LLM. Its model
card reports 5.42% average English WER over eight Open ASR Leaderboard sets. Several newer entries
average over seven sets because they omit TED-LIUM, so the like-for-like figure for Cohere is
**5.84%** (mean of its published AMI, Earnings22, GigaSpeech, LibriSpeech clean/other, SPGISpeech
and VoxPopuli scores).

Models with lower or comparable scores were checked against each criterion using their model cards
and `config.json` files (October 2026):

| Model | Licence | Architecture | 7-set avg WER | Eligible |
| ----- | ------- | ------------ | ------------- | -------- |
| MOSS-Transcribe-preview-2B | Apache-2.0 | Qwen3-Omni audio encoder + **Qwen3-1.7B-base LLM backbone** (`MossForCausalLM`); also RL-tuned on leaderboard training splits | 4.87% | No — LLM |
| ARK-ASR-3B | Apache-2.0 | Whisper-style encoder + MLP adapter + **3B Qwen decoder LLM** (`ArkasrForConditionalGeneration`) | 5.04% | No — LLM |
| Zipformer CR-CTC Transducer XL 290M (`soundsgoodai`) | **CC-BY-NC-4.0** | Zipformer transducer/CTC (non-LLM) | 5.56% | No — non-commercial licence |
| Granite Speech 4.1 2B (and 2B-NAR) | Apache-2.0 | LLM decoder (the NAR variant edits CTC output with a bidirectional LLM) | 5.65% | No — LLM |
| Canary-Qwen-2.5B | CC-BY-4.0 | FastConformer encoder + Qwen3-1.7B decoder | — | No — LLM and licence |
| Qwen3-ASR-1.7B, Voxtral Mini 4B Realtime | Apache-2.0 | LLM decoder | — | No — LLM |
| **Cohere Transcribe** | **Apache-2.0** | **Conformer encoder + lightweight Transformer decoder (non-LLM)** | **5.84%** | **Yes — selected** |
| Parakeet TDT 0.6B v3, Kyutai STT 2.6B | CC-BY-4.0 | Non-LLM | 6.32% / 6.40% (published) | No — licence, and less accurate |
| Whisper large-v3-turbo | MIT | Encoder-decoder (non-LLM) | Higher than Cohere | Yes — fallback |

Cohere Transcribe is therefore the most accurate eligible model: every model ahead of it either
embeds an LLM or has a non-commercial licence. Known caveats from its model card: the Hugging Face
repository is gated behind a contact-information agreement, there are no timestamps or diarization,
and it tends to transcribe silence, so it needs a voice-activity gate. A community Rust port exists
and is an EA-0 runtime candidate. Whisper large-v3-turbo is the fallback if the primary model fails
the EA-0 runtime or hardware gate.

## 5. User Journey

1. The user opens a new elicitation session or continues a prior session, optionally supplying E-B gap
   questions from the previous cycle.
2. The user names the session, identifies participants and roles, and chooses typed or microphone
   input. The UI states that speech is transcribed locally and audio is not stored.
3. The facilitator asks one neutral, open question at a time. It can ask about the problem, affected
   stakeholders, desired outcomes, constraints, examples, uncertainty, and explicit boundaries
   (including what the solution is not).
4. The user can pause, correct the transcript, mark uncertain ASR spans, retract or clarify a
   statement, and distinguish stakeholder speech from facilitator or agent turns.
5. Before export, the user reviews the full conversation and transcript metadata. The application
   validates the transcript locally and reports actionable errors.
6. The user saves or downloads a `.md` transcript and may then pass it to E-B. No audio exists to
   retain.

## 6. Work Packages

General execution rules:

- Read `.github/copilot-instructions.md`, `.shared/ai/core/service-instructions.md`, and
  `.shared/ai/core/common-instructions.md` before implementation.
- Execute EA-0 first and stop for user approval before EA-1 onward.
- Do not add an LLM dependency or LLM inference to TKS. The D-131 ASR model is the only model
  inference permitted in-process, and only behind an optional Cargo feature.
- Never persist microphone audio or send it to any network service (C-28). There is no opt-in
  exception.
- Model weights are downloaded once to a local cache. Read any Hugging Face token needed for gated
  access from the environment (for example `$HF_READ_TOKEN`); never commit, log, or embed it.
- Reuse existing gateway authentication, web delivery, and UI patterns where appropriate; do not add
  a second web stack without evidence from EA-0.
- Keep the transcript specification authoritative. If it must change, update its examples and
  validators together and obtain approval for any format compatibility impact.
- After Markdown changes, run `npx markdownlint-cli2 --fix "**/*.md"` and inspect its complete
  worktree impact; do not edit `.markdownlint-cli2.jsonc`.
- Keep each work package independently verifiable. Do not commit unless requested.

### 6.1 EA-0 — Conversation-Provider, ASR-Runtime, and Audio Data-Flow Spike (first; stop gate)

**Goal:** gather the implementation evidence listed in §4.2 before building product features. This
is a time-boxed investigation and prototype, not production UI.

**Conversation provider (D-130).** Compare:

1. MCP sampling with a compatible external agent/client.
2. Local Ollama offload through a separate service or process outside `tks serve`.
3. Typed/manual conversation as the no-model baseline and resilience path.

For each path, record client compatibility, consent flow, latency, setup requirements, failure
handling, and how facilitator turns are returned to the session. Do not treat the availability of
local Ollama for embeddings as evidence that a conversational model is configured or appropriate.

**ASR runtime (D-131).** For Cohere Transcribe, and Whisper large-v3-turbo as the comparison and
fallback:

1. Confirm the licence and gated-access terms on the model card, and record the model revision.
2. Select a Rust inference path (for example ONNX Runtime via `ort`, `candle`, or an existing Rust
   port) and confirm it builds behind an optional Cargo feature without affecting default builds.
3. Measure load time, memory, real-time factor, and end-of-utterance latency on CPU and on the
   devcontainer GPU, given that the RTX 3080's 8 GiB is shared with Ollama and Blender.
4. Score WER and correction effort on a small, non-sensitive or explicitly consented English audio
   set, including disfluent and noisy speech. Only local processing is permitted.
5. Choose and test a voice-activity gate that suppresses transcription of silence.

**Audio data flow (D-132).** Choose the browser-to-TKS audio transport (for example an authenticated
WebSocket or chunked POST to the local daemon). Map every place audio exists (browser capture buffers,
transport, server-side decode and inference buffers, error and panic paths) and show how each is
bounded and dropped.

**EA-0 acceptance criteria:**

- [ ] A short spike report documents tested environments, evidence, limitations, and the chosen
      conversation provider path(s) and ASR runtime.
- [ ] The MCP-sampling and local-offload paths are compared with typed/manual operation; unsupported
      client capabilities are stated rather than assumed.
- [ ] Cohere Transcribe is either confirmed or replaced by the fallback with recorded evidence; WER is
      reported on a documented test set using a reproducible scoring method.
- [ ] The ASR prototype builds behind an optional Cargo feature, runs fully offline after model
      download, and makes no network calls during transcription.
- [ ] A data-flow table shows that audio is never persisted or logged, including on error paths.
- [ ] Any LLM prototype remains outside the `tks serve` process and is optional.
- [ ] The implementation agent stops and presents the results for user approval; no EA-1+ work
      begins before that approval.

### 6.2 EA-1 — Session Model and Conversation Lifecycle

**Goal:** define a user-controlled session lifecycle that supports new and continuing elicitation
cycles and preserves speaker attribution.

Deliverables:

- Session metadata mapped to the transcript frontmatter: title, stable `session_id`, positive
  `cycle`, date, language, transcript-level source, and participants with roles.
- Draft states for setup, active conversation, paused, review, exported, and discarded sessions.
- Optional import of E-B gap questions with their provenance and a clear distinction from stakeholder
  statements.
- Recovery behavior for browser refresh or connection loss that preserves transcript text only, never
  audio.

**EA-1 acceptance criteria:**

- [ ] A user can start, pause, resume, review, export, or discard a text-only session.
- [ ] Continuing a session keeps its `session_id`, increments `cycle`, and records the session date.
- [ ] Every turn is attributed to a declared participant and an allowed role.
- [ ] Imported gap questions remain facilitator context and are not emitted as stakeholder turns.
- [ ] Lifecycle tests cover refresh/connection-loss behavior and ensure discarded drafts are not
      presented as successful exports.

### 6.3 EA-2 — Browser Conversation and Microphone Capture

**Goal:** provide accessible typed and microphone input through the existing TKS web surface, using
the transport selected in EA-0.

Deliverables:

- Clear microphone permission request, active-recording indicator, pause/stop controls, and visible
  error states.
- Typed turn entry available regardless of microphone permission or ASR availability.
- Accessible keyboard operation and labels for all recording and transcript controls.
- A visible statement that audio is transcribed locally and not stored.

**EA-2 acceptance criteria:**

- [ ] Microphone permission is requested only after an explicit user action.
- [ ] Recording state is unambiguous, and audio capture stops on stop, discard, navigation away, or
      loss of permission.
- [ ] Denied or unavailable microphone access leaves typed input usable and reports the issue.
- [ ] Automated browser tests cover permission grant/deny, stop/discard, and empty-input cases.
- [ ] No audio is written to browser storage, server disk, the database, or logs; tests verify this.

### 6.4 EA-3 — Local English Speech Recognition and Transcript Drafting

**Goal:** produce editable transcript turns from microphone input using the in-process D-131 model.

Deliverables:

- An ASR component behind an optional Cargo feature, using the runtime and model revision approved
  in EA-0, with the voice-activity gate in front of it.
- ASR output marked as a draft requiring stakeholder review.
- Turn segmentation, participant attribution, timestamps where available, and uncertainty handling.
- Explicit errors for a missing model, an unsupported audio format, and recognition failure. There
  is no cloud fallback; if local ASR is unavailable, typed input remains.

**EA-3 acceptance criteria:**

- [ ] ASR draft turns can be edited, split, joined, and attributed before export.
- [ ] The UI distinguishes unreviewed ASR output from typed or manually corrected content during the
      session; exported per-turn provenance is not claimed because the current format has no such
      field.
- [ ] Evaluation reports WER on the agreed English test set, including noisy and disfluent samples;
      the target threshold is set from EA-0 evidence and approved before implementation.
- [ ] Silence and background noise do not produce fabricated turns.
- [ ] Failure or timeout does not silently discard captured text or claim a successful transcript.
- [ ] Tests verify that transcription makes no network calls and that audio buffers are dropped after
      each utterance is transcribed.

### 6.5 EA-4 — User-Reviewed Transcript and E-B Interoperability

**Goal:** make the reviewed conversation a valid, portable, E-B-ready `tks-transcript/v1` document.

Deliverables:

- Preview and edit of frontmatter, participants, turns, and optional timestamps.
- Deterministic local validation before export, using the current normative transcript rules.
- Export as Markdown to a user-selected location or browser download, with no hidden server-side
  publication.
- Optional handoff instructions or link to the E-B workflow; E-B remains a separate operation.

**EA-4 acceptance criteria:**

- [ ] Exported documents pass the existing transcript parser and conform to
      [elicitation-transcript-format.md](../vision/elicitation-transcript-format.md).
- [ ] Turn IDs are contiguous and unique; every participant and role is declared and valid.
- [ ] Transcript-level provenance uses `source: asr` for ASR-produced transcripts or `source: manual`
      for manually authored transcripts; mixed-source handling is documented without adding
      unapproved format fields.
- [ ] Invalid metadata, unsupported roles, empty utterances, and malformed turns are explained before
      export with an actionable correction.
- [ ] An exported transcript can be opened by E-B without manual format repair.

### 6.6 EA-5 — Socratic Facilitation and Cycle Continuity

**Goal:** help a stakeholder explore intent without leading them toward a predetermined solution,
using the conversation provider(s) approved in EA-0.

Deliverables:

- Facilitator behavior for problem exploration, desired outcomes, stakeholder needs, constraints,
  trade-offs, examples, uncertainty, and boundary scoping by negation.
- One question at a time, with user controls to skip, rephrase, answer later, or end the session.
- Optional prior-cycle gap questions presented as prompts, not as asserted facts.
- Conversation context bounded to the current user-authorized session and selected prior-cycle
  material.

**EA-5 acceptance criteria:**

- [ ] A prompt/behavior test suite covers neutral questions, explicit boundaries, uncertainty,
      corrections, retractions, and facilitator-led wording.
- [ ] The facilitator does not state that a tentative response is confirmed intent without explicit
      stakeholder confirmation.
- [ ] Facilitator and agent turns are attributed with the correct allowed roles.
- [ ] The user can inspect and edit all generated facilitator turns before transcript export.
- [ ] A cycle-2 test imports selected prior gaps and produces an E-B-valid transcript with the same
      `session_id` and incremented `cycle`.

### 6.7 EA-6 — Privacy, Security, and Operational Hardening

**Goal:** verify that the implemented capture, ASR, and conversation-provider paths match D-130 …
D-134 and C-28.

Deliverables:

- Threat and data-flow review for microphone permissions, browser buffers, audio transport, ASR
  inference buffers, session drafts, exports, and logs.
- Explicit conversation-provider and model-download configuration, with credential handling
  consistent with repository conventions.
- A user-facing explanation that audio is transcribed locally, is never stored, and never leaves the
  machine.

**EA-6 acceptance criteria:**

- [ ] Tests and review confirm that no code path persists or logs audio, including error and panic
      paths, and that transcripts are not logged with secrets.
- [ ] The audio endpoint requires authentication and accepts only bounded, short-lived streams.
- [ ] Model-download and provider credentials are never embedded in browser assets, returned by APIs,
      logged, or written to source control.
- [ ] Network and provider errors are visible and never result in success-shaped fallback behavior.
- [ ] A reviewer verifies consent, access control, and the no-audio-storage guarantee before release.

### 6.8 EA-7 — Evaluation, Documentation, and Release Gate

**Goal:** validate the end-to-end E-A-to-E-B journey and document supported behavior and limitations.

Evaluation should cover transcript validity, transcription quality, correction effort, successful
completion of an elicitation session, user control over capture, ASR and provider failures, and
cycle continuity. Report each metric and its test conditions; do not treat transcript conformance as
a proxy for conversational quality.

**EA-7 acceptance criteria:**

- [ ] Automated tests cover session lifecycle, transcript validation, ASR and provider failures, and
      privacy-sensitive paths.
- [ ] A dogfood session is reviewed by a human and exported as a valid `tks-transcript/v1` document,
      then consumed by E-B.
- [ ] Provisional thresholds for ASR quality, correction effort, and completion are calibrated using
      the agreed test set and approved by the user.
- [ ] User documentation explains prerequisites, enabling the ASR feature and downloading the model,
      microphone permission, the local-only and no-audio-storage guarantees, transcript
      review/export, and E-B handoff.
- [ ] The ASR model's licence and attribution are recorded alongside the other third-party notices.
- [ ] Known unsupported clients, languages, and hardware limitations are documented.

## 7. Risks and Mitigations

| Risk | Mitigation |
| ---- | ---------- |
| MCP sampling support varies between clients. | Make support a measured EA-0 result; keep the provider boundary and typed/manual operation; avoid depending on undocumented client behavior. |
| The 2B ASR model is too slow on CPU or does not fit the shared 8 GiB GPU. | Measure in EA-0; fall back to Whisper large-v3-turbo; keep ASR behind an optional Cargo feature so default builds are unaffected. |
| No mature Rust runtime exists for the selected model. | Evaluate ONNX export, `candle`, and existing Rust ports in EA-0; use the fallback model if none is viable. |
| The model transcribes silence or noise as speech. | Put a voice-activity gate in front of ASR and test silence and noise cases in EA-3. |
| Gated model download requires a Hugging Face token. | Read the token from the environment at download time only; never commit, log, or embed it. |
| Audio is inadvertently retained or exposed. | Hold audio in bounded in-memory buffers only, add tests over error paths and logs, and review in EA-6. |
| Automatic clean-up changes stakeholder meaning and breaks verbatim grounding. | Treat recognition and clean-up as editable drafts; user-reviewed transcript text is authoritative. |
| Facilitator questions lead the stakeholder or convert analogies into requirements. | Use neutral, versioned facilitator instructions and test leading-question, retraction, and boundary cases. |
| A newer model overtakes the selection. | Revisit D-131 when a permissively licensed, non-LLM model beats it on comparable leaderboard averages and fits the hardware budget. |
| E-B format changes independently. | Keep `tks-transcript/v1` as the integration contract and run E-B parser conformance tests before release. |

## 8. Dependency and Exit Gates

| Work package | Depends on | Exit gate |
| ------------ | ---------- | --------- |
| EA-0 | D-130 … D-134, C-28, and the E-B transcript contract | Spike evidence presented; user approves provider path(s), ASR runtime and model, and audio data flow. |
| EA-1 | EA-0 | Session lifecycle and metadata design approved; cycle/session rules testable. |
| EA-2 | EA-0, EA-1 | Browser capture works with explicit consent and typed fallback; no audio persisted. |
| EA-3 | EA-0, EA-2 | Local ASR output is editable, provenance-aware, and meets the approved quality threshold. |
| EA-4 | EA-1, EA-3 | Exported Markdown validates and is accepted by E-B. |
| EA-5 | EA-0, EA-1, EA-4 | Facilitator behavior is reviewable, non-leading, and cycle-aware. |
| EA-6 | EA-0 through EA-5 | Privacy/security review approves the implemented data flow. |
| EA-7 | EA-1 through EA-6 | End-to-end acceptance and documentation complete; user approves release. |

## 9. Definition of Done

E-A is complete when a user can conduct or manually enter an intent-elicitation session, review and
correct its transcript, export a valid `tks-transcript/v1` document, and pass it into E-B. The
implementation transcribes speech locally with a permissively licensed non-LLM model, never stores
or transmits audio, keeps LLM cognition outside TKS, supports recovery and typed fallback, passes its
documented quality and privacy gates, and does not silently invent or rewrite stakeholder intent.
