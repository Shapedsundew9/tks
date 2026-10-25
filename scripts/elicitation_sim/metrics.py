#!/usr/bin/env python3
"""Evaluation and convergence metrics for Intent Elicitation simulations (WP EB-8, EA-5).

Measures:
1. Turns to Foundation Anchor Resolution (A1: Outcome, A2: Actors, A3: Invariant)
2. Coverage across the 5 Orthogonal Domain Axes
3. Neutrality & Technology Presupposition Compliance
4. Bisection Fork Rate (Entropy reduction)
"""

from __future__ import annotations

import json
import re
from typing import Any

# Technology lexicon aligned with src/ingest/elicitation.rs
TECHNOLOGY_LEXICON = {
    "api", "apis", "app", "application", "aws", "backend", "cloud",
    "dashboard", "database", "db", "docker", "endpoint", "frontend",
    "http", "kafka", "kubernetes", "k8s", "microservice", "microservices",
    "postgres", "queue", "redis", "rest", "saas", "server", "servers",
    "sql", "ui", "websocket", "websockets", "webhook", "webhooks"
}

PRESUPPOSITION_PATTERNS = [
    re.compile(r"\b(?:system|tool|app|application|platform|software)\s+(?:you(?:'re|\s+are)?\s+planning\s+to\s+build|you\s+want\s+to\s+build|to\s+be\s+built)\b", re.I),
    re.compile(r"\bthe\s+new\s+(?:tool|system|app|software|platform|solution)\b", re.I),
    re.compile(r"\b(?:system|tool|platform|application|solution)\s+you\s+(?:want|need)\b", re.I),
    re.compile(r"\b(?:what\s+features|what\s+functionality)\s+(?:should|will)\s+the\s+(?:system|tool|app|software)\b", re.I),
]

AXIS_KEYWORDS = {
    "agency": [
        "who", "actor", "role", "coordinator", "fixer", "nurse", "doctor",
        "driver", "manager", "decide", "authority", "supervisor", "responsibility",
        "delegate", "unilateral", "sign-off", "staff", "volunteer"
    ],
    "preconditions": [
        "must", "require", "prerequisite", "condition", "safety", "rule",
        "strictly", "non-negotiable", "eligible", "limit", "insurance",
        "threshold", "standard", "regulatory", "baseline"
    ],
    "exceptions": [
        "if", "when", "fail", "delay", "disagree", "dispute", "excursion",
        "unclaimed", "loss", "missing", "conflict", "emergency", "quarantine",
        "override", "fallback", "drop", "bottleneck"
    ],
    "cadence": [
        "minute", "hour", "weekend", "week", "schedule", "deadline",
        "saturday", "shift", "transit", "timeline", "temporal", "immediate",
        "duration", "window", "period", "reversal", "final"
    ],
    "accountability": [
        "record", "log", "ticket", "sign", "audit", "proof", "report",
        "telemetry", "seal", "track", "serial", "document", "justify",
        "countersign", "evidence", "ownership"
    ],
}


def check_neutrality(text: str) -> list[str]:
    """Returns list of neutrality violation descriptions."""
    violations = []
    # Check presuppositions
    for pat in PRESUPPOSITION_PATTERNS:
        matches = pat.findall(text)
        for m in matches:
            violations.append(f"Presupposition: '{m}'")

    # Check technology terms
    words = re.findall(r"\b[A-Za-z0-9_]+\b", text.lower())
    for w in words:
        if w in TECHNOLOGY_LEXICON:
            violations.append(f"Technology mechanism: '{w}'")

    return violations


def is_bisection_question(text: str) -> bool:
    """Checks if a facilitator question presents a binary fork or category bisection."""
    lower = text.lower()
    has_fork = (
        " or " in lower
        or "whether " in lower
        or "does " in lower and " or " in lower
        or "should " in lower and " or " in lower
        or "option a" in lower
    )
    has_bracket_tag = bool(re.match(r"^\[[^\]]+\]", text.strip()))
    return has_fork or has_bracket_tag


