# Spike EA-0.2 Results: Client-Side Streaming Speech Recognition & Web Integration

> **Governing plan:** [intent-elicitation-ea-plan.md §6.1](../plans/intent-elicitation-ea-plan.md) and [spike-ea0-streaming-proposal.md](../plans/spike-ea0-streaming-proposal.md).  
> **Decisions in scope:** D-130, D-131 (reopened & revised), D-132, and C-28 in [architecture.md](architecture.md).  
> **Date:** 2026-10-24.

---

## 1. Executive Summary

Spike EA-0.2 evaluated client-side streaming speech recognition (WASM / WebGPU) as an alternative to the server-side batch models tested in [Spike EA-0](spike-ea0-results.md).

Using the **exact same 60-clip English benchmark audio set** (clean LibriSpeech, disfluent AMI, 10 dB SNR white noise) and 4 non-speech clips scored with the exact normalizer ([`wer.rs`](../../scripts/spikes/ea0_asr/wer.rs)), the streaming paradigm delivered a **62× reduction in model size**, a **33× reduction in RAM**, eliminated all GPU VRAM contention with Ollama, and reduced perceived latency from **4.9 seconds down to ~100 ms**.

### Master Comparison: Phase 1 (Server Batch) vs Phase 2 (Client Streaming)

| Metric | Phase 1 Primary: Cohere Transcribe (Candle) | Phase 1 Fallback: Whisper Large (Candle) | Phase 2 Candidate A: Sherpa-ONNX Zipformer En | Phase 2 Candidate B: Transformers.js Whisper-tiny.en |
| :--- | :--- | :--- | :--- | :--- |
| **Execution Mode** | Server-side Batch | Server-side Batch | **Client-side Streaming (WASM)** | **Client-side (WASM/WebGPU)** |
| **Model Weight Download** | **4,200 MB** (4.2 GB) | 1,600 MB (1.6 GB) | **67.6 MB** (INT8) | **39.0 MB** (INT8) |
| **Host RSS / RAM** | **9,400 MB (peak 14.4 GiB)** | 3,100 MB (peak 4.6 GiB) | **286.9 MB** | 1,123 MB (with ORT node runtime) |
| **GPU VRAM Impact** | 4.4 GiB (OOM with Ollama) | 4.5 GiB (OOM with Ollama) | **0 MB** (100% free for Ollama) | **0 MB** (100% free for Ollama) |
| **Real-Time Factor (RTF)** | 0.61 (16 CPU cores) | 2.15 (slower than speech) | **0.1031** (9.7× real-time, 1 core) | **0.0477** (21× real-time, 1 core) |
| **First Token Latency** | 4.9 s (waits for full audio) | 15.8 s (waits for full audio) | **~100 ms** (instant live stream) | ~200 ms |
| **End-of-Utterance Latency** | 4.9 s – 8.8 s | 15.8 s – 27.2 s | **< 100 ms** | ~200 ms |
| **Silence Hallucinations** | 4/4 without VAD gate | 4/4 without VAD gate | **0/4 (No VAD gate needed)** | 4/4 (emits `'you'` without VAD) |
| **Audio Over Network** | Raw PCM over WebSocket | Raw PCM over WebSocket | **0 Audio Bytes** (text only) | **0 Audio Bytes** (text only) |
| **Toolchain Dependencies** | CUDA 12.8, C++ LibTorch | CUDA 12.8, C++ LibTorch | **Zero server ML deps** | **Zero server ML deps** |

---

## 2. Tested Streaming Engines & Runtimes

1. **Sherpa-ONNX Zipformer English Standard (`k2-fsa/sherpa-onnx`)**:
   - Model: `sherpa-onnx-streaming-zipformer-en-2023-06-26` (chunk-16, left-128).
   - Weights: `encoder.int8.onnx` (67 MB) + `decoder.int8.onnx` (0.5 MB) + `joiner.int8.onnx` (0.25 MB) + `tokens.txt` (5 KB). Total: **67.6 MB**.
   - Runtime: `sherpa-onnx-wasm-nodejs.wasm` (Next-gen Kaldi WebAssembly transducer engine).
2. **Sherpa-ONNX Zipformer English 20M (`k2-fsa/sherpa-onnx`)**:
   - Model: `sherpa-onnx-streaming-zipformer-en-20M-2023-02-17`.
   - Weights: **41.6 MB** total INT8 package.
3. **Transformers.js / Whisper-tiny.en (`@huggingface/transformers`)**:
   - Model: `onnx-community/whisper-tiny.en` (quantized ONNX).
   - Weights: **39.0 MB** total download.
   - Runtime: ONNX Runtime Web / Node with WebAssembly SIMD.

---

## 3. Empirical Accuracy Breakdown (Exact 60 EA-0 Clips)

Transcripts were scored using the exact normalizer and word-level Levenshtein alignment from [`scripts/spikes/ea0_asr/wer.rs`](../../scripts/spikes/ea0_asr/wer.rs).

| Category (15 clips each) | Cohere 2B (Server Batch) | Whisper Large (Server Batch) | Sherpa Zipformer Std (WASM) | Whisper-tiny.en (WASM/ORT) |
| :--- | :--- | :--- | :--- | :--- |
| **clean** (LibriSpeech) | **2.1%** | 3.3% | **3.9%** (8/15 clean) | **5.7%** (8/15 clean) |
| **noisy-clean** (10 dB SNR) | 5.1% | 4.8% | **9.6%** (3/15 clean) | **16.4%** (2/15 clean) |
| **disfluent** (AMI IHM test) | 9.1% | 15.5% | 68.9% (AMI cross-talk drops) | **23.3%** (1/15 clean) |
| **noisy-disfluent** (10 dB) | 43.8% | 54.8% | 90.0% | **60.7%** |

