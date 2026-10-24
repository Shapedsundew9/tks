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
`tks-transcript/v1` Markdown document. Audio is never stored or transmitted across network interfaces.

```text
 [stakeholder] ──speaks/types──▶ [Client Browser UI]
                                      │
                 ┌────────────────────┴─────────────────────┐
                 ▼                                          ▼
       client-side streaming ASR                    user review / edit
       (WASM/WebGPU in WebWorker)                   (interactive dictation)
                 │                                          │
                 └────────────────────┬─────────────────────┘
                                      │ UTF-8 JSON turn
                                      ▼
                                 [tks serve]
                                      │
                                      ├────▶ Socratic dialogue (external LLM/Ollama)
                                      ▼
                          tks-transcript/v1 Markdown
                                      │
                                      ▼
                                E-B extraction
                    goals, grounding, gaps, next questions
```

E-A covers browser-based conversation capture, local client-side streaming transcription, transcript review, and export.
E-B continues to own transcript parsing, goal extraction, entailment checks, gap detection, and goal
staging. E-A does not create or stage `GOAL` graph nodes.

## 3. Governing Principles

1. **Bring your own cognition.** Follow D-124, C-25 and C-27: LLM cognition (Socratic questioning,
   clean-up suggestions) runs in the user's agent or in an optional local LLM service outside the
   `tks serve` core path. No LLM SDK or LLM inference may be added to TKS.
2. **Speech recognition is a client-side interaction tool, not a server dependency.** ASR is executed
   entirely client-side in the browser via WebAssembly or WebGPU using lightweight, permissively licensed
   (Apache-2.0 or MIT) models. Microphone audio is processed and dropped in client memory; raw audio never
   crosses any network interface (including loopback to `tks serve`). `tks serve` remains a lean Rust
   daemon free of CUDA/C++ audio frameworks (D-131, C-28).
3. **No audio storage or transmission.** Microphone audio exists only in volatile client-side browser
   memory long enough to be transcribed, then is dropped. It is never transmitted across any network
   interface, and never written to disk, database, logs, caches, browser storage (`IndexedDB`,
   `localStorage`, `CacheStorage`), or exports (D-132, C-28).
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
| D-EA-2 | D-131 | Speech recognition is client-side only, with no cloud services or network audio transmission. **Confirmed & Approved (2026-10-24)**: use **Sherpa-ONNX streaming Zipformer** (`k2-fsa/sherpa-onnx-streaming-zipformer-en-2023-06-26`, Apache-2.0, 67.6 MB INT8) via WebAssembly in the client browser. Fallback: Transformers.js / Whisper-tiny.en (MIT, 39 MB). | Proven in Spike EA-0.2 (`spike-ea0-2-results.md`): sub-100 ms streaming latency, 68 MB download, 3.9% clean WER, 0/4 silence hallucinations, 0% server CPU/VRAM overhead. |
| D-EA-3 | D-132 | Zero audio storage or network transmission. Audio exists only in volatile client-side browser memory long enough to be transcribed, then is dropped. Audio never crosses the network to `tks serve`. | Client-side memory lifecycle and network trace verification (0 audio bytes). |
| D-EA-4 | D-133 | Human-reviewed utterance text is authoritative. ASR-derived transcripts declare `source: asr`; audio is never needed for E-B ingestion. | Round-trip test with the transcript parser and E-B grounding. |
| D-EA-5 | D-134 | Follow-up cycles keep a stable `session_id`, increment `cycle`, and show imported E-B gap questions as prompts rather than confirmed intent. | Cycle-2 transcript validation test. |

**ASR model selection rationale & architectural pivot (D-131).**
The original EA-0 baseline investigation evaluated Cohere Transcribe (2B, Apache-2.0) and Whisper large-v3-turbo (MIT) on Candle as server-side batch models. The results in [`spike-ea0-results.md`](../vision/spike-ea0-results.md) demonstrated that while WER accuracy was high (Cohere 2.1% clean, 9.1% disfluent), running a 2B parameter model on the server daemon incurred 9.4–14.4 GiB RAM, required heavy CUDA toolchains, caused fatal GPU memory contention with local facilitator LLMs (`qwen3:8b`), and produced an unacceptable 4.9 s – 8.8 s end-of-utterance latency.

