#!/usr/bin/env python3
"""EA-0 spike: out-of-process conversation provider (Ollama) vs typed/manual entry.

TKS never runs LLM inference itself (C-28/D-130). This script plays the role of an
*external* provider: it asks a local Ollama model for neutral, Socratic facilitator
questions while a scripted stakeholder answers, then writes a `tks-transcript/v1`
document that TKS ingests unchanged. It also writes a typed/manual baseline and a
cycle-2 continuation so the parser round-trip can be checked for every path.

Usage:
    facilitator_spike.py --out DIR [--host http://ollama:11434] [--model qwen3:8b]
"""

from __future__ import annotations

import argparse
import json
import statistics
import time
import urllib.error
import urllib.request
from pathlib import Path

SYSTEM_PROMPT = """You are a neutral requirements facilitator interviewing a stakeholder.
Rules:
- Ask exactly ONE short open question per turn (max 30 words).
- Build on what the stakeholder just said; ask about goals, users, constraints,
  boundaries, success criteria and exceptions.
- Never propose solutions, technologies or features. Never lead or assume an answer.
- Do not summarise, praise or add commentary. Output only the question."""

# Scripted stakeholder answers keep the run deterministic on the stakeholder side,
# so differences between runs come only from the provider.
STAKEHOLDER_SCRIPT = [
    "We run a small community repair cafe. Volunteers fix people's broken items on "
    "Saturdays, but we lose track of what came in and who is working on it.",
    "Mostly the volunteer coordinator, Ana, and the fixers themselves. Visitors just "
    "want to know when their toaster or bike is ready.",
    "Paper tickets get lost. Some items wait three weeks because nobody remembers "
    "they need a spare part ordered.",
    "We must not collect more personal data than a first name and a phone number. "
    "Some visitors are wary of giving anything at all.",
    "Success would be every item having an owner and a status, and no item waiting "
    "more than two weekends without someone knowing why.",
    "Electrical items need a qualified fixer to sign off a safety test before they "
    "go back. That is non-negotiable for our insurance.",
    "Fixers come and go; a third of them only show up once a month, so anything "
    "they need to learn has to be obvious on the day.",
    "We open at ten and the queue is worst in the first hour. Logging an item "
    "cannot take longer than a minute or people walk off.",
]
CYCLE2_SCRIPT = [
    "Since last time we realised parts ordering is the real bottleneck; the fixers "
    "want to flag an item as waiting for a part with an expected date.",
    "Ana should see a weekly list of items waiting on parts, sorted by how long "
    "they have been waiting.",
    "Some parts are donated by members, so the expected date is often unknown. "
    "We still need to know who promised to bring it.",
    "If a part never arrives the visitor should be told after two weekends and "
    "offered their item back.",
    "Visitors should not see who promised the part; that stays internal to "
    "volunteers.",
    "We do not want reminders by text message; Ana phones people herself because "
    "it is friendlier.",
    "Items waiting on parts still need the electrical safety sign-off when the "
    "part is fitted, even if the original fixer has left.",
    "If an item has waited more than six weeks Ana wants to decide case by case "
    "rather than have a rule close it automatically.",
]

PARTICIPANTS = [("Ana", "stakeholder"), ("Facilitator", "facilitator")]


def chat(host: str, model: str, messages: list[dict], timeout: float) -> tuple[str, float]:
    body = json.dumps(
        {
            "model": model,
            "messages": messages,
            "stream": False,
            "think": False,
            "options": {"temperature": 0.2, "num_ctx": 8192},
        }
    ).encode()
    req = urllib.request.Request(
        f"{host}/api/chat", data=body, headers={"Content-Type": "application/json"}
    )
    t0 = time.perf_counter()
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        payload = json.load(resp)
    return payload["message"]["content"].strip(), time.perf_counter() - t0


def clean_question(text: str) -> str:
    """Single line, no markdown emphasis that would collide with turn headers."""
    text = " ".join(text.replace("**", "").split())
    return text.strip('"')


