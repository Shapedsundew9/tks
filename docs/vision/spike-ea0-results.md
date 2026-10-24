# Spike EA-0 Results: Conversation Provider, Local ASR Runtime, and Audio Data Flow

> [!NOTE]
> **Phase 1 Baseline Archive & Architectural Pivot:**
> This document records the empirical results of the **Phase 1 Server-Side Batch ASR Baseline** (Cohere Transcribe 2B & Whisper Large on Candle). While these results validated offline execution and WER accuracy, the investigation proved that server-side batch inference introduces prohibitive memory overhead (9.4–14.4 GiB RAM), requires heavy CUDA toolchains, breaks GPU co-tenancy with local facilitator LLMs, and causes high turn latency (4.9s–8.8s p50).
>
> On 2026-10-24, user approval reopened D-131 to pivot to **Client-Side In-Browser Streaming ASR (WASM/WebGPU)** to match interactive dictation experiences like VS Code Speech and Antigravity voice. The new methodology and benchmark plan are defined in [spike-ea0-streaming-proposal.md](../plans/spike-ea0-streaming-proposal.md).

Governing plan: [intent-elicitation-ea-plan.md §6.1](../plans/intent-elicitation-ea-plan.md).
Decisions in scope: D-130 … D-134 and C-28 in [architecture.md](architecture.md). Date: 2026-10-23.

## 1. Executive Summary

| Question | Outcome |
| --- | --- |
| Conversation provider (D-130) | **Ship out-of-process Ollama offload + typed/manual.** Both proven end to end with parser-valid `tks-transcript/v1` output. **MCP sampling is not viable yet**: TKS's MCP transport cannot send server→client requests, and of the target clients only VS Code advertises sampling. |
| ASR model (D-131) | **Cohere Transcribe confirmed** over Whisper large-v3-turbo: lower WER in every category (clean 2.1% vs 3.3%, disfluent 9.1% vs 15.5%) and 3.5× faster on CPU. |
| Rust runtime | **candle 0.9** via a **vendored, patched** community port (`cohere-transcribe-candle`). The upstream port had a positional-attention bug (clean WER 46%); after the fix it matches the reference `transformers` implementation on 24/30 clips (2.0% word disagreement). |
| Hardware fit | CPU works (Cohere RTF 0.61, end-of-utterance p50 4.9 s, RSS 9.4 GiB). GPU is faster (RTF 0.19, p50 2.0 s) but needs ~4.2 GiB VRAM and **cannot co-reside with Ollama's `qwen3:8b`** (6.2 GiB) on the 8 GiB RTX 3080. |
| Voice-activity gate | **Mandatory.** Ungated, Cohere hallucinates repeated sentences on silence and mains hum. earshot VAD (threshold 0.4, ≥500 ms sustained speech) rejected 4/4 non-speech clips and 0/45 clean, disfluent or noisy-clean clips, but also 4/15 noisy-disfluent clips. |
| Offline / no network (C-28) | **Proven.** CPU runs in an empty network namespace under `strace -e trace=%network` recorded **0** network syscalls; GPU runs used `docker --network none` (loopback only, 0 bytes). |
| No audio storage (D-132) | Data-flow table in §7: every buffer is bounded, volatile, and dropped; none is persisted or logged. Hardening items are listed for EA-6. |
| Optional feature | `--features asr-spike` only; default `cargo check/build/test` unaffected. |

**Stop gate:** EA-1+ has not been started. §9 lists the decisions requested from the user.

## 2. Tested Environments

