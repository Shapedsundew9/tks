# Chapter 9: Model Context Protocol (MCP) Integration

This chapter covers connecting an MCP-capable agent to the Knowledge Substrate (TKS), including
the tools and versioned prompts used by the Intent Elicitation workflow.

## What is MCP in TKS?

The Model Context Protocol (MCP) lets compatible agent clients discover and invoke tools and
prompts exposed by a server. TKS exposes graph-query and mutation tools as well as a structured
protocol for extracting grounded goals from transcripts and vision documents.

TKS supplies deterministic parsing, validation, storage, and review. The connected agent supplies
language-model reasoning; the `tks serve` path does not call an LLM.

## Transport Modes

TKS provides two MCP transport mechanisms:

1. **Stdio proxy (`tks mcp-stdio`):** For local agent clients. The proxy streams JSON-RPC over
   standard input/output and communicates with `tks serve` over HTTP.
2. **HTTP/SSE (`/mcp` and `/mcp/sse`):** For remote or containerized agent clients.

## Configuring Agent Environments

The daemon must be running, and the client needs an identity token accepted by the daemon.
Provision an identity with `tks identity create` and provide the resulting token through the
client's supported secret/environment mechanism. Do not commit a real token to a project config.

### Claude Code configuration

Add TKS to `~/.claude.json` or the project-level `.mcp.json`, substituting the local binary path and
token:

```json
{
  "mcpServers": {
    "tks": {
      "command": "/path/to/tks",
      "args": ["mcp-stdio"],
      "env": {
        "TKS_SERVER_URL": "http://127.0.0.1:8080",
        "TKS_AUTH_TOKEN": "replace-with-your-identity-token"
      }
    }
  }
}
```

### Cursor configuration

In Cursor's MCP settings, add a command server:

* **Name:** `tks`
* **Command:** `cargo run --bin tks -- mcp-stdio`
* **Environment:** `TKS_SERVER_URL=http://127.0.0.1:8080` and
  `TKS_AUTH_TOKEN=<your-identity-token>`

Other MCP clients can use either transport supported by their runtime.

## MCP Tools

TKS exposes the following 14 tools. Clients can discover the authoritative argument schemas with
MCP `tools/list`; each tool definition uses the standard `inputSchema` field.

| Tool | Purpose |
| --- | --- |
| `get_context_envelope` | Retrieve bounded graph context around a node. |
| `query_requirements` | Search active requirements. |
| `get_document_span` | Retrieve a node's verbatim source span. |
| `propose_node_mutation` | Submit a governed candidate mutation. |
| `create_subtask` | Create an execution task under a governing node. |
| `update_node_status` | Update a node's execution status. |
| `revert_mutations` | Preview or execute a compensating rollback. |
| `reverify_node` | Reverify a node in `NEEDS_REVERIFICATION`. |
| `open_extraction_session` | Parse a transcript or vision document and initialize an E-B session. |
| `submit_episodes` | Submit contiguous topic episodes covering all transcript turns. |
| `submit_goal_candidates` | Submit goals for grounding verification and technology-term lint. |
| `submit_verification_verdicts` | Store entailment and reverse-overconstraint verdicts. |
| `get_gap_report` | Run deterministic gap detectors and report open gaps. |
| `stage_goals` | Stage confirmed, grounded goals and source excerpts for review. |

The last six tools implement the E-B extraction protocol. For their end-to-end sequence, payload
examples, and review steps, see [Chapter 11: Intent Elicitation & Goal Staging](11-intent-elicitation.md).

## Versioned E-B Prompts

The MCP `prompts/list` and `prompts/get` capabilities provide canonical pass instructions:

| Prompt | Purpose |
| --- | --- |
| `elicitation/episode-segmentation/v1` | Segment turns into contiguous episodes and resolve coreferences. |
| `elicitation/goal-extraction/v1` | Extract technology-agnostic goals with verbatim grounding. |
| `elicitation/entailment-judge/v1` | Judge entailment and detect over-constraint. |
| `elicitation/gap-questions/v1` | Formulate neutral questions grounded in detected gaps. |

Prompts guide the external agent; they do not cause TKS to invoke an LLM.