On 2026-10-24, user approval reopened D-131 to align with interactive dictation interfaces (analogous to VS Code Speech and Antigravity CLI voice). Following the approved reframe ([`spike-ea0-streaming-proposal.md`](spike-ea0-streaming-proposal.md)), Spike EA-0.2 benchmarked client-side streaming candidates on the exact 60-clip EA-0 testset ([`spike-ea0-2-results.md`](../vision/spike-ea0-2-results.md)).

**Outcome:** User approved **Sherpa-ONNX streaming Zipformer** as the primary speech recognition engine for Intent Elicitation:

| Model / Framework | Licence | Architecture | Footprint | Streaming Mechanism | Status |
| ----- | ------- | ------------ | --------- | ------------------- | ------ |
| **Sherpa-ONNX Web** (`k2-fsa`) | **Apache-2.0** | **Streaming Zipformer (INT8)** | **67.6 MB weights** | **Frame-by-frame streaming transducer (<100ms)** | **Selected & Confirmed** |
| Transformers.js / ORT Web | MIT | Quantized Whisper `tiny.en` | ~39 MB weights | Sliding-window / WebGPU chunked | Fallback candidate |
| whisper.cpp WASM | MIT | Quantized GGML Whisper `tiny.en` | ~75 MB weights | Sliding-window streaming | Fallback candidate |

This pivot eliminates all CUDA and C++ dependencies from `tks serve`, ensures 100% of GPU VRAM remains available for Ollama, and guarantees that microphone audio never leaves the client browser.

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

### 6.1 EA-0 / EA-0.2 — Conversation-Provider and Streaming ASR Spike (COMPLETED & APPROVED)

**EA-0 Baseline Outcome (Completed 2026-10-23):** Gathered implementation evidence for provider offload and server-side batch ASR ([`spike-ea0-results.md`](../vision/spike-ea0-results.md)). Confirmed out-of-process Ollama offload for Socratic dialogue (D-130). Proved that server-side batch inference (Cohere Transcribe 2B / Whisper Large on Candle) introduces severe memory overhead (9.4–14.4 GiB RAM), requires heavy CUDA toolchains, breaks GPU co-tenancy with Ollama, and results in unacceptable 4.9 s – 8.8 s turn latency.

**EA-0.2 Client-Side Streaming Spike (Completed & Approved 2026-10-24):** Authorized via [`spike-ea0-streaming-proposal.md`](spike-ea0-streaming-proposal.md) and executed via [`scripts/spikes/ea0_streaming/run_benchmark.js`](../../scripts/spikes/ea0_streaming/run_benchmark.js). Evaluated Sherpa-ONNX streaming Zipformer and Transformers.js Whisper-tiny on the exact 60-clip EA-0 benchmark ([`spike-ea0-2-results.md`](../vision/spike-ea0-2-results.md)). User approved **Sherpa-ONNX Zipformer** (`k2-fsa/sherpa-onnx-streaming-zipformer-en-2023-06-26`, Apache-2.0, 67.6 MB INT8) via WebAssembly. Stop gate is passed; implementation of EA-1+ authorized.

**EA-0 / EA-0.2 acceptance criteria:**

- [x] A short spike report documents tested environments, evidence, limitations, and the chosen
      conversation provider path(s) and baseline ASR runtime (`spike-ea0-results.md`).
- [x] The MCP-sampling and local-offload paths are compared with typed/manual operation; unsupported
      client capabilities are stated rather than assumed.
- [x] Baseline batch models benchmarked; hardware conflict with Ollama identified, motivating D-131 reopening.
- [x] EA-0.2 benchmark harness evaluates candidate client-side streaming engines (Sherpa-ONNX Web, Transformers.js) on the 60 EA-0 test clips.
- [x] Time-to-first-partial-token (~100 ms) and end-of-utterance latency (<100 ms) confirm sub-100 ms responsiveness.
- [x] Network trace verifies 0 audio bytes sent over network to `tks serve` (turns sent as UTF-8 JSON text).
- [x] Stop gate: results presented in `spike-ea0-2-results.md` and approved by user on 2026-10-24.

### 6.2 EA-1 — Session Model, Metadata & Conversation Lifecycle (tks serve)

**Goal:** define and implement the server-side session state model and conversation lifecycle in `tks serve` that manages active elicitation sessions, attributes turns, coordinates external Socratic facilitation, and stores draft transcripts until export.

Deliverables:

