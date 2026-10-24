#!/usr/bin/env bash
# EA-0 spike: run the ASR harness with no network access and record every network syscall.
#
# The process runs in a fresh network namespace (loopback only, down) and under strace
# filtered to the %network syscall class. The resulting syscall count is the evidence for
# "no network calls during transcription" (C-28).
#
#   scripts/spikes/ea0_asr/run_offline.sh cohere cpu
set -euo pipefail

MODEL="${1:?model: cohere|whisper}"
DEVICE="${2:-cpu}"
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
CACHE="${TKS_EA0_CACHE:-$HOME/.cache/tks}"
OUT="$CACHE/ea0-results"
BIN="${EA0_BIN:-$ROOT/target/release/ea0_asr_spike}"
mkdir -p "$OUT"

unshare -rn strace -f --seccomp-bpf -e trace=%network -o "$OUT/$MODEL-$DEVICE.netlog" \
  "$BIN" --model "$MODEL" --device "$DEVICE" \
  --models-dir "$CACHE/models" --testset "$CACHE/ea0-testset" \
  --out "$OUT/$MODEL-$DEVICE.json" "${@:3}"

calls=$(grep -cvE '^[0-9]+ +(\+\+\+|---)' "$OUT/$MODEL-$DEVICE.netlog" || true)
echo "network syscalls during run: $calls (log: $OUT/$MODEL-$DEVICE.netlog)"
