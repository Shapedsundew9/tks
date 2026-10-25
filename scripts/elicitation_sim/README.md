# Intent Elicitation Simulation & Bisection Testbed

> **Purpose:** Automated closed-loop simulation harness for evaluating and tuning the TKS Socratic Facilitator and Inspiration Assistant against real-world domain briefs (WP EA-5, EB-8).

---

## Architecture

The simulation runner (`runner.py`) pits the **TKS Socratic Facilitator** (Ollama `qwen3:8b`) against a **Simulated Stakeholder** in a multi-turn conversation over a hidden domain brief:

1. **Hidden Domain Brief (`briefs/*.json`):** Holds ground-truth facts about actors, invariants, exceptions, temporal cadence, and accountability rules.
2. **Stakeholder Persona:** Emulated in one of four modes:
   * `agy` (**Default / Zero-Config Frontier Model**): Uses the pre-authenticated `agy` CLI to drive frontier models (e.g. `gemini-3.8-flash-low`, `gemini-3.8-flash-high`, `claude-sonnet-5-5-high`) directly from the terminal without requiring public API keys or credits.
   * `scripted`: Deterministic answers drawn directly from the brief (ideal for fast CI/CD and regression testing).
   * `local`: Local Ollama model (e.g. `qwen3:8b` or `gemma4:e4b`) prompted with the persona and brief.
   * `commercial`: A commercial high-reasoning LLM (e.g. Claude 3.5 Sonnet, GPT-4o, Gemini 1.5 Pro) via standard OpenAI-compatible HTTP API.
3. **Automated Evaluator (`metrics.py`):** Scores:
   * **Foundation Anchors Resolution:** Exact turn where Problem/Outcome (A1), Actors (A2), and Core Invariant (A3) are all resolved.
   * **5-Axis Coverage:** Proportion of the 5 universal domain axes explored (`Agency`, `Preconditions`, `Exceptions`, `Cadence`, `Accountability`).
   * **Bisection Fork Rate:** Percentage of facilitator questions that formulate a high-entropy binary bisection.
   * **Neutrality Compliance:** Deterministic check against solution presuppositions and the technology mechanism lexicon.

---

## Quickstart

### 1. Run with `agy` CLI (Recommended — Zero external API keys needed)
Uses the local `agy` CLI to power the stakeholder persona with high-reasoning models:

```bash
# Default (uses gemini-3.8-flash-low for fast, intelligent persona responses)
python3 scripts/elicitation_sim/runner.py --mode agy --turns 6

# Use a specific frontier model for the stakeholder persona:
python3 scripts/elicitation_sim/runner.py --mode agy --persona-model claude-sonnet-5-5-low --turns 6
```

### 2. Run Scripted Baseline (Fast, deterministic)
Runs through all briefs in `briefs/` using deterministic persona responses:

```bash
python3 scripts/elicitation_sim/runner.py --mode scripted --turns 6
```

### 3. Run with Local Ollama Persona
Uses local Ollama for both the Facilitator and Stakeholder:

```bash
python3 scripts/elicitation_sim/runner.py --mode local --persona-model qwen3:8b --turns 6
```

### 4. Run with Commercial HTTP API Persona
Benchmark against a direct external API:

```bash
export OPENAI_API_KEY="sk-..."
python3 scripts/elicitation_sim/runner.py \
  --mode commercial \
  --persona-model gpt-4o \
  --turns 8
```

Or with Gemini via Google AI Studio's OpenAI-compatible endpoint:

```bash
export OPENAI_BASE_URL="https://generativelanguage.googleapis.com/v1beta/openai"
export OPENAI_API_KEY="AIza..."
python3 scripts/elicitation_sim/runner.py \
  --mode commercial \
  --persona-model gemini-1.5-pro \
  --turns 8
```

---

## Outputs & Reports

The runner exports into `scripts/elicitation_sim/results/`:
* `*.md`: Canonical `tks-transcript/v1` Markdown transcripts for each simulated brief.
* `simulation_report.md`: Markdown summary table and per-axis breakdown.
* `simulation_metrics.json`: Machine-readable metrics.
