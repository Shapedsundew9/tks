#!/usr/bin/env python3
"""EA-0 spike: fetch the reproducible English ASR evaluation set.

Sources (both CC-BY-4.0, public, non-sensitive):
  * clean     - LibriSpeech dev-clean utterances (hf-internal-testing/librispeech_asr_dummy)
  * disfluent - AMI meeting corpus, individual headset mics, test split (edinburghcstr/ami, ihm)

Selection is deterministic: rows are scanned in dataset order and the first that satisfy the
rules are taken. Original files (FLAC/WAV, 16 kHz) are written unmodified to
$TKS_EA0_TESTSET (default ~/.cache/tks/ea0-testset) with manifest.json. The Rust harness
(`ea0_asr_spike`) derives the seeded noisy variants and the non-speech clips itself.
"""

import json
import os
import pathlib
import sys
import urllib.parse
import urllib.request

ROWS_API = "https://datasets-server.huggingface.co/rows"
N_CLEAN = 15
N_DISFLUENT = 15
EXTENSIONS = {"audio/flac": "flac", "audio/wav": "wav", "audio/x-wav": "wav"}

OUT = pathlib.Path(os.environ.get("TKS_EA0_TESTSET", pathlib.Path.home() / ".cache/tks/ea0-testset"))


def rows(dataset, config, split, offset, length=100):
    query = urllib.parse.urlencode(
        {"dataset": dataset, "config": config, "split": split, "offset": offset, "length": length}
    )
    with urllib.request.urlopen(f"{ROWS_API}?{query}", timeout=60) as resp:
        return json.load(resp)["rows"]


def collect(dataset, config, split, wanted, accept, category, license_):
    picked, offset = [], 0
    while len(picked) < wanted:
        batch = rows(dataset, config, split, offset)
        if not batch:
            break
        for item in batch:
            row = item["row"]
            if not accept(row):
                continue
            audio = row["audio"][0]
            clip_id = f"{category}-{len(picked) + 1:02d}"
            name = f"{clip_id}.{EXTENSIONS[audio['type']]}"
            with urllib.request.urlopen(audio["src"], timeout=60) as resp:
                (OUT / name).write_bytes(resp.read())
            picked.append(
                {
                    "id": clip_id,
                    "category": category,
                    "file": name,
                    "reference": row["text"],
                    "source": f"{dataset}/{config}/{split}#row{item['row_idx']}",
                    "license": license_,
                }
            )
            if len(picked) == wanted:
                break
        offset += len(batch)
    return picked


def ami_ok(row):
    duration = row["end_time"] - row["begin_time"]
    return 3.0 <= duration <= 20.0 and len(row["text"].split()) >= 8


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    clips = collect(
        "hf-internal-testing/librispeech_asr_dummy", "clean", "validation", N_CLEAN,
        lambda r: True, "clean", "CC-BY-4.0 (LibriSpeech)",
    )
    clips += collect(
        "edinburghcstr/ami", "ihm", "test", N_DISFLUENT, ami_ok, "disfluent", "CC-BY-4.0 (AMI)",
    )
    (OUT / "manifest.json").write_text(json.dumps(clips, indent=2))
    print(f"wrote {len(clips)} clips to {OUT}", file=sys.stderr)


if __name__ == "__main__":
    main()
