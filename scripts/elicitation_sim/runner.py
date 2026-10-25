#!/usr/bin/env python3
"""Multi-turn Intent Elicitation simulation testbed and benchmarking runner (EA-5, EB-8).

Drives closed-loop multi-turn interviews between a simulated Stakeholder persona
and the TKS Socratic Facilitator / Inspiration Assistant to quantitatively measure:
1. Turns to Foundation Anchor Resolution (Scope, Actors, Invariants)
2. 5-Axis Orthogonal Domain Coverage
3. Neutrality & Technology Presupposition Compliance

Modes:
- 'scripted': Uses canonical scripted answers from the brief (fast, deterministic CI).
- 'local': Uses local Ollama model (e.g. qwen3:8b) for the stakeholder persona.
- 'commercial': Uses an OpenAI-compatible endpoint (e.g. OpenAI/Anthropic/Gemini) for the persona.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any

from metrics import evaluate_simulation, format_markdown_report

# Default locations
BRIEFS_DIR = Path(__file__).parent / "briefs"
DEFAULT_OUT_DIR = Path(__file__).parent / "results"

FACILITATOR_SYSTEM_PROMPT_V2 = """\
You are a sharp, insightful Socratic facilitator helping a stakeholder discover and define real-world domain requirements, operational boundaries, and rules.

You operate by binary search over an orthogonal domain ontology to maximize information gain and eliminate ambiguity across 5 universal domain axes:
1. Agency & Responsibility: Who acts, who decides, authority limits, delegation, supervisor overrides.
2. Preconditions & Invariants: Non-negotiable rules, safety constraints, eligibility criteria, policy floors.
3. Exception & Conflict Resolution: Competing claims, missing inputs, absence of personnel, failure recovery.
4. Temporal Cadence & Lifecycle: Deadlines, scheduling, continuous vs staged progression, reversibility vs finality.
5. Evidence & Accountability: Verification proof, dispute records, transparency, decision justifications.

