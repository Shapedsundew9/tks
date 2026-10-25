# User Guide: Intent Elicitation Studio (Track E-A)

The Intent Elicitation Studio (`/explorer/elicitation`) provides an in-browser dictation and conversational elicitation environment designed to capture high-fidelity stakeholder intent before formal requirements decomposition.

It integrates local streaming speech recognition (ASR via Sherpa-ONNX WASM) and optional Socratic facilitator assistance (via local Ollama inference) while enforcing a strict **zero-audio persistence and zero-audio transmission guarantee** (D-131, D-132, C-28).

---

## 1. Prerequisites and Setup

### Browser Requirements

- Any modern **Chromium-based browser** (Google Chrome, Chromium, Brave, Microsoft Edge) running in a secure origin context (`http://localhost` or `http://127.0.0.1`).
- WebAssembly SIMD support (enabled by default in all modern Chromium releases).
- Microphone permission granted on explicit user interaction.

### Fetching ASR Model Assets

Before enabling speech dictation, download and verify the pinned Sherpa-ONNX Zipformer model assets using the idempotent repository fetcher:

```bash
./scripts/elicitation/fetch_asr_assets.sh
```

By default, assets are verified against `scripts/elicitation/asr-assets.lock` and installed into:

```text
~/.cache/tks/models/sherpa-onnx-web/v1.13.7/
```

### Starting the Server Daemon

Launch `tks serve` specifying the verified asset directory and desired facilitator mode:

```bash
# Production setup with local ASR and Ollama facilitator
tks serve \
  --asr-assets-dir "${HOME}/.cache/tks/models/sherpa-onnx-web/v1.13.7" \
  --facilitator ollama \
  --facilitator-model qwen3:8b

# Or typed-only mode without ASR or Ollama
tks serve
```

Once started, navigate to:

```text
http://localhost:8080/explorer/elicitation
```

---

## 2. Using the Studio

### Capability Detection & Progressive Degradation

When the studio opens, it queries `GET /api/v1/elicitation/capabilities` and checks your browser's WebAssembly capabilities:

- **ASR Ready:** When the local assets are present and WASM SIMD is detected, the WebWorker downloads the model into the browser's disk cache. The microphone button enables automatically when ready.
- **Typed-Only Fallback:** If microphone permission is denied, ASR assets are omitted, or SIMD is unsupported, the studio displays an informative banner and remains fully operational in typed mode.

### Push-to-Talk Spoken Dictation

Dictation operates strictly via **push-to-talk**:

1. **Press and hold** the `Ctrl + Alt + V` keyboard shortcut or click-and-hold the microphone button.
2. Speak your intent clearly. While talking, the **Live Partial Preview** bar displays real-time streaming hypotheses.
3. **Release** the shortcut or button to stop. The audio pipeline applies a 250 ms trailing silence flush and appends the final transcribed text into the editable draft box.
4. Dictated text is flagged with `Mode: Spoken (Dictated)`.

> [!NOTE]
> `Space` is explicitly **not** bound to recording so you can type freely in the draft box.

### Reviewing and Submitting Turns

- **Draft Editing:** All dictated text arrives in the editable composer box. You can correct words, add punctuation, or append typed text before committing.
- **Keyboard Shortcuts:**
  - `Enter`: Commits and submits the turn.
  - `Shift + Enter`: Inserts a space (newlines are prohibited in turns to maintain canonical single-paragraph utterances).
  - `Esc`: Clears the current draft.
- **Role Switching:** The composer allows toggling between `Stakeholder` and `Facilitator` roles.

### Post-Commit Turn Review & Actions

Committed turns appear immediately in the chronological timeline:

- **Correct Transcription:** Click **Correct** on any turn to edit the transcription. This issues an in-place `PATCH` update.
- **Flag Uncertain:** Click **Flag Uncertain** to tag terms or sentences requiring follow-up clarification.
- **Retractions & Mind Changes:** To retract or alter a previous statement, simply articulate the change in a subsequent turn. Elicitation preserves conversational history; downstream goal extraction (Track E-B) reconciles goal states and supersessions.

---

## 3. Socratic Facilitator & Neutrality Linting

