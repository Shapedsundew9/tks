# Vendored: cohere-transcribe-candle

- **Upstream:** <https://github.com/JustMaier/cohere-transcribe-candle>
- **Revision:** `a37a812fd7983a1e8c8699b40980ecda84da82d4` (2026-04-13, latest at vendoring)
- **Licence:** Apache-2.0, as declared in the upstream `Cargo.toml` and README (upstream ships no
  `LICENSE` file; the standard Apache-2.0 text is included here as [LICENSE](LICENSE)).
- **Copyright:** the upstream author(s). Vendored for the TKS EA-0 spike only; not published.

## Local modifications

1. `src/decoder.rs`: add `#[cfg(not(feature = "flash-attn"))] use candle_nn::ops;`. Without it the
   non-flash-attention path (every CPU build) fails to compile with
   `E0433: cannot find module or crate 'ops'`.
2. `src/encoder.rs` (`RelPosAttn::rel_shift`): correctness fix. Upstream viewed the padded
   `(t, n_pos+1)` scores as `(n_pos+1, t)` and took rows `1..t+1`, which equals the
   Transformer-XL/NeMo relative shift only when `n_pos == t`; with `n_pos = 2t-1` every frame
   after the first attends with the wrong relative offsets. The fix drops the first row, views
   back as `(t, n_pos)` and keeps the first `t` columns, as NeMo does. Before the fix the port
   truncated and repeated phrases (clean LibriSpeech WER 46%); after it, output matches the
   reference `transformers` implementation (see `docs/vision/spike-ea0-results.md`).
3. `Cargo.toml`: removed dependencies used only by the upstream examples and server (`axum 0.7`,
   `tokio`, `tower-http`, `tempfile`, `clap`, `tracing-subscriber`, `thiserror`, `safetensors`,
   `num-complex`), made `half` CUDA-only, removed `[[example]]` targets and `[profile]`, and set
   `publish = false`.

`examples/`, `tools/` and the upstream `Cargo.lock` are not vendored.
