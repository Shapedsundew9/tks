# Spike EA-0.2 Proposal: Client-Side Streaming Speech Recognition & Web Integration

> **Governing plan:** [intent-elicitation-ea-plan.md §6.1](intent-elicitation-ea-plan.md).  
> **Decisions in scope:** D-130, D-131 (reopened & revised), D-132, and C-28 in [architecture.md](../vision/architecture.md).  
> **Status:** Approved by user on 2026-10-24.

---

## 1. Executive Summary & Reframing

### 1.1 The Pivot

Spike EA-0 tested an **offline, server-side batch transcription** approach using a 2-billion parameter model (Cohere Transcribe) and Whisper Large on `candle`. While rigorous and informative, it proved that server-side batch inference:

1. Imposes extreme resource overhead (9.4–14.4 GiB RAM on CPU, or 4.4 GiB VRAM on GPU).
2. Introduces heavy toolchain dependencies (CUDA 12.8, massive checkpoint downloads, C++ toolchains).
3. Breaks GPU co-tenancy with local facilitator LLMs (OOM with `qwen3:8b`).
4. Creates high turn latency (4.9s–8.8s p50 End-of-Utterance wait on CPU).
5. Requires streaming raw microphone audio over WebSockets to the server daemon.

The intended user experience is **interactive, low-latency dictation** analogous to VS Code Speech (`Ctrl+Alt+V`) or the Antigravity CLI voice interface, accessed through a browser UI served by `tks serve`.

### 1.2 The New Architectural Premise

1. **Client-Side In-Browser ASR (WASM / WebGPU):** Audio capture and speech recognition run entirely inside the client's browser (Web Worker via WebAssembly SIMD or WebGPU).
2. **Zero Audio Over the Wire:** Microphone audio never leaves the browser process. No audio bytes or audio WebSockets ever touch `tks serve`.
3. **Text-Only Transport ("Squirted" back to Server):** The client emits real-time partial text hypotheses to the UI, allows user review/correction, and sends only UTF-8 transcript turns (JSON) over HTTP/SSE to `tks serve`.
4. **Clean Server Daemon:** `tks serve` remains a lightweight, 100% Rust daemon free of heavy ML frameworks, audio codecs, and CUDA toolchains.

```text
┌─────────────────────────────────────────────────────────────┐
│                      Client Browser                         │
│                                                             │
│  [Microphone] ──▶ [AudioWorklet] ──▶ [WASM / WebGPU Engine] │
│                                             │               │
│                                     real-time tokens        │
│                                     (<100ms latency)        │
│                                             ▼               │
│                                   [Live Transcript UI]      │
│                                             │               │
│                                   user reviews / edits      │
│                                             ▼               │
│                                   UTF-8 JSON Turn           │
└─────────────────────────────────────────────┼───────────────┘
                                              │ POST /api/elicit/turn
                                              ▼
┌─────────────────────────────────────────────────────────────┐
│                        tks serve                            │
│                                                             │
│       [Elicitation Session Store] ◀── text only             │
│                     │                                       │
│                     ▼                                       │
│         [Facilitator Provider]                              │
│        (External Agent or Ollama)                           │
│                     │                                       │
│              Socratic prompt                                │
│                     ▼                                       │
│          SSE / Facilitator Turn ──▶ Client Browser          │
└─────────────────────────────────────────────────────────────┘
```

---

## 2. Updated Architectural Decisions

### 2.1 Reopen and Revise D-131 (Architecture Decision)

* **Previous Decision:** In-process server-side English ASR using Cohere Transcribe (2B) on `candle`.
* **Revision Status:** Reopened under D-131's explicit condition (*"The selected model fails the EA-0 runtime or hardware gate..."*).
* **New Decision:** Speech recognition is executed **client-side within the browser interface** using a lightweight, permissively licensed (Apache-2.0 or MIT) streaming ASR engine running via WebAssembly (WASM SIMD) or WebGPU. `tks serve` serves static engine/model assets but performs zero audio processing.
* **Primary Candidates:**
  * **Sherpa-ONNX WebAssembly** (streaming Zipformer / Conformer-RNN-T, Apache-2.0, ~20–60 MB weights).
  * **Transformers.js / ONNX Runtime Web** (quantized Whisper `tiny.en` / `base.en`, MIT, ~39–73 MB weights).

### 2.2 Strengthen C-28 (Constraint)

* **Revised Text:** **Client-side only speech recognition and absolute network audio isolation:** Speech recognition occurs exclusively on the client device inside the web browser runtime (WASM/WebGPU). Raw microphone audio is never transmitted across any network interface (including loopback to `tks serve`), never leaves client memory, and is dropped immediately upon frame consumption. `tks serve` provides no audio ingestion endpoints and remains strictly free of audio processing libraries and GPU/CUDA compute dependencies.