def render(session_id: str, cycle: int, title: str, source: str, turns: list[tuple[str, str]]) -> str:
    roles = dict(PARTICIPANTS)
    lines = [
        "---",
        "format: tks-transcript/v1",
        f'title: "{title}"',
        f'session_id: "{session_id}"',
        f"cycle: {cycle}",
        f'date: "{time.strftime("%Y-%m-%d")}"',
        'language: "en"',
        f'source: "{source}"',
        "participants:",
        *[f'  - {{ name: "{n}", role: "{r}" }}' for n, r in PARTICIPANTS],
        "---",
        "",
    ]
    for i, (speaker, text) in enumerate(turns, 1):
        lines += [f"**[t{i}] {speaker} ({roles[speaker]}):** {text}", ""]
    return "\n".join(lines)


def run_session(host, model, script, timeout, history=None):
    messages = [{"role": "system", "content": SYSTEM_PROMPT}]
    if history:
        messages.append({"role": "user", "content": "Summary of the previous cycle:\n" + history})
    messages.append({"role": "user", "content": "Begin the interview with your first question."})
    turns, latencies = [], []
    for answer in script:
        question, dt = chat(host, model, messages, timeout)
        question = clean_question(question)
        latencies.append(dt)
        turns += [("Facilitator", question), ("Ana", answer)]
        messages += [
            {"role": "assistant", "content": question},
            {"role": "user", "content": answer},
        ]
    return turns, latencies


def failure_probe(label, host, model, timeout):
    t0 = time.perf_counter()
    try:
        chat(host, model, [{"role": "user", "content": "Say hi."}], timeout)
        outcome = "unexpected success"
    except (urllib.error.URLError, TimeoutError, OSError) as exc:
        outcome = f"{type(exc).__name__}: {getattr(exc, 'reason', exc)}"
    return {"probe": label, "outcome": outcome, "elapsed_s": round(time.perf_counter() - t0, 3)}


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--host", default="http://ollama:11434")
    ap.add_argument("--model", default="qwen3:8b")
    ap.add_argument("--timeout", type=float, default=120.0)
    args = ap.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)

    # Warm-up so the first measured turn excludes model load.
    _, warm = chat(args.host, args.model, [{"role": "user", "content": "Reply OK."}], args.timeout)

    turns, lat = run_session(args.host, args.model, STAKEHOLDER_SCRIPT, args.timeout)
    (args.out / "EA0-ollama-c1.md").write_text(
        render("EA0-ollama", 1, "Repair cafe intake (Ollama facilitator)", "synthetic", turns)
    )
    recap = " ".join(t for s, t in turns if s == "Ana")
    turns2, lat2 = run_session(args.host, args.model, CYCLE2_SCRIPT, args.timeout, history=recap)
    (args.out / "EA0-ollama-c2.md").write_text(
        render("EA0-ollama", 2, "Repair cafe intake cycle 2", "synthetic", turns2)
    )

    # Typed/manual baseline: a human types both sides; no provider involved.
    manual_q = [
        "What problem are you trying to solve?",
        "Who is involved day to day?",
        "What goes wrong today?",
        "What must the solution never do?",
        "How would you know it is working?",
        "Are there any rules that cannot be broken?",
        "Who are the fixers and how often do they come?",
        "How busy does it get on a Saturday?",
    ]
    manual = [x for q, a in zip(manual_q, STAKEHOLDER_SCRIPT) for x in (("Facilitator", q), ("Ana", a))]
    (args.out / "EA0-manual-c1.md").write_text(
        render("EA0-manual", 1, "Repair cafe intake (typed)", "manual", manual)
    )

    all_lat = lat + lat2
    report = {
        "host": args.host,
        "model": args.model,
        "warmup_s": round(warm, 3),
        "turns": len(all_lat),
        "latency_s": {
            "p50": round(statistics.median(all_lat), 3),
            "max": round(max(all_lat), 3),
            "per_turn": [round(x, 3) for x in all_lat],
        },
        "questions": [t for s, t in turns + turns2 if s == "Facilitator"],
        "failures": [
            failure_probe("unreachable host", "http://127.0.0.1:9", args.model, 5),
            failure_probe("unknown DNS name", "http://ollama-missing.invalid:11434", args.model, 5),
            failure_probe("timeout", args.host, args.model, 0.05),
        ],
    }
    (args.out / "facilitator-report.json").write_text(json.dumps(report, indent=2))
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