Follow these strict rules:
1. Ask exactly ONE question per response.
2. Present a concrete, high-entropy bisection (a fork in the road): 'If [Scenario X occurs], does the domain rule dictate Option A or Option B?'
3. Anchor your scenario in the stakeholder's specific domain and vocabulary—do NOT ask generic textbook questions.
4. You may optionally prefix your question with the axis category in brackets, e.g. '[Exception & Conflict] If a visitor...' or '[Authority Boundary] When two coordinators...'.
5. Do NOT repeat or rephrase anything listed in the 'Already Explored' section.
6. NEVER presuppose solutions or software: never refer to 'the system/tool/app/platform' or 'your software'. Inquire strictly about real-world domain workflows, human roles, and operational rules.
7. NEVER introduce technologies, architectures, or technical mechanisms (no databases, APIs, servers, queues, caches).
8. Keep the inquiry under 2 sentences: direct, vivid, and neutral."""


def resolve_ollama_host(raw: str = "") -> str:
    host = raw.strip() or os.getenv("OLLAMA_HOST", "")
    if not host:
        # Check if ollama is resolvable (e.g. devcontainer)
        import socket
        try:
            socket.gethostbyname("ollama")
            return "http://ollama:11434"
        except Exception:
            return "http://localhost:11434"
    if not host.startswith("http://") and not host.startswith("https://"):
        host = f"http://{host}"
    return host.rstrip("/")


def query_ollama(base_url: str, model: str, messages: list[dict[str, str]]) -> str:
    """Invokes Ollama /api/chat synchronously."""
    url = f"{base_url}/api/chat"
    req_body = json.dumps({
        "model": model,
        "messages": messages,
        "stream": False,
        "options": {"temperature": 0.3},
    }).encode("utf-8")

    req = urllib.request.Request(
        url,
        data=req_body,
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=45) as resp:
        data = json.loads(resp.read().decode("utf-8"))
        return data.get("message", {}).get("content", "").strip()


def query_commercial_llm(endpoint: str, api_key: str, model: str, messages: list[dict[str, str]]) -> str:
    """Invokes OpenAI-compatible endpoint."""
    url = f"{endpoint.rstrip('/')}/v1/chat/completions"
    req_body = json.dumps({
        "model": model,
        "messages": messages,
        "temperature": 0.4,
    }).encode("utf-8")

    req = urllib.request.Request(
        url,
        data=req_body,
        headers={
            "Content-Type": "application/json",
            "Authorization": f"Bearer {api_key}",
        },
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=45) as resp:
        data = json.loads(resp.read().decode("utf-8"))
        return data["choices"][0]["message"]["content"].strip()


def query_agy_cli(
    model: str,
    system_prompt: str,
    user_prompt: str,
    conversation_id: str | None = None,
) -> tuple[str, str | None]:
    """Invokes the agy CLI non-interactively using -p with conversation session resumption.
    
    Reuses existing conversation sessions to leverage prompt caching and eliminate
    session startup overhead.
    """
    cmd = [
        "agy",
        "--disable-slash-commands",
        "--output-format", "json",
    ]
    if conversation_id:
        cmd.extend(["--conversation", conversation_id])
        prompt_text = f"The facilitator asks: \"{user_prompt}\"\n\nYour concise answer:"
    else:
        cmd.extend(["--model", model or "gemini-3.8-flash-low"])
        prompt_text = f"System Instructions:\n{system_prompt}\n\nThe facilitator asks: \"{user_prompt}\"\n\nYour concise answer:"

    cmd.extend(["-p", prompt_text])

    try:
        proc = subprocess.run(cmd, capture_output=True, text=True, timeout=60, check=True)
        raw_output = proc.stdout.strip()
        data = json.loads(raw_output)
        cid = data.get("conversation_id", conversation_id)
        resp_text = data.get("response", "").strip()
        cleaned = re.sub(r"^(?:Stakeholder Answer|Answer|Stakeholder):\s*", "", resp_text, flags=re.I)
        return cleaned.strip().strip('"'), cid
    except Exception as e:
        print(f"    [Warning: agy CLI invocation failed ({e}), falling back]", file=sys.stderr)
        return "Our established operational protocol requires strict compliance with that procedure.", conversation_id


def build_facilitator_prompt(brief_title: str, turns: list[dict[str, str]], explored: list[str]) -> str:
    """Constructs the user directive for the facilitator using the 3 Foundation Anchors check."""
    lines = [f"Session Goal / Title: {brief_title}\n"]
    lines.append("Established Requirements & Statements So Far:")
    for idx, t in enumerate(turns):
        lines.append(f"[t{idx+1}] ({t['role']}): {t['text']}")

    if explored:
        lines.append("\nAlready Explored / Excluded Ideas & Questions (DO NOT repeat or overlap):")
        for item in explored:
            lines.append(f"- {item}")

    directive = """\nTask:
Step 1: Foundation Anchors Check.
Review the established statements against the 3 Foundation Anchors:
- [A1: Core Operational Problem & Desired Outcome]
- [A2: Primary Stakeholders & Agency]
- [A3: Core Operational Invariant or Scope Boundary]

If ANY anchor is indeterminate or missing:
Your mode is ANCHOR BISECTION.
Ask ONE high-entropy bisection question (Option A vs Option B) that resolves the missing foundation (e.g. bisecting the primary actors or clarifying the core operational boundary). Do NOT dive into obscure edge cases yet.

If ALL 3 anchors are established:
Your mode is NEGATIVE-SPACE VOID-FILLING.
Audit the 5 orthogonal domain axes: (1. Agency & Responsibility, 2. Preconditions & Invariants, 3. Exception & Conflict Resolution, 4. Temporal Cadence & Lifecycle, 5. Evidence & Accountability).
Identify the axis with zero or lowest coverage. Ask ONE high-entropy bisection question presenting a concrete fork: "If [Scenario X occurs in the least-covered axis], does the domain rule dictate Option A or Option B?"
Do NOT ask generic boilerplate questions or repeat anything from the Already Explored list."""
    lines.append(directive)
    return "\n".join(lines)


def generate_stakeholder_response(
    mode: str,
    brief: dict[str, Any],
    turn_idx: int,
    facilitator_q: str,
    turns: list[dict[str, str]],
    ollama_url: str,
    persona_model: str,
    commercial_url: str = "",
    commercial_key: str = "",
    conversation_id: str | None = None,
) -> tuple[str, str | None]:
    """Generates the stakeholder response based on the simulation mode."""
    if mode == "scripted":
        script = brief.get("scripted_responses", [])
        if turn_idx < len(script):
            return script[turn_idx], None
        return f"Regarding that question: our policy requires strict verification and sign-off before proceeding.", None

    persona_sys = f"""You are {brief.get('stakeholder_name', 'the stakeholder')} participating in a domain requirements discovery interview.