### 2.3 Strengthen D-132 (Zero Audio Storage)

* Reaffirmed and simplified: Since `tks serve` never receives audio bytes, server-side audio leakage, swap dumping, ring buffer zeroing, or audio socket eavesdropping risks are completely eliminated at the architectural boundary.

### 2.4 Revise D-130 (Conversation Provider Boundary)

* `tks serve` exposes an elicitation session API (`/api/v1/elicitation/...`) that accepts finalized text turns from the client, coordinates Socratic questioning via external providers (Ollama / MCP sampling), and streams facilitator turns back to the client via SSE.

---

## 3. Spike EA-0.2 Execution Plan

### 3.1 Objectives

1. Evaluate candidate client-side streaming speech engines running in the browser (WASM/WebGPU).
2. Measure performance and accuracy using the exact same 60 test audio clips used in EA-0 (clean LibriSpeech, disfluent AMI, 10 dB noise, non-speech).
3. Validate true streaming responsiveness (perceived token latency < 100 ms).
4. Measure resource impact (asset download size, browser memory, zero server overhead).
5. Verify end-to-end data flow: audio captured in browser → transcribed in WASM → text turn submitted to `tks serve` via REST/SSE.

### 3.2 Candidate Engines & Models Under Test

| Candidate | Runtime Engine | Model Architecture | Model Weights & Download | Licence |
| :--- | :--- | :--- | :--- | :--- |
| **Candidate A: Sherpa-ONNX Web** | `sherpa-onnx-wasm` (ONNX Runtime WASM) | Streaming **Zipformer / RNN-T** | ~20 MB – 60 MB (INT8 quantized) | Apache-2.0 |
| **Candidate B: Transformers.js / ORT Web** | `@huggingface/transformers.js` (WASM / WebGPU) | Quantized **Whisper `tiny.en` / `base.en`** | ~39 MB (`tiny.en`), ~73 MB (`base.en`) | MIT |
| **Candidate C: whisper.cpp WASM** | `whisper.wasm` (Emscripten / GGML) | Quantized **Whisper `tiny.en` / `base.en`** | ~75 MB (`tiny.en`), ~140 MB (`base.en`) | MIT |

### 3.3 Test Matrix & Methodology

The spike will run an automated test runner executing the exact same test clips from EA-0 (`/workspaces/.tks-cache/ea0-testset`):

1. **15 clean clips:** `hf-internal-testing/librispeech_asr_dummy`
2. **15 disfluent clips:** `edinburghcstr/ami` IHM test split
3. **15 noisy-clean clips:** 10 dB SNR white noise
4. **15 noisy-disfluent clips:** 10 dB SNR white noise
5. **4 non-speech clips:** silence, room noise, mains hum, loud noise

Transcriptions will be scored against reference texts using the exact normalizer from EA-0 (`scripts/spikes/ea0_asr/wer.rs`).

### 3.4 Key Evaluation Criteria & Metrics

1. **Recognition Quality:**
   * Word Error Rate (WER) across the 4 speech sets.
   * Silence / hallucination resistance on the 4 non-speech clips (does it emit phantom words on silence or hum?).
2. **Responsiveness & Latency:**
   * **Time to First Partial Token:** Latency from voice onset to visible text on screen (target: < 150 ms).
   * **End-of-Utterance Latency:** Delay from end-of-speech to finalized turn (target: < 100 ms).
   * **Real-Time Factor (RTF):** Processing time / audio duration in browser WASM.
3. **Asset & Resource Footprint:**
   * Initial asset download size (WASM binary + model weights transferred to browser cache).
   * Browser client memory (WASM linear memory + JS heap).
   * Server host CPU, RAM, and VRAM utilization during transcription (expected: 0%).
4. **Hardware & Browser Compatibility:**
   * Operation on plain CPU via WebAssembly SIMD.
   * Acceleration when WebGPU is available.
   * Zero CUDA, Python, or PyTorch host dependencies.
5. **Network & Privacy Isolation:**
   * Network trace verification: confirm 0 audio bytes sent over the network.
   * End-to-end delivery of finalized turn text to `tks serve` mock endpoint.

---

## 4. Spike EA-0.2 Deliverables

1. **Headless Benchmark Harness:** `scripts/spikes/ea0_streaming/run_browser_benchmark.js` running the EA-0 testset through the candidate WASM engines.
2. **Minimal Interactive Demo Page:** Served via `tks serve` (under `/spike/dictate`), demonstrating push-to-talk live dictation, live partial hypotheses, and turn submission.
3. **Results Document (`docs/vision/spike-ea0-2-results.md`):**
   * Comparative scorecard between EA-0 (Cohere Transcribe / Whisper Large on Candle) and EA-0.2 (Sherpa-ONNX / Transformers.js).
   * Recommendation for the official engine to embed into TKS Intent Elicitation (EA-2).
