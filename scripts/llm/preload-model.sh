#!/usr/bin/env bash
# Load a model into Ollama and keep it resident indefinitely.
# Non-fatal by design so it is safe to run from devcontainer start hooks.
set -uo pipefail

OLLAMA_HOST="${OLLAMA_HOST:-http://ollama:11434}"
MODEL="${1:-${TKS_LLM_MODEL:-qwen3:8b}}"

for _ in $(seq 1 30); do
  curl --fail --silent "${OLLAMA_HOST}/api/version" >/dev/null && break
  sleep 2
done

tags_json="$(curl --fail --silent --show-error "${OLLAMA_HOST}/api/tags")" || {
  echo "Preload skipped: Ollama is not reachable at ${OLLAMA_HOST}" >&2
  exit 0
}

if ! python3 -c '
import json
import sys

names = {m.get("name", "") for m in json.loads(sys.argv[2]).get("models", [])}
sys.exit(0 if sys.argv[1] in names else 1)
' "${MODEL}" "${tags_json}"; then
  echo "Preload skipped: ${MODEL} is not pulled. Run scripts/llm/pull-models.sh first." >&2
  exit 0
fi

echo "Preloading ${MODEL} (keep_alive=-1)..."
if response="$(curl --fail-with-body --silent --show-error \
  -X POST "${OLLAMA_HOST}/api/generate" \
  -H "Content-Type: application/json" \
  -d "{\"model\":\"${MODEL}\",\"keep_alive\":-1}")"; then
  echo "Preloaded ${MODEL}"
else
  echo "Preload failed for ${MODEL}: ${response}" >&2
fi