When `--facilitator ollama` is active, the studio assists the elicitation process by proposing neutral, open-ended questions:

- **Proposal Card:** Proposed questions appear above the composer with action buttons:
  - **Accept:** Commits the proposed question as a facilitator turn.
  - **Edit then Accept:** Copies the proposal into the composer for manual refinement before committing.
  - **Regenerate:** Requests an alternative inquiry from the facilitator.
  - **Reject:** Dismisses the proposal.
- **Neutrality Lint Badges:** All proposals are mechanically analyzed by a deterministic neutrality linter (`src/elicitation/neutrality.rs`):
  - `Presupposition`: Warns if the question presupposes a solution (e.g. *"the system you want to build"*).
  - `Technology`: Warns if the question introduces implementation mechanisms (e.g. databases, APIs, Kubernetes).
  - `Multiple Questions`: Warns if more than one question mark is present.
  - `Closed Question`: Warns if the inquiry begins with a closed yes/no opener (*"Is...", "Can..."*).
- **Opening Question:** Empty sessions include an **Ask Opening Question** button to kick off intent exploration.

---

## 4. Multi-Cycle Continuation & E-B Gap Seeding

To conduct multi-cycle interviews:

1. In the **New Session** form, expand **Cycle-2 Continuation Options**.
2. **Prior Transcript:** Paste the Markdown transcript from Cycle 1. The server validates frontmatter, increments `cycle = 2`, and preserves the canonical `session_id`.
3. **Prior E-B Extraction Session ID:** Provide the UUID from an E-B goal extraction session. The server imports detected open gaps (e.g., missing anti-goals, ungrounded soft goals) into the **Questions from Previous Cycle** panel.
4. Clicking any gap question loads it into the composer for the facilitator to review and pose.

---

## 5. Closing & Canonical Export

When the conversation concludes:

1. Click **Close Session**.
2. The server executes validation checks (§6.5):
   - At least one turn is recorded.
   - All uncertainty flags have been cleared.
   - The transcript conforms to canonical syntax.
3. Click **Export Canonical Transcript** to download the canonical Markdown transcript (`<session_id>-c<cycle>.md`).

### E-B MCP Handoff

After exporting the canonical transcript:

1. Save the file to your repository (e.g. `docs/transcripts/<session_id>-c<cycle>.md`).
2. Ask your connected AI agent to run the Track E-B goal extraction workflow:

   ```text
   Please call open_extraction_session with doc_path="docs/transcripts/<session_id>-c<cycle>.md",
   segment episodes, extract grounded goal candidates, and present the gap report.
   ```

---

## 6. Privacy & Security Guarantees

### Zero Audio Transmission & Persistence Guarantee

- **No Server Audio:** The Rust server daemon (`tks serve`) contains no audio codecs, ML models, or audio ingestion endpoints. Zero audio bytes ever reach `tks serve` or any remote server.
- **No Client Persistence:** The front-end creates zero `MediaRecorder`, `Blob`, `IndexedDB`, or `CacheStorage` audio buffers. Raw audio exists solely as transient Float32 frames in browser RAM during an active push-to-talk press.
- **Track Lifecycle:** Audio tracks and worklet nodes are terminated and disconnected immediately upon release, tab switch, blur, or window discard.

### Loopback Isolation & Residual Risks

- **Loopback Enforcement:** Elicitation API and UI endpoints are guarded by middleware rejecting non-loopback `Host` headers (`localhost`, `127.0.0.1`, `[::1]`) and refusing cross-origin mutations. Elicitation routes carry no permissive CORS headers.
- **Microphone Permissions:** Modern browsers permit microphone access only in secure origins (`localhost` or HTTPS).
- **Residual Risk:** Because `tks serve` binds `0.0.0.0` to support multi-container setups, a malicious process on the local network could theoretically spoof `Host: localhost` if an `elicitation_id` UUID is known. Production deployments should restrict binding or use devcontainer network isolation.
- **In-Memory Volatility:** Active drafts reside in server RAM. Restarting `tks serve` discards active unexported sessions. Sessions idle past `--elicitation-idle-expiry` (default `24h`) are automatically swept.