| Environment | Detail |
| --- | --- |
| CPU | Devcontainer `app` service, 16 vCPU, 30 GiB RAM, no GPU, Rust 1.99 stable, release build. |
| GPU | Sibling container `nvidia/cuda:12.8.1-devel-ubuntu24.04` on the same Docker host via CDI (`--device nvidia.com/gpu=all`), RTX 3080 Laptop 8 GiB, driver 580.178, `CUDA_COMPUTE_CAP=86`, `--network none` for runs. No devcontainer rebuild was needed. |
| Ollama | Devcontainer `ollama` service, `qwen3:8b` Q4_K_M (resident with keep-alive −1, 6.2 GiB VRAM). |
| Models | `CohereLabs/cohere-transcribe-03-2026@d552dadc2922ba6f6c1f994f8db904d52dcaf942` (Apache-2.0, gated "auto" contact-info agreement; accepted by the user); `openai/whisper-large-v3-turbo@41f01f3fe87f28c78e2fbf8b568835947dd65ed9` (MIT, ungated). |
| Reference | PyTorch 2.14.1 CPU + transformers 5.20.2 `CohereAsrForConditionalGeneration` (throwaway venv outside the repo) for port fidelity only. |

## 3. Conversation Provider Comparison (D-130)

| Dimension | MCP sampling | Out-of-process Ollama | Typed / manual |
| --- | --- | --- | --- |
| Status | **Not viable today** (transport gap + limited clients) | **Proven** | **Proven** (always available) |
| Client support | VS Code: supported. Claude Code, Claude.ai, GitHub Copilot CLI, Gemini CLI, Cursor, Codex: **not advertised** ([apify/mcp-client-capabilities](https://github.com/apify/mcp-client-capabilities) `mcp-clients.json` @ `1e7dfab`, 2026-09-02). | Any; TKS (or a helper) calls a configured HTTP endpoint. | n/a |
| TKS changes needed | Server→client JSON-RPC requests: `handle_mcp_sse` in `src/gateway/mcp/mod.rs` only emits `endpoint` + keep-alives, `initialize` advertises only `tools`/`prompts` (protocol `2024-11-05`), and the stdio proxy (`src/cli/mcp_stdio.rs`) is request/response only. Needs a bidirectional session channel and response correlation. | A small provider client in a **separate process** (C-27) — not inside `tks serve`. | None beyond the UI. |
| Consent flow | VS Code prompts the user on the first sampling request, lets them restrict which models a server may use (*MCP: List Servers → Configure Model Access*), and exposes request history (*Show Sampling Requests*) ([VS Code MCP guide](https://code.visualstudio.com/api/extension-guides/ai/mcp)). | Explicit user configuration of the endpoint/model; no implicit use of the embeddings Ollama. | n/a |
| Latency | Not measurable (no compatible path in TKS). | Per facilitator turn p50 **0.30 s**, max **1.03 s** (16 turns, warm, think off); cold model load 11.5 s. | Human typing speed. |
| Failure handling | Client may refuse or the user may deny consent → must fall back to typed. | Connection refused / DNS failure surfaced in **2 ms**; request timeout honoured (50 ms probe). Fallback: continue typed. | n/a |
| How turns return | `sampling/createMessage` result → facilitator turn. | HTTP response → facilitator turn. | The user types the facilitator turn. |
| Setup cost | High (transport work, VS Code-only). | Low where Ollama exists; model download otherwise. | None. |

Facilitator spike (`scripts/spikes/ea0_facilitator/facilitator_spike.py`, stdlib only): a neutral
Socratic system prompt with a scripted stakeholder (8 exchanges per cycle) produced three transcripts,
all **accepted by the TKS parser** (`ParsedTranscript::parse`):

| File | `session_id` | `cycle` | `source` | Turns |
| --- | --- | --- | --- | --- |
| `EA0-manual-c1.md` | EA0-manual | 1 | manual | 16 |
| `EA0-ollama-c1.md` | EA0-ollama | 1 | synthetic | 16 |
| `EA0-ollama-c2.md` | EA0-ollama | 2 | synthetic | 16 |

Findings that affect EA-1/EA-5:

- **Neutrality drift:** despite the prompt, `qwen3:8b` asked solution-presupposing questions
  ("What is the primary goal of the *system you're planning to build*?", "…the *new tool*…").
  EA-5 needs a neutrality check (prompt hardening plus a reviewable or rejectable facilitator turn).
- **Turn bounds:** the parser rejects transcripts with fewer than 15 or more than 40 turns. Short
  sessions (the first attempt had 12 and 4 turns) fail validation, so EA-1/EA-4 need either a
  "draft" state or an explicit actionable error before export.
- **Provenance:** `source` allows only `synthetic | manual | asr`. A real session mixing typed and
  spoken turns, or recording which provider generated facilitator turns, has no value today. The
  spike used `synthetic` (the stakeholder was scripted). EA-4 must decide whether the specification
  needs a per-turn or `mixed` provenance (a format change that needs approval).

## 4. ASR Comparison (D-131)

Both models were run by the same harness (`ea0_asr_spike`), with the same VAD gate and the same audio.
Greedy decoding, English, punctuation on, max 448 tokens. Cohere: F32 on CPU, F16 on CUDA.
Whisper: F32 on both (see §4.3).

### 4.1 Accuracy

WER is corpus-level `(S + D + I) / N` after normalisation (§5). "Edits/min" is S+D+I per minute
of audio (a correction-effort proxy). "Clean" is the number of clips needing no correction.
Results were **identical on CPU and GPU** for both models.

| Category (15 clips each) | Cohere WER | Whisper WER | Cohere edits/min | Whisper edits/min | Cohere / Whisper clean |
| --- | --- | --- | --- | --- | --- |
| clean (LibriSpeech) | **2.1%** | 3.3% | **2.9** | 4.5 | **11** / 9 |
| disfluent (AMI IHM) | **9.1%** | 15.5% | **15.8** | 26.9 | **5** / 4 |
| noisy-clean (10 dB SNR) | 5.1% | **4.8%** | 7.0 | **6.6** | 7 / **8** |
| noisy-disfluent (10 dB SNR) | **43.8%** | 54.8% | **75.8** | 94.8 | 0 / 0 |
| noisy-disfluent, excluding the 4 VAD-rejected clips | **23.6%** | 38.5% | — | — | — |

Noisy-disfluent WER is dominated by 4 quiet AMI clips that the VAD gate rejected; each is scored as
all deletions. Whisper's disfluent deletions (24 vs 8) come mostly from it dropping repetitions and
fillers that the AMI references keep.

**Port fidelity.** The upstream port initially scored 46% WER on clean speech: it truncated and
repeated phrases. The root cause was `RelPosAttn::rel_shift`, which is correct only when the number
of relative positions equals the sequence length (see the [vendoring note](../../scripts/spikes/ea0_asr/vendor/cohere-transcribe-candle/VENDORED.md)).
After the fix, on the 30 clean and disfluent clips, the port and the reference PyTorch model produce
**identical normalised text on 24/30 clips**, with 11/552 words (2.0%) differing. All differences are
on AMI clips (filler or repetition choices consistent with dither and numeric noise).

### 4.2 Performance

| Metric | Cohere CPU | Whisper CPU | Cohere GPU | Whisper GPU |
| --- | --- | --- | --- | --- |
| Load + warm-up | 22.5 s | 17.7 s | 27.7 s | 11.7 s |
| Host RSS after load / peak | 9.4 / 14.4 GiB | 3.1 / 4.6 GiB | 4.1 / 7.9 GiB | 0.5 / 1.9 GiB |
| VRAM peak (baseline 0.23 GiB) | — | — | 4.4 GiB | 4.5 GiB |
| Real-time factor (whole set) | **0.61** | 2.15 | 0.19 | **0.06** |
| Processing p50 / p95 | 4.3 / 8.2 s | 15.2 / 26.6 s | 1.4 / 1.6 s | 0.45 / 0.65 s |
| End-of-utterance latency p50 / p95 | 4.9 / 8.8 s | 15.8 / 27.2 s | 2.0 / 2.2 s | 1.0 / 1.2 s |

End-of-utterance latency = VAD hangover (600 ms of trailing silence before the utterance closes) +
processing. Clip lengths are 2–30 s; Cohere scales with audio length, Whisper always pads to a 30 s
window.

**GPU co-tenancy.** With `qwen3:8b` resident (6.2 GiB / 6353 MiB used), both models fail with
`CUDA_ERROR_OUT_OF_MEMORY` (peaks 7.5–7.6 GiB). GPU numbers above were taken with `qwen3:8b`
temporarily unloaded (it was reloaded with keep-alive −1 afterwards). On this hardware, GPU ASR and a
resident facilitator LLM are mutually exclusive.

### 4.3 Runtime notes and limitations

- **Cohere port memory:** upstream staged all weights on the device and then made transposed copies,
  so VRAM held two copies of the model and even an empty 8 GiB GPU went out of memory. The vendored
  copy stages on the CPU and uploads each final tensor once (VENDORED.md change 3). CPU RSS is high
  because the BF16 checkpoint is expanded to F32. Keeping BF16 on CPU was not tested.
- **Whisper F16 is broken in `candle-transformers` 0.9.2:** the encoder builds sinusoidal positions
  in F32, so F16 fails with a dtype mismatch. Whisper therefore runs F32 on CUDA. Its CPU speed is
  limited by a decoder that recomputes the whole sequence each step (no self-attention KV cache).
- **Packaging:** the port is not on crates.io and is a path dependency of the optional feature.
  `cargo package` for a release needs the fixes upstreamed, a published fork, or the crate
  published alongside TKS.
- **Other Rust paths considered:** `cohere_transcribe_rs` (libtorch/`tch`, heavy C++ dependency)
  was not pursued. An ungated ONNX export (`onnx-community/cohere-transcribe-03-2026-ONNX`) via
  `ort` remains a fallback runtime if the candle port becomes a maintenance burden; it was not
  benchmarked.

## 5. Test Set and Scoring Method

| Set | Clips | Source | Licence |
| --- | --- | --- | --- |
| clean | 15 (2.2–29.4 s) | `hf-internal-testing/librispeech_asr_dummy` (validation, clean) | CC-BY-4.0 |
| disfluent | 15 (3.2–7.9 s, ≥8 words) | `edinburghcstr/ami` IHM test split | CC-BY-4.0 |
| noisy-clean / noisy-disfluent | 15 + 15 | Above + white Gaussian noise at 10 dB SNR (xorshift + Box–Muller, seed 20261023) | derived |
| non-speech | 4 × 10 s | Generated: digital silence, room noise σ = 3e-4, loud white noise σ = 0.03, 50 Hz hum at 0.025 | generated |

All audio is public and non-sensitive; it is fetched to a local cache outside the repository
(`fetch_testset.py`) and processed only locally. In total there are 442 s of speech across 60 scored
clips. The set is small: treat WER differences under about 2 points as indicative only.

Normalisation (`scripts/spikes/ea0_asr/wer.rs`, unit-tested):

- Lowercase the text.
- Strip punctuation, keeping apostrophes inside words.
- Split hyphenated words.
- Drop the fillers *hmm, mm, mhm, mmm, uh, um, er, ah*.
- Map *mr/mrs/dr/ok/gonna/wanna* to their reference spellings.
- Spell out integers below 10 000.

Contractions are **not** expanded (for example *we're* vs *WE ARE* counts as 2 edits), which is
neutral between models. Alignment uses word-level Levenshtein with separate S/D/I counts.

## 6. Voice-Activity Gate

Engine: [`earshot`](https://crates.io/crates/earshot) 1.2 (pure Rust, MIT OR Apache-2.0, 16 ms frames
at 16 kHz). Rule: an utterance reaches ASR only if it contains a **sustained** speech run of at least
`min_run_ms` with frame score ≥ `threshold`, tolerating one non-speech frame inside a run. The first
rule, *total* speech ≥ 250 ms, passed loud white noise (336 ms of scattered frames).

| Threshold / min run | Speech clips rejected (of 60) | Non-speech passed (of 4) |
| --- | --- | --- |
| 0.3 / 300 ms | 1 | 1 (white noise, 2.8 s run) |
| 0.4 / 300 ms | 3 | 1 |
| **0.4 / 500 ms (chosen)** | **4** (all noisy-disfluent) | **0** |
| 0.5 / 400 ms | 5 | 0 |

Without the gate, Cohere outputs repeated invented sentences on digital silence, room noise and
mains hum ("I'm going to go to the next slide." ×3), and Whisper outputs "you", "." or "so". With the
gate, all four produce no output. Thresholds were tuned on this same set, so they overfit it. EA-3
should re-validate them on held-out audio and the user's microphone, and expose them as settings.
False rejects only happen on very quiet speech mixed with noise, where the UI should say "nothing
heard" rather than fail silently.

## 7. Audio Data Flow (D-132, C-28)

**Recommended transport:** an authenticated **WebSocket on the existing axum gateway** (enable
axum's `ws` feature; no second web stack). It carries binary frames of PCM16 mono 16 kHz audio
captured by an `AudioWorklet`.

- **Authentication:** browsers cannot set `Authorization` on a WebSocket upgrade, and the existing
  `TraceLayer` logs request URIs. The browser should therefore obtain a short-lived, single-use
  ticket through an authenticated POST and present it in `Sec-WebSocket-Protocol`. It must never go
  in the query string.
- **Why not chunked POST:** it was rejected because each chunk is a separate request, which
  complicates streaming VAD and back-pressure, and every chunk passes through body-limit and
  tracing middleware.

| # | Where audio exists | Form | Bound | Lifetime / disposal | Persisted? | Logged? |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | Browser `MediaStream` (`getUserMedia`) | Browser-internal | Browser-managed | Tracks stopped on pause/end/unload | No — **no `MediaRecorder`, Blob, IndexedDB, `localStorage` or download path** | No |
| 2 | `AudioWorklet` render quanta → ring | Float32, 128 samples | Fixed ring (for example 1 s) | Overwritten in place; released on stop | No | No |
| 3 | Main thread → `WebSocket.send` | Int16 frames (20 ms = 640 B) | Pause capture when `bufferedAmount` > cap (for example 64 KiB) | Owned by the socket; freed on send/close | No | No |
| 4 | Network | Loopback/LAN WebSocket frames | Max message size enforced server-side | In flight only | No | No (frames never traced) |
| 5 | Server socket read buffer | Bytes | axum/tungstenite message limit | Dropped per message | No | No |
| 6 | Server utterance ring (VAD in progress) | f32 | **30 s** (`UtteranceBuffer::MAX_SECONDS`); overflow closes the utterance | Zeroed on `Drop` | No | No |
| 7 | VAD frame | `[f32; 256]` | 16 ms | Zeroed after scoring | No | Scores only (numbers) |
| 8 | ASR input: dithered copy, mel features, tensors (CPU/GPU) | f32/f16 | Proportional to ≤30 s | Dithered copy zeroed; features/tensors freed after `transcribe` (device memory returns to the allocator pool; not zeroed — §8) | No | Port logs timings, tensor shapes and token counts only |
| 9 | Inference queue | Utterances awaiting ASR | **One** in flight + at most one queued; further utterances are rejected with a UI notice | Dropped (zeroed) on reject | No | Reject event, no audio |
| 10 | Error paths (decode error, ASR error, socket close, cancel) | Any of 5–8 | — | Owned values drop on early return; `Drop` zeroes 6–7 | No | Error kind only, never samples |
| 11 | Panic | Any of 5–8 | — | Release profile uses `panic = unwind` (default), so `Drop` runs | No | Panic message must not format buffers (no `Debug` on audio types) |
| 12 | Process kill / crash | Process memory | — | Released by the OS | No | Core dumps: see §8 |

What the spike enforces in code: `UtteranceBuffer` has a hard 30 s bound, zeroes on `Drop`, and
deliberately does **not** implement `Debug`, `Clone` or `Serialize`, so samples cannot reach logs,
reports or panic messages. The dithered copy and VAD frames are explicitly zeroed. The harness writes
only text, numbers and VAD statistics to its JSON report.

## 8. Limitations and EA-6 Hardening Items

- **Zeroing is best effort.** Candle tensors, GPU allocations and browser buffers are freed rather
  than zeroed. The hard guarantee is *no persistence and no logging*, not memory sanitisation.
- **Swap and core dumps:** add `prctl(PR_SET_DUMPABLE, 0)` / `RLIMIT_CORE = 0` for the ASR process.
  Consider `mlock` on the utterance ring or document that swap may hold volatile audio.
- MCP sampling latency and consent UX were not measured; there is no compatible TKS transport.
- No real-microphone, accented or far-field audio. The test set is read speech plus meeting
  headset audio with synthetic noise.
- GPU and CPU numbers are single runs. Ollama (GPU) and CPU ASR were not stress-tested concurrently,
  but the facilitator run did overlap a CPU ASR run with no visible facilitator slowdown.

## 9. Recommendations and Decisions Requested

1. **Providers:** implement the provider boundary with **Ollama out-of-process** and
   **typed/manual** in EA-1/EA-5. Defer MCP sampling until TKS gains a bidirectional MCP transport
   and more clients support it; record it as a D-130 follow-up rather than reopening the decision.
2. **ASR:** keep **Cohere Transcribe** (D-131 confirmed) on candle, using the patched port. Upstream
   the two fixes, or publish a maintained fork, before release. Keep Whisper large-v3-turbo as the
   fallback; it needs no vendoring.
3. **Device policy (decision needed):** on this 8 GiB GPU the user must choose between:
   - **(a)** CPU ASR (EoU p50 ≈ 5 s, about 9.5 GiB RAM) with the GPU left to Ollama; or
   - **(b)** GPU ASR (EoU p50 ≈ 2 s) with the facilitator model on CPU, a smaller model, or unloaded
     while dictating.

   The recommendation is **(a) as the default, with GPU opt-in when VRAM is free**.
4. **VAD:** earshot, threshold 0.4, min run 500 ms, hangover 600 ms, exposed as settings.
5. **Transport:** an authenticated WebSocket on the existing gateway, using a ticket in
   `Sec-WebSocket-Protocol`, bounded frames, a 30 s utterance bound and a single-slot inference
   queue.
6. **Transcript format (approval needed):** decide how to represent mixed typed/spoken provenance,
   and how drafts below the 15-turn minimum are handled.

## 10. Reproduction

> [!NOTE]
> The server-side ASR reproduction code (`scripts/spikes/ea0_asr/`, the `ea0_asr_spike` binary, and the `asr-spike` Cargo feature) was removed in EA-1 following the client-side streaming decision (D-131, EA-0.2). It is preserved and recoverable from git history at commit `60f25c3`.

All caches live outside the repository. Requires `HF_READ_TOKEN` in the environment for the gated
Cohere download; the token is never logged or written to disk.

```bash
export TKS_MODEL_CACHE=/workspaces/.tks-cache/models TKS_EA0_CACHE=/workspaces/.tks-cache
scripts/spikes/ea0_asr/fetch_models.sh
TKS_EA0_TESTSET=/workspaces/.tks-cache/ea0-testset python3 scripts/spikes/ea0_asr/fetch_testset.py
cargo build --release --features asr-spike --bin ea0_asr_spike
scripts/spikes/ea0_asr/run_offline.sh cohere cpu     # results + network syscall log in $TKS_EA0_CACHE/ea0-results
scripts/spikes/ea0_asr/run_offline.sh whisper cpu
./target/release/ea0_asr_spike --model cohere --vad-only --models-dir "$TKS_MODEL_CACHE" \
  --testset /workspaces/.tks-cache/ea0-testset        # VAD calibration only
python3 scripts/spikes/ea0_facilitator/facilitator_spike.py --out /workspaces/.tks-cache/ea0-results/facilitator
```

GPU: build inside `nvidia/cuda:12.8.1-devel-ubuntu24.04` (add `libssl-dev` and `pkg-config`) with:

```bash
CUDA_COMPUTE_CAP=86 cargo build --release --bin ea0_asr_spike \
  --features asr-spike,cohere-transcribe-candle/cuda,candle-core/cuda,candle-nn/cuda,candle-transformers/cuda
```

Then run the binary with `--device cuda` under `docker run --network none --device nvidia.com/gpu=all`.