def evaluate_simulation(turns: list[dict[str, str]], brief: dict[str, Any]) -> dict[str, Any]:
    """Evaluates a multi-turn simulation dialogue against domain criteria."""
    total_turns = len(turns)
    facilitator_turns = [t for t in turns if t.get("role") == "facilitator"]
    stakeholder_turns = [t for t in turns if t.get("role") == "stakeholder"]

    # 1. Foundation Anchors Check
    anchor_a1_turn = None  # Problem & Outcome
    anchor_a2_turn = None  # Primary Actors
    anchor_a3_turn = None  # Core Invariant / Scope

    accumulated_text = ""
    for idx, t in enumerate(turns):
        accumulated_text += " " + t.get("text", "")
        acc_lower = accumulated_text.lower()

        if anchor_a1_turn is None and any(w in acc_lower for w in ["problem", "outcome", "struggle", "losing", "need", "loss", "goal"]):
            anchor_a1_turn = idx + 1

        if anchor_a2_turn is None and any(w in acc_lower for w in ["ana", "volunteer", "fixer", "nurse", "doctor", "driver", "manager", "staff"]):
            anchor_a2_turn = idx + 1

        if anchor_a3_turn is None and any(w in acc_lower for w in ["must", "safety", "rule", "insurance", "accreditation", "celsius", "emergency"]):
            anchor_a3_turn = idx + 1

    macro_resolution_turn = None
    if anchor_a1_turn and anchor_a2_turn and anchor_a3_turn:
        macro_resolution_turn = max(anchor_a1_turn, anchor_a2_turn, anchor_a3_turn)

    # 2. 5-Axis Coverage
    axis_coverage = {}
    full_transcript_lower = " ".join(t.get("text", "") for t in turns).lower()
    for axis, kws in AXIS_KEYWORDS.items():
        matched = [w for w in kws if w in full_transcript_lower]
        axis_coverage[axis] = {
            "covered": len(matched) >= 2,
            "matched_terms": matched[:5],
        }

    covered_axes_count = sum(1 for a in axis_coverage.values() if a["covered"])
    axis_coverage_pct = round((covered_axes_count / len(AXIS_KEYWORDS)) * 100.0, 1)

    # 3. Neutrality & Bisection in Facilitator Turns
    all_violations = []
    bisection_count = 0
    for f_turn in facilitator_turns:
        f_text = f_turn.get("text", "")
        v = check_neutrality(f_text)
        if v:
            all_violations.extend(v)
        if is_bisection_question(f_text):
            bisection_count += 1

    bisection_rate = round(bisection_count / max(1, len(facilitator_turns)), 3)
    neutrality_score = round(max(0.0, 1.0 - (len(all_violations) * 0.1)), 2)

    return {
        "brief_id": brief.get("id"),
        "brief_title": brief.get("title"),
        "total_turns": total_turns,
        "facilitator_turns": len(facilitator_turns),
        "stakeholder_turns": len(stakeholder_turns),
        "foundation_anchors": {
            "a1_outcome_turn": anchor_a1_turn,
            "a2_actors_turn": anchor_a2_turn,
            "a3_invariant_turn": anchor_a3_turn,
            "macro_resolution_turn": macro_resolution_turn,
        },
        "axis_coverage": axis_coverage,
        "covered_axes_count": covered_axes_count,
        "axis_coverage_pct": axis_coverage_pct,
        "bisection_rate": bisection_rate,
        "neutrality_score": neutrality_score,
        "neutrality_violations": all_violations,
    }


def format_markdown_report(results: list[dict[str, Any]]) -> str:
    """Formats simulation metrics into a Markdown report."""
    md = []
    md.append("# Intent Elicitation Simulation & Bisection Benchmark\n")
    md.append("| Brief ID | Title | Turns | Macro Resolved Turn | Axis Coverage | Bisection Rate | Neutrality Score | Status |")
    md.append("| :--- | :--- | :---: | :---: | :---: | :---: | :---: | :---: |")

    for r in results:
        macro = r["foundation_anchors"]["macro_resolution_turn"] or "Unresolved"
        cov = f"{r['covered_axes_count']}/5 ({r['axis_coverage_pct']}%)"
        bis = f"{int(r['bisection_rate'] * 100)}%"
        neu = f"{r['neutrality_score'] * 100:.0f}%"
        status = "PASSED" if r["axis_coverage_pct"] >= 80.0 and r["neutrality_score"] >= 0.9 else "FLAGGED"
        md.append(f"| **{r['brief_id']}** | {r['brief_title']} | {r['total_turns']} | Turn {macro} | {cov} | {bis} | {neu} | **{status}** |")

    md.append("\n### Detailed Axis Breakdown\n")
    for r in results:
        md.append(f"#### {r['brief_id']}: {r['brief_title']}")
        md.append(f"- **Macro Envelope Resolution:** Turn {r['foundation_anchors']['macro_resolution_turn']}")
        for axis, info in r["axis_coverage"].items():
            icon = "✓" if info["covered"] else "✗"
            md.append(f"  - `{axis}`: {icon} (anchors: {', '.join(info['matched_terms']) or 'none'})")
        if r["neutrality_violations"]:
            md.append(f"- **Neutrality Warnings:** {len(r['neutrality_violations'])}")
            for v in r["neutrality_violations"]:
                md.append(f"  - ⚠️ {v}")
        else:
            md.append("- **Neutrality:** Clean (0 violations)")
        md.append("")

    return "\n".join(md)