- Session metadata adhering to `tks-transcript/v1` normative frontmatter: title, stable `session_id`, positive `cycle`, date, language, `source: asr` (or `manual`), and declared `participants` with allowed roles (`stakeholder`, `facilitator`).
- Elicitation REST and SSE routes on `tks serve` (under `src/gateway/routes/elicitation.rs`):
  - `POST /api/v1/elicitation/sessions`: initialize session or continue prior cycle.
  - `GET /api/v1/elicitation/sessions/{id}`: retrieve active session status and turn history.
  - `POST /api/v1/elicitation/sessions/{id}/turns`: receive finalized stakeholder text turns (UTF-8 JSON).
  - `GET /api/v1/elicitation/sessions/{id}/events`: Server-Sent Events (SSE) stream pushing facilitator turns and session state updates.
  - `POST /api/v1/elicitation/sessions/{id}/close`: close session and trigger pre-export validation.
- Draft session store in `tks serve` holding volatile turn text in memory or temporary draft tables with automatic session expiry. Zero audio buffers.
- Optional import of prior E-B gap questions as facilitator prompt context, distinctly separated from confirmed intent.
- Client state recovery on browser reload/reconnect, restoring existing text turns without loss.

**EA-1 acceptance criteria:**

- [ ] A user can create, inspect, pause, resume, and close an elicitation session via REST API.
- [ ] Continuing an existing session preserves `session_id`, increments `cycle`, and records current date.
- [ ] Every turn is attributed to a declared participant and allowed role.
- [ ] Imported gap questions remain facilitator context and are never emitted as stakeholder turns.
- [ ] Turn history survives browser refresh without losing draft text turns; zero audio exists on the server.

### 6.3 EA-2 — Browser Dictation Surface & Sherpa-ONNX WASM Streaming Integration

**Goal:** provide a high-responsiveness, zero-audio-transmission dictation interface in the browser served by `tks serve`, executing streaming speech recognition entirely client-side via Sherpa-ONNX WebAssembly.

Deliverables:

- Interactive web UI embedded in `tks serve` (under `src/gateway/elicitation/` or `/explorer`), including conversation timeline, live streaming preview, editable turn drafting box, and mic controls.
- Bundled `sherpa-onnx-wasm` WebAssembly engine and Sherpa Zipformer INT8 model assets (`encoder`, `decoder`, `joiner`, `tokens.txt`) served statically by `tks serve`.
- Browser `CacheStorage` / ServiceWorker caching to ensure the 67.6 MB model assets are downloaded once and cached offline.
- Dedicated `WebWorker` managing the `OnlineRecognizer` off the main UI thread.
- `AudioWorkletNode` capturing 16 kHz mono Float32 audio from `navigator.mediaDevices.getUserMedia` and piping 100 ms frames into the WebWorker.
- Push-to-talk UX:
  - Global keyboard shortcut: `Ctrl+Alt+V` or `Space` hold.
  - Click-and-hold mic button.
- Real-time partial token streaming: emitting live sub-100 ms hypotheses into the UI as speech occurs.
- Causal flush: automatic 250 ms trailing silence injection on mic release to flush Zipformer lookahead without hallucinations.
- Typed input fallback always available if microphone permission is denied or unsupported.

**EA-2 acceptance criteria:**

- [ ] Microphone access is requested only upon explicit user action.
- [ ] Sherpa-ONNX WebAssembly initializes and emits partial tokens in real time (<100 ms latency).
- [ ] Audio capture immediately terminates upon stop, discard, or page navigation.
- [ ] Network audit verifies exactly **0 audio bytes** are transmitted over the network (loopback or external).
- [ ] When microphone is denied or unavailable, typed input operates seamlessly.
- [ ] Model weights are served with appropriate cache headers and load from local browser cache on repeat visits.

### 6.4 EA-3 — Live Turn Review, Editing & Socratic Questioning Flow

**Goal:** provide seamless stakeholder review and editing of transcribed text turns before committing them to the session, and coordinate live Socratic facilitator turns with `tks serve`.

Deliverables:

- Editable turn drafting area where streaming ASR hypotheses land upon mic release, allowing the user to review, correct, edit, or retract words before submitting.
- One-key turn dispatch (`Enter` or "Submit Turn" button) sending a clean JSON payload (`POST /api/v1/elicitation/sessions/{id}/turns`) to `tks serve`.
- Integration between `tks serve` and the out-of-process Ollama conversation provider (`qwen3:8b`) to generate the next Socratic question.
- SSE event consumer in the web UI appending facilitator turns to the conversation timeline with typing/streaming indicators.
- User controls to retract a previous statement, flag an uncertain span, or split/combine turns.

