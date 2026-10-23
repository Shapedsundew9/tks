# Chapter 11: Intent Elicitation & Goal Staging

Intent Elicitation is the intake path for conversational transcripts and free-form vision documents
that are too early or ambiguous to decompose directly into requirements. TKS parses and verifies
agent submissions; the connected MCP agent performs episode segmentation, goal abstraction, and
judgment. TKS does not generate intent or call an LLM in `tks serve`.

This chapter covers E-B, Goal Extraction & Gap Analysis. E-A conversation capture is a separate,
backlogged phase. For transcript syntax, see the normative
[tks-transcript/v1 specification](../vision/elicitation-transcript-format.md).

## Before You Start

1. Start the TKS daemon with its PostgreSQL database available.
2. Provision an agent identity and connect an MCP-capable agent as described in
   [Chapter 9](09-mcp-agent-integration.md).
3. Prepare either:
   * A Markdown transcript using `tks-transcript/v1`, with contiguous turn IDs and declared
     participants; or
   * A free-form Markdown vision document.

The source must be readable by the TKS server process if you pass only `doc_path`. Otherwise, pass
the document text with the optional `content` argument. Transcripts must follow the
[transcript-format rules](../vision/elicitation-transcript-format.md); vision documents do not need
frontmatter.

## E-B Extraction Flow

Use the MCP tools in this order. `tools/list` publishes the input schemas; the examples below show
the key fields and omit response wrappers.

### 1. Open an extraction session

Call `open_extraction_session` with a path and, optionally, inline content:

```json
{
  "doc_path": "docs/vision/vision.md"
}
```

For content not available on the server filesystem:

```json
{
  "doc_path": "product/vision.md",
  "content": "# Product vision\n\nPeople need to coordinate..."
}
```

The response includes a `session_id`, source hash and kind, and parsed turns or document chunks.
Keep the `session_id`; subsequent tools require it. For a cycle-2 transcript, supply its
`prior_session_id` as the previous extraction session ID so TKS can reconcile candidate goals across
cycles. The cycle number comes from the transcript frontmatter.

### 2. Segment the source into episodes

Ask the agent to use `elicitation/episode-segmentation/v1` with the transcript text or the parsed
turns/chunks. Submit the resulting complete, contiguous episode ranges with `submit_episodes`:

```json
{
  "session_id": "<SESSION_UUID>",
  "episodes": [
    {
      "id": "E1",
      "turns": ["t1", "t5"],
      "topic": "Coordination and availability"
    }
  ]
}
```

Use the turn/chunk IDs returned by `open_extraction_session`; vision-document chunks use `cN`
aliases. TKS checks that the ranges are ordered, contiguous, non-overlapping, and cover the source.
For transcripts, the agent should also resolve conversational references to verbatim referent quotes.

### 3. Submit grounded goal candidates

Ask the agent to use `elicitation/goal-extraction/v1` for each episode, then call
`submit_goal_candidates`. A candidate includes a technology-agnostic statement, kind, stakeholder,
status, and at least one source quote. Transcript grounding uses `turn`; document grounding uses
`section`.

```json
{
  "session_id": "<SESSION_UUID>",
  "goals": [
    {
      "id": "G1",
      "kind": "ACHIEVE",
      "statement": "Coordinators can assess current team availability without interrupting staff.",
      "stakeholder": "Priya",
      "status": "CONFIRMED",
      "episode": "E1",
      "grounding": [
        {
          "turn": "t2",
          "quote": "I spend half my Monday chasing tech leads"
        }
      ],
      "acceptable_paraphrases": [],
      "forbidden_concretizations": ["dashboard", "Slack integration"],
      "links": []
    }
  ]
}
```

Allowed kinds are `ACHIEVE`, `MAINTAIN`, `AVOID`, and `SOFTGOAL`; allowed statuses are `CONFIRMED`,
`TENTATIVE`, and `RETRACTED`. TKS mechanically checks each quote against the cited source and
returns an error for ungrounded candidates. It also flags technology terms in a goal statement when
they are absent from its grounding quotes. Treat analogies and UI metaphors as evidence, not as
requirements. Only confirmed goals are eligible for staging; tentative and retracted candidates
remain useful for review and analysis.