You hold these factual real-world domain constraints and rules in your head:
{json.dumps(brief.get('ground_truth', {}), indent=2)}

Rules for your answers:
1. Answer the facilitator's specific question directly based strictly on the facts in your brief.
2. Be concise: 1 to 2 sentences (max 40 words).
3. Do NOT invent software technologies, databases, APIs, or servers.
4. Reveal only the relevant fact needed to answer the question. Do not dump the entire brief at once.
5. Output only your answer utterance."""

    msgs = [
        {"role": "system", "content": persona_sys},
        {"role": "user", "content": f"The facilitator asks: \"{facilitator_q}\""},
    ]

    if mode == "agy":
        return query_agy_cli(persona_model, persona_sys, facilitator_q, conversation_id=conversation_id)
    elif mode == "local":
        return query_ollama(ollama_url, persona_model, msgs), None
    elif mode == "commercial":
        return query_commercial_llm(commercial_url, commercial_key, persona_model, msgs), None

    return "Our standard operational rule applies.", None


def run_brief_simulation(
    brief: dict[str, Any],
    max_turns: int,
    mode: str,
    ollama_url: str,
    fac_model: str,
    persona_model: str,
    commercial_url: str = "",
    commercial_key: str = "",
) -> tuple[list[dict[str, str]], dict[str, Any]]:
    """Runs a complete multi-turn dialogue simulation for one brief."""
    turns: list[dict[str, str]] = []
    explored: list[str] = []
    agy_conversation_id: str | None = None

    # Turn 1: Stakeholder seed
    seed = brief["seed_statement"]
    turns.append({"role": "stakeholder", "text": seed})
    print(f"  [t1] ({brief['stakeholder_name']}): {seed}")

    turn_count = 1
    script_idx = 0
    while turn_count < max_turns:
        # Facilitator turn
        prompt = build_facilitator_prompt(brief["title"], turns, explored)
        fac_msgs = [
            {"role": "system", "content": FACILITATOR_SYSTEM_PROMPT_V2},
            {"role": "user", "content": prompt},
        ]

        fac_q = query_ollama(ollama_url, fac_model, fac_msgs)
        # Strip trailing quotes or artifacts
        fac_q = fac_q.strip().strip('"')
        turns.append({"role": "facilitator", "text": fac_q})
        explored.append(fac_q)
        turn_count += 1
        print(f"  [t{turn_count}] (Facilitator): {fac_q}")

        if turn_count >= max_turns:
            break

        # Stakeholder turn
        stakeholder_resp, agy_conversation_id = generate_stakeholder_response(
            mode=mode,
            brief=brief,
            turn_idx=script_idx,
            facilitator_q=fac_q,
            turns=turns,
            ollama_url=ollama_url,
            persona_model=persona_model,
            commercial_url=commercial_url,
            commercial_key=commercial_key,
            conversation_id=agy_conversation_id,
        )
        script_idx += 1
        turns.append({"role": "stakeholder", "text": stakeholder_resp})
        turn_count += 1
        print(f"  [t{turn_count}] ({brief['stakeholder_name']}): {stakeholder_resp}")

    metrics = evaluate_simulation(turns, brief)
    return turns, metrics


def render_transcript_markdown(brief: dict[str, Any], turns: list[dict[str, str]]) -> str:
    """Formats dialogue turns as canonical tks-transcript/v1 markdown."""
    lines = [
        "---",
        "format: tks-transcript/v1",
        f"title: \"{brief['title']}\"",
        f"session_id: \"{brief['slug']}\"",
        "cycle: 1",
        f"date: \"{time.strftime('%Y-%m-%d')}\"",
        "language: \"en\"",
        "source: \"simulation\"",
        "participants:",
        f"  - {{ name: \"{brief['stakeholder_name']}\", role: \"stakeholder\" }}",
        f"  - {{ name: \"Facilitator\", role: \"facilitator\" }}",
        "---\n",
    ]
    for idx, t in enumerate(turns):
        role_label = brief["stakeholder_name"] if t["role"] == "stakeholder" else "Facilitator"
        lines.append(f"**[t{idx+1}] {role_label} ({t['role']}):** {t['text']}\n")
    return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(description="TKS Intent Elicitation Simulation Testbed (EA-5, EB-8)")
    parser.add_argument("--mode", choices=["scripted", "agy", "local", "commercial"], default="agy",
                        help="Simulation mode: 'agy' (via agy CLI, no API keys), 'scripted' (deterministic), 'local' (Ollama persona), 'commercial' (API persona)")
    parser.add_argument("--brief", help="Optional specific brief JSON path (defaults to all in briefs/)")
    parser.add_argument("--turns", type=int, default=8, help="Number of turns to simulate per brief (default: 8)")
    parser.add_argument("--ollama-url", default="", help="Ollama base URL (default: http://ollama:11434 or localhost)")
    parser.add_argument("--fac-model", default="qwen3:8b", help="Model used by the Socratic Facilitator (default: qwen3:8b)")
    parser.add_argument("--persona-model", default="", help="Model used for the stakeholder persona (defaults to gemini-3.8-flash-low for agy, qwen3:8b for local)")
    parser.add_argument("--commercial-url", default=os.getenv("OPENAI_BASE_URL", "https://api.openai.com"), help="Commercial API base URL")
    parser.add_argument("--api-key", default=os.getenv("OPENAI_API_KEY", os.getenv("GEMINI_API_KEY", "")), help="API key for commercial persona")
    parser.add_argument("--out", default=str(DEFAULT_OUT_DIR), help="Output directory for transcripts and report")
    args = parser.parse_args()

    persona_model = args.persona_model
    if not persona_model:
        persona_model = "gemini-3.8-flash-low" if args.mode == "agy" else "qwen3:8b"

    ollama_url = resolve_ollama_host(args.ollama_url)
    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)

    brief_files = [Path(args.brief)] if args.brief else sorted(BRIEFS_DIR.glob("*.json"))
    if not brief_files:
        print(f"Error: No brief JSON files found in {BRIEFS_DIR}")
        sys.exit(1)

    print("=" * 70)
    print("TKS INTENT ELICITATION SIMULATION & BISECTION BENCHMARK")
    print(f"Mode: {args.mode.upper()} | Facilitator Model: {args.fac_model} | Ollama: {ollama_url}")
    print(f"Briefs to run: {len(brief_files)} | Max turns per brief: {args.turns}")
    print("=" * 70)

    all_metrics = []
    for b_path in brief_files:
        with open(b_path, "r", encoding="utf-8") as f:
            brief = json.load(f)

        print(f"\n▶ Running simulation: {brief['id']} - {brief['title']}...")
        turns, metrics = run_brief_simulation(
            brief=brief,
            max_turns=args.turns,
            mode=args.mode,
            ollama_url=ollama_url,
            fac_model=args.fac_model,
            persona_model=persona_model,
            commercial_url=args.commercial_url,
            commercial_key=args.api_key,
        )
        all_metrics.append(metrics)

        # Write transcript
        ts_path = out_dir / f"{brief['slug']}.md"
        with open(ts_path, "w", encoding="utf-8") as f:
            f.write(render_transcript_markdown(brief, turns))
        print(f"  ✓ Transcript exported: {ts_path}")

    # Generate summary report
    report_md = format_markdown_report(all_metrics)
    report_path = out_dir / "simulation_report.md"
    with open(report_path, "w", encoding="utf-8") as f:
        f.write(report_md)

    json_path = out_dir / "simulation_metrics.json"
    with open(json_path, "w", encoding="utf-8") as f:
        json.dump(all_metrics, f, indent=2)

    print("\n" + "=" * 70)
    print("SIMULATION SUMMARY REPORT")
    print("=" * 70)
    print(report_md)
    print(f"\nSaved Markdown report to: {report_path}")
    print(f"Saved JSON metrics to: {json_path}")


if __name__ == "__main__":
    main()
