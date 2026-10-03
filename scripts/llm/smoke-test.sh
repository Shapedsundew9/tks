#!/usr/bin/env bash
set -euo pipefail

OLLAMA_HOST="${OLLAMA_HOST:-http://ollama:11434}"
MODEL="${1:-${TKS_LLM_MODEL:-qwen3:8b}}"

echo "Smoke testing Ollama at ${OLLAMA_HOST} using model ${MODEL}"

curl --fail --silent --show-error "${OLLAMA_HOST}/api/version" >/dev/null
echo "✓ API version endpoint is reachable"

tags_json="$(curl --fail --silent --show-error "${OLLAMA_HOST}/api/tags")"
python3 -c '
import json
import sys

model = sys.argv[1]
payload = json.loads(sys.argv[2])
names = {m.get("name", "") for m in payload.get("models", [])}
if model not in names:
    print(f"Model not found: {model}", file=sys.stderr)
    print("Run scripts/llm/pull-models.sh first.", file=sys.stderr)
    sys.exit(1)
' "${MODEL}" "${tags_json}"
echo "✓ Model is present"

curl --fail --silent --show-error \
  -X POST "${OLLAMA_HOST}/v1/chat/completions" \
  -H "Content-Type: application/json" \
  -d "{\"model\":\"${MODEL}\",\"messages\":[{\"role\":\"user\",\"content\":\"Reply with exactly OK.\"}],\"max_tokens\":8}" \
  >/dev/null
echo "✓ OpenAI-compatible /v1 chat completion succeeded"

ps_json="$(curl --fail --silent --show-error "${OLLAMA_HOST}/api/ps")"
python3 -c '
import json
import sys

model = sys.argv[1]
payload = json.loads(sys.argv[2])
for item in payload.get("models", []):
    if item.get("name") != model:
        continue
    total = item.get("size")
    vram = item.get("size_vram")
    if isinstance(total, int) and isinstance(vram, int):
        if total == vram:
            print("✓ Model is fully resident in GPU VRAM")
        else:
            print(
                f"⚠ Model appears partially offloaded to CPU "
                f"(size_vram={vram}, size={total})"
            )
    else:
        print("⚠ Could not determine VRAM residency from /api/ps payload")
    break
else:
    print("⚠ Model is not currently loaded (this is acceptable immediately after idle unload)")
' "${MODEL}" "${ps_json}"

echo "Smoke test complete."