**EA-3 acceptance criteria:**

- [ ] ASR hypotheses can be edited by the user before being committed as official session turns.
- [ ] Submitting a turn sends only UTF-8 JSON text to `tks serve`.
- [ ] The conversation timeline updates in real time with alternating stakeholder and facilitator turns.
- [ ] Ollama facilitator generation failure surfaces an actionable error and allows manual facilitator entry or retry.
- [ ] Socratic facilitator turns are clearly attributed to the facilitator participant and role.

### 6.5 EA-4 — Transcript Normalization, Turn Bounds & E-B Interoperability

**Goal:** format and export the completed conversation as a canonical, parser-valid `tks-transcript/v1` Markdown document that E-B can ingest directly without repair.

Deliverables:

- Canonical `tks-transcript/v1` frontmatter generator:
  - `session_id`, `cycle`, `source: asr` (or `manual`), `title`, `date`, `participants`.
- Turn normalization: contiguous `t1..tN` turn identifiers, exactly one paragraph per turn, stripping disallowed control characters.
- Turn bounds enforcement:
  - Minimum 15 turns, maximum 40 turns (per transcript specification).
  - Pre-export validation warning if turn count is below 15, offering options to continue Socratic dialogue or export as explicit draft.
- Direct Markdown export: download via browser or save directly to a configured repository path (e.g. `docs/transcripts/`).
- Mechanical validation against TKS E-B transcript parser (`ParsedTranscript::parse`).

**EA-4 acceptance criteria:**

- [ ] Exported Markdown passes `ParsedTranscript::parse` with zero warnings or errors.
- [ ] Turn IDs are contiguous and unique; all participant names match frontmatter declarations.
- [ ] Sessions below 15 turns are flagged with actionable remediation before final export.
- [ ] Exported file can be directly targeted by E-B extraction (`tks elicit extract`) without manual repair.

### 6.6 EA-5 — Socratic Prompt Hardening, Neutrality Checks & Gap Seeding

**Goal:** ensure facilitator LLM asks neutral, non-leading Socratic questions that help stakeholders discover their own intent without hallucinating solutions or presupposing architectures.

Deliverables:

- Socratic system prompt hardening for Ollama (`qwen3:8b`):
  - Focus on problem exploration, affected stakeholders, constraints, trade-offs, and boundary scoping by negation ("what the solution is NOT").
  - Explicit anti-presupposition rules: forbid asking "what tool/system do you want to build?".
- Facilitator turn review & regeneration: user can edit, rephrase, or reject a facilitator question before replying.
- Cycle-2 gap seeding: import E-B gap questions (`uncovered_intent`, `ambiguous_boundary`) as neutral facilitator prompts without asserting them as confirmed facts.
- Automated neutrality regression test suite scoring prompt neutrality against leading-question probes.

**EA-5 acceptance criteria:**

- [ ] Automated test suite verifies facilitator prompt asks open, neutral questions on problem definition.
- [ ] Facilitator does not treat speculative analogies as confirmed requirements.
- [ ] User can reject or edit a facilitator question in the UI.
- [ ] Cycle-2 sessions successfully import prior E-B gaps as facilitator prompts.

### 6.7 EA-6 — Privacy, Security, and Operational Hardening

**Goal:** verify that the implemented capture, client ASR, and conversation-provider paths strictly satisfy D-130 … D-134 and C-28.

Deliverables:

- Client-side audio lifecycle audit: verify that Float32 PCM buffers in `AudioWorklet` and WebWorker are garbage-collected and dropped immediately after frame decoding.
- Content Security Policy (CSP) headers on `tks serve`: restrict web traffic to loopback/self; prohibit any outbound network connections to third-party endpoints.
- Zero-audio network audit: automated browser test verifying 0 audio bytes and 0 network requests containing binary audio payloads.
- WebAssembly memory bounds & leak detection: ensure long-running elicitation sessions do not leak WASM linear memory.
- Server route authentication and CSRF protection for elicitation endpoints.

**EA-6 acceptance criteria:**

- [ ] Automated network trace verifies 0 audio bytes sent across loopback or external network.
- [ ] Code inspection and browser profiling confirm no audio is stored in `localStorage`, `IndexedDB`, or cookies.
- [ ] CSP headers block third-party analytics, cloud speech APIs, and unauthorized scripts.
- [ ] WebWorker memory consumption remains bounded (<200 MB) across 40 consecutive turns.