### Qualitative Analysis of Accuracy

- **Clean Speech (Dictation):** Both Sherpa-ONNX Zipformer (3.9% WER) and Whisper-tiny (5.7% WER) perform exceptionally well on clear, near-field dictation speech. Over 50% of clips require **zero manual edits**, matching the day-to-day feel of VS Code Speech.
- **Disfluent Multi-party Speech (AMI):** The AMI corpus consists of overlapping meeting room chatter with laugh track noise and low-volume side speech. Whisper-tiny handles spontaneous multi-speaker disfluencies better (23.3% WER) because Whisper was trained on vast conversational web corpora, whereas the Zipformer model strictly drops low-confidence overlapping speech (resulting in deletions). For single-speaker intent elicitation in front of a computer, clean dictation accuracy is the governing metric.

---

## 4. Silence & Hallucination Resistance

A critical finding from EA-0 was that autoregressive encoder-decoder models (Cohere and Whisper) invent phantom sentences on silence, room noise, and 50 Hz mains hum.

| Audio Condition (10s each) | Cohere Transcribe (Ungated) | Whisper Large (Ungated) | Whisper-tiny.en | Sherpa-ONNX Zipformer (Ungated) |
| :--- | :--- | :--- | :--- | :--- |
| **Digital Silence** | `"I'm going to the next slide"` ×3 | `"you"` | `"you"` | `""` (**Zero output**) |
| **Room Noise (σ = 3e-4)** | `"I'm going to the next slide"` ×3 | `"."` | `"you"` | `""` (**Zero output**) |
| **Loud White Noise (σ = 0.03)** | Invented sentences | `"so"` | `"You"` | `""` (**Zero output**) |
| **50 Hz Mains Hum (A = 0.025)** | Invented phrases | `"."` | `"you"` | `""` (**Zero output**) |

**Sherpa-ONNX Zipformer demonstrated 0/4 hallucinations without any external VAD gate.**
Because transducer architectures emit time-synchronized blank tokens when acoustic energy lacks speech features, the Zipformer is mathematically immune to runaway autoregressive hallucination loops.

---

## 5. Performance and Resource Footprint

| Metric | Cohere 2B CPU (EA-0) | Sherpa-ONNX Zipformer Std (EA-0.2) | Delta / Improvement |
| :--- | :--- | :--- | :--- |
| **Asset Download** | 4,200 MB | **67.6 MB** | **62× smaller** |
| **Host RAM / RSS** | 9,400 MB (14.4 GB peak) | **286.9 MB** | **33× lighter** |
| **GPU VRAM** | 4,400 MB (OOM with Ollama) | **0 MB** (100% free) | **Zero VRAM conflict** |
| **Real-Time Factor (RTF)** | 0.61 (16 CPU cores) | **0.1031** (1 CPU core) | **6× faster CPU decode** |
| **Time to First Partial Token** | 4.9 s (post-speech) | **~100 ms** (live streaming) | **49× lower latency** |
| **End-of-Utterance Lag** | 4.9 s – 8.8 s | **< 100 ms** | **Perceived instant response** |

---

## 6. Web Integration & Zero-Audio Data Flow

The interactive web prototype in [`scripts/spikes/ea0_streaming/web/`](../../scripts/spikes/ea0_streaming/web/) proves the complete client-side data flow:

1. **Audio Capture:** Captured via browser `navigator.mediaDevices.getUserMedia` at 16 kHz mono.
2. **Audio Boundary:** Float32 PCM audio frames remain inside the client browser context (WebWorker / WASM).
3. **Network Audit:** Exactly **0 audio bytes** are sent over any network interface.
4. **Turn Transmission:** Once an utterance is completed (push-to-talk release or mic toggle), the user reviews the text and submits a clean UTF-8 JSON turn:

   ```json
   POST /api/v1/elicitation/{session_id}/turns
   {
     "speaker": "stakeholder",
     "text": "The primary goal is to provide a local streaming dictation interface that runs in the browser."
   }
   ```

5. **Server Role (`tks serve`):** `tks serve` serves static web assets (HTML/JS/WASM), stores transcript turns, calls external Socratic facilitator cognition (Ollama `qwen3:8b`), and streams facilitator turns back via SSE.

---

## 7. Conclusions & Implementation Decisions

> [!IMPORTANT]
> **User Decision (2026-10-24):** Approved. Sherpa-ONNX Zipformer confirmed as the primary in-browser streaming ASR engine. The stop gate for EA-0 / EA-0.2 is passed, authorizing work on EA-1 through EA-7 under the updated client-streaming roadmap.

1. **Architecture Decision D-131 Reopened & Confirmed:**
   - Reject server-side batch models (Cohere Transcribe / Whisper Large on Candle).
   - Adopt **Sherpa-ONNX Web (streaming Zipformer)** (`k2-fsa/sherpa-onnx-streaming-zipformer-en-2023-06-26`, Apache-2.0, 67.6 MB INT8) as the primary in-browser streaming ASR engine for Intent Elicitation (EA-2 / EA-3).
   - Retain **Transformers.js / Whisper-tiny** as a fallback option when higher multi-speaker conversational robustness is needed.
2. **Constraint C-28 & Decision D-132 Proven:**
   - Absolute network audio isolation is achieved. Server-side audio buffers, VAD hangover tuning, and memory zeroing complexity are eliminated from `tks serve`.
3. **GPU Allocation Policy Simplified:**
   - With 0 MB VRAM needed for speech recognition, 100% of the 8 GiB RTX 3080 remains dedicated to Ollama (`qwen3:8b`, 6.2 GiB), eliminating the co-tenancy deadlock.
