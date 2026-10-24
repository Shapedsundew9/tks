#!/usr/bin/env bash
# EA-0 spike: download pinned ASR model revisions to a local cache (D-131).
#
# The Cohere repository is gated; the Hugging Face token is read from
# $HF_READ_TOKEN at download time only and is never written to disk or logged.
# After download, transcription runs fully offline.
set -euo pipefail

CACHE="${TKS_MODEL_CACHE:-$HOME/.cache/tks/models}"

COHERE_REPO="CohereLabs/cohere-transcribe-03-2026"
COHERE_REV="d552dadc2922ba6f6c1f994f8db904d52dcaf942"
COHERE_FILES=(config.json generation_config.json preprocessor_config.json tokenizer.json
  tokenizer_config.json special_tokens_map.json model.safetensors)

WHISPER_REPO="openai/whisper-large-v3-turbo"
WHISPER_REV="41f01f3fe87f28c78e2fbf8b568835947dd65ed9"
WHISPER_FILES=(config.json generation_config.json preprocessor_config.json tokenizer.json
  model.safetensors)

fetch() {
  local repo="$1" rev="$2" dest="$3"
  shift 3
  mkdir -p "$dest"
  for f in "$@"; do
    if [[ -s "$dest/$f" ]]; then
      echo "skip  $repo/$f (cached)"
      continue
    fi
    echo "fetch $repo@${rev:0:8}/$f"
    if [[ -n "${HF_READ_TOKEN:-}" ]]; then
      curl -fSL --retry 3 -o "$dest/$f.part" \
        -H @<(printf 'Authorization: Bearer %s\n' "$HF_READ_TOKEN") \
        "https://huggingface.co/$repo/resolve/$rev/$f"
    else
      curl -fSL --retry 3 -o "$dest/$f.part" "https://huggingface.co/$repo/resolve/$rev/$f"
    fi
    mv "$dest/$f.part" "$dest/$f"
  done
  printf '%s\n%s\n' "$repo" "$rev" >"$dest/REVISION"
}

fetch "$COHERE_REPO" "$COHERE_REV" "$CACHE/cohere-transcribe-03-2026" "${COHERE_FILES[@]}"
fetch "$WHISPER_REPO" "$WHISPER_REV" "$CACHE/whisper-large-v3-turbo" "${WHISPER_FILES[@]}"

# The candle port decodes with an id -> piece map; derive it from tokenizer.json
# (equivalent to the port's tools/extract_vocab.py, without sentencepiece).
python3 - "$CACHE/cohere-transcribe-03-2026" <<'PY'
import json, pathlib, sys
d = pathlib.Path(sys.argv[1])
tok = json.loads((d / "tokenizer.json").read_text())
vocab = tok["model"]["vocab"]
if isinstance(vocab, dict):
    pieces = {str(i): p for p, i in vocab.items()}
else:
    pieces = {str(i): entry[0] for i, entry in enumerate(vocab)}
for added in tok.get("added_tokens", []):
    pieces[str(added["id"])] = added["content"]
(d / "vocab.json").write_text(json.dumps(pieces, ensure_ascii=False))
print(f"wrote vocab.json ({len(pieces)} pieces)")
PY

echo "models cached under $CACHE"