### 4. Submit entailment-verification verdicts

The agent can use `elicitation/entailment-judge/v1` in a fresh reasoning context, or a configured
verification workflow outside `tks serve`. Submit the verdicts with `submit_verification_verdicts`:

```json
{
  "session_id": "<SESSION_UUID>",
  "verdicts": [
    {
      "goal_id": "G1",
      "forward": "ENTAILED",
      "reverse_overconstrained": false,
      "notes": "The statement preserves the stakeholder's stated outcome."
    }
  ]
}
```

`forward` is `ENTAILED`, `NEUTRAL`, or `CONTRADICTED`. A neutral verdict indicates the premise does
not entail the goal; `reverse_overconstrained` flags a goal that is narrower than its evidence.
These judgments are supplied by the agent or an explicitly configured verifier, not generated by
the TKS server.

### 5. Review gaps and formulate follow-up questions

Call `get_gap_report` to run the deterministic structural gap detectors:

```json
{
  "session_id": "<SESSION_UUID>"
}
```

The report identifies open gaps such as goals without stakeholders, soft goals without contributors,
unresolved conflicts, or missing boundary statements. Use
`elicitation/gap-questions/v1` to phrase a neutral, grounded question for each gap. The report does
not invent stakeholder intent; questions are prompts for a later elicitation cycle.

For a follow-up cycle, create a `tks-transcript/v1` transcript whose `cycle` is incremented, then
open it with the prior session linked as described above. Re-run the extraction passes. TKS
reconciles unchanged goals to preserve their UUIDs and reports gaps that the new cycle resolves.

### 6. Stage goals for human review

Call `stage_goals` only after the candidates and verification pass have been reviewed:

```json
{
  "session_id": "<SESSION_UUID>"
}
```

An optional `target_workspace_id` stages the goal drafts in an isolated workspace. TKS creates
`GOAL` and `SOURCE_EXCERPT` nodes in `DRAFT` state, with `GROUNDED_IN` edges and any valid candidate
links. Staging is not approval: a human must inspect and approve the drafts before they become
active graph nodes.

## Review, Approve, or Reject

The extraction session ID is also the staging job ID. Use the CLI for narrative-order review:

```bash
# List staged goals, grounding, verdicts, and lint warnings
tks staging list <SESSION_UUID>

# Inspect an individual goal by UUID or canonical node key
tks staging inspect <NODE_ID>

# Approve all candidates, or select/exclude nodes
tks staging approve <SESSION_UUID>
tks staging approve <SESSION_UUID> --only <NODE_ID,...>
tks staging approve <SESSION_UUID> --exclude <NODE_ID,...>

# Reject the whole session or a single candidate
tks staging reject --job-id <SESSION_UUID> --reason "Needs stakeholder review"
tks staging reject <NODE_ID> --reason "Not supported by source"
```

Inspect the verbatim grounding, goal kind, stakeholder, entailment verdict, and technology-lint
warnings before approval. Use `--confirm-omissions` when intentionally confirming that omitted
active nodes should be deprecated. See [Chapter 10](10-cli-reference.md) for the complete staging
command syntax.

## Safety and Scope

* Every staged goal must have at least one mechanically verified verbatim grounding quote.
* TKS rejects unsupported quotes and warns when a non-retracted goal is grounded only in
  facilitator or agent turns. Reviewers should require stakeholder evidence or explicit
  stakeholder confirmation before approval.
* A candidate's technology or interface metaphor must not be turned into a required mechanism unless
  the stakeholder explicitly grounded that mechanism.
* `stage_goals` creates drafts only. Human staging review remains the approval gate.
* Elicitation cognition is external to TKS. The server provides protocol, prompts, mechanical
  verification, storage, and review surfaces.
