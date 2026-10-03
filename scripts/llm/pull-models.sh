#!/usr/bin/env bash
set -euo pipefail

OLLAMA_HOST="${OLLAMA_HOST:-http://ollama:11434}"
DEFAULT_MODELS=("qwen3:8b" "gemma4:e4b")

usage() {
  cat <<'EOF'
Usage: pull-models.sh [--yes] [model ...]

Examples:
  scripts/llm/pull-models.sh
  scripts/llm/pull-models.sh --yes qwen3:8b
EOF
}

confirm=true
models=()

while (($# > 0)); do
  case "$1" in
    -y|--yes)
      confirm=false
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      models+=("$1")
      shift
      ;;
  esac
done

if ((${#models[@]} == 0)); then
  models=("${DEFAULT_MODELS[@]}")
fi

echo "Using Ollama endpoint: ${OLLAMA_HOST}"
if ! curl --fail --silent --show-error "${OLLAMA_HOST}/api/version" >/dev/null; then
  echo "Error: Ollama is not reachable at ${OLLAMA_HOST}" >&2
  exit 1
fi

tags_json="$(curl --fail --silent --show-error "${OLLAMA_HOST}/api/tags")"

model_exists() {
  local candidate="$1"
  python3 -c '
import json
import sys

candidate = sys.argv[1]
payload = json.loads(sys.argv[2])
names = {m.get("name", "") for m in payload.get("models", [])}
sys.exit(0 if candidate in names else 1)
' "$candidate" "${tags_json}"
}

to_pull=()
for model in "${models[@]}"; do
  if model_exists "${model}"; then
    echo "Already present: ${model}"
  else
    to_pull+=("${model}")
  fi
done

if ((${#to_pull[@]} == 0)); then
  echo "Nothing to pull."
  exit 0
fi

echo "Will pull ${#to_pull[@]} model(s):"
for model in "${to_pull[@]}"; do
  echo "  - ${model}"
done

if [[ "${confirm}" == true ]]; then
  read -r -p "Proceed with download(s)? [y/N] " answer
  case "${answer}" in
    y|Y|yes|YES) ;;
    *)
      echo "Cancelled."
      exit 0
      ;;
  esac
fi

for model in "${to_pull[@]}"; do
  echo "Pulling ${model}..."
  if ! response="$(curl --fail-with-body --silent --show-error \
    -X POST "${OLLAMA_HOST}/api/pull" \
    -H "Content-Type: application/json" \
    -d "{\"model\":\"${model}\",\"stream\":false}")"; then
    echo "Error: failed to pull ${model}: ${response}" >&2
    exit 1
  fi
  echo "Pulled ${model}"
done

echo "Done."