### 6.8 EA-7 — Evaluation, Documentation, and Dogfooding Release Gate

**Goal:** validate the complete end-to-end Intent Elicitation journey from microphone dictation to E-B goal extraction, document usage, and pass the final release gate.

Deliverables:

- Full end-to-end dogfooding elicitation session: stakeholder articulates intent via push-to-talk, Socratic facilitator guides dialogue, transcript is exported as valid `tks-transcript/v1`, and E-B extracts grounded `GOAL` nodes.
- User documentation in `docs/`:
  - Quickstart guide for launching `tks serve` and accessing the elicitation interface.
  - Push-to-talk keyboard shortcuts (`Ctrl+Alt+V`) and dictation tips.
  - Zero-audio privacy guarantee explanation.
  - E-B handoff instructions.
- Third-party licensing documentation: recording `k2-fsa/sherpa-onnx` (Apache-2.0) and Zipformer model weights in repository notices.
- User sign-off on release gate.

**EA-7 acceptance criteria:**

- [ ] Automated tests cover session lifecycle, client WASM worker, turn submission, and transcript export.
- [ ] A human-conducted dogfood session exports a valid transcript that passes E-B extraction with 100% mechanical quote grounding.
- [ ] User documentation is complete and verified.
- [ ] Final user approval received before merging to main release track.

## 7. Risks and Mitigations

| Risk | Mitigation |
| ---- | ---------- |
| Browser does not support WebAssembly SIMD. | Detect WASM SIMD capability on startup; fall back gracefully to typed input with an informative notice. |
| Model asset download delay on cold cache. | Model is compact (67.6 MB INT8); pre-cache in browser `CacheStorage` on first visit; show a clear initial loading progress indicator. |
| Stakeholder speaks over background noise. | Sherpa-ONNX Zipformer proved resilient (9.6% WER at 10 dB SNR); push-to-talk UX ensures mic is active only while speaking. |
| Model hallucinates on silence or hum. | Transducer architecture naturally emits blanks on non-speech; Spike EA-0.2 proved 0/4 hallucinations on silence, white noise, and mains hum without external VAD. |
| Socratic facilitator asks leading questions. | Hardened system prompt with explicit anti-presupposition instructions; UI provides controls to reject or edit facilitator questions (EA-5). |
| Sessions have fewer than 15 turns. | Pre-export validation warns user of minimum turn bounds and guides continued conversation before export (EA-4). |
| Microphone permission denied by browser. | UI provides an immediate, full-featured typed input fallback; no functionality is blocked. |

## 8. Dependency and Exit Gates

| Work package | Depends on | Exit gate |
| ------------ | ---------- | --------- |
| **EA-0 / EA-0.2** | Spike plan | **Completed & Approved:** Sherpa-ONNX Zipformer confirmed; zero audio over wire proven. |
| **EA-1** | EA-0.2 | Session lifecycle, turn endpoints, and draft model working in `tks serve`. |
| **EA-2** | EA-1 | Browser UI with client-side Sherpa-ONNX WASM streaming dictation working (<100 ms latency). |
| **EA-3** | EA-2 | Real-time draft turn editing, turn submission, and Socratic Ollama SSE turn stream functioning. |
| **EA-4** | EA-1, EA-3 | Exported Markdown satisfies `tks-transcript/v1` rules and passes `ParsedTranscript::parse`. |
| **EA-5** | EA-1, EA-3 | Socratic facilitator prompts hardened and validated for neutrality; gap questions imported. |
| **EA-6** | EA-2, EA-3 | Privacy and security audit confirms zero audio leakage, strict CSP, and bounded WASM memory. |
| **EA-7** | EA-1 through EA-6 | Successful dogfood session, documentation verified, and user release approval granted. |

## 9. Definition of Done

E-A is complete when a stakeholder can articulate intent through an interactive push-to-talk browser dictation interface powered by in-browser Sherpa-ONNX WebAssembly, engage in neutral Socratic dialogue, review and correct transcript turns in real time, export a canonical `tks-transcript/v1` document, and feed it directly into E-B for goal extraction. Microphone audio never leaves the client's browser, `tks serve` remains free of heavy ML dependencies, and the facilitator never invents or leads stakeholder requirements.
