# The Knowledge Substrate (TKS) User's Guide

Welcome to the **Knowledge Substrate (TKS)** User's Guide. This guide provides an end-to-end operational walkthrough of TKS, demonstrating how to transform passive software architecture documents into an active, verified, causal property graph.

Throughout this guide, we use the real governing assets of the TKS repository—specifically [`docs/vision/vision.md`](file:///workspaces/tks/docs/vision/vision.md), [`docs/vision/architecture.md`](file:///workspaces/tks/docs/vision/architecture.md), and [`docs/vision/technical-backlog.md`](file:///workspaces/tks/docs/vision/technical-backlog.md)—as our continuous running example.

---

## Table of Contents

1. [Mental Model & Core Architecture](file:///workspaces/tks/docs/user-guide/01-mental-model-and-architecture.md)
   * The problem with static Markdown documentation
   * The causal property graph: Nodes, typed upward edges, and source spans
   * Core invariants (`INV-1` through `INV-7`)
   * Dual mutation pathways: Autonomous task elaboration vs. normative proposals

2. [Daemon & Identity Provisioning](file:///workspaces/tks/docs/user-guide/02-daemon-and-identities.md)
   * Starting the unified daemon: `tks serve`
   * Bare Git repository storage and PostgreSQL `pgvector` configuration
   * Provisioning and revoking agent identities: `tks identity`
   * Authentication and cache eviction mechanics

3. [Ingesting Governing Assets](file:///workspaces/tks/docs/user-guide/03-ingesting-governing-documents.md)
   * Submitting `docs/vision/vision.md` via `POST /api/v1/documents/ingest`
   * Deterministic CommonMark AST parsing (`pulldown-cmark`) and heading anchor extraction
   * Candidate draft staging review: `tks staging list` and `tks staging inspect`
   * Staging approval, atomic draft promotion, and local vector generation

4. [Context Envelopes & Requirement Retrieval](file:///workspaces/tks/docs/user-guide/04-context-envelopes-and-querying.md)
   * Fast full-text search: `query_requirements` / `POST /api/v1/requirements/query`
   * Bounded context envelope assembly: $k \le 3$ hop recursive CTE traversal
   * Quota partitioning: 30-node topological budget + 10-node vector neighbor allocation
   * Verbatim source span extraction from Git ODB without database bloat

5. [Task Elaboration & Autonomous Execution](file:///workspaces/tks/docs/user-guide/05-task-elaboration-and-execution.md)
   * Decomposing requirements into tasks: `tks task create`
   * Governance policy inheritance (`AUTONOMOUS_ELABORATION`)
   * Safe leaf node updates under native row locks (`FOR UPDATE`): `tks task update`
   * Audit ledger event sequence tracking (`event_seq`, `batch_id`)

6. [Multi-Agent Workspaces & Collaborative Co-Evolution](file:///workspaces/tks/docs/user-guide/06-workspaces-and-multi-agent-coevolution.md)
   * Ephemeral branch workspace containers: `tks workspace create`
   * Advisory-lock-free task elaboration and caller draft confidentiality (`INV-7`)
   * Dynamic draft query overlays combining live active state with workspace drafts
   * Three-way topological merges (`tks workspace merge`) and auto-reparenting lineage resolution

7. [Document Evolution, Invalidation Storms & Reverification](file:///workspaces/tks/docs/user-guide/07-document-evolution-and-invalidation.md)
   * Updating governing documents in Git and re-ingesting revisions
   * In-place span re-anchoring across Git commit boundaries
   * Downward invalidation cascade engine: Shortest-path staleness propagation (`MIN(depth)`)
   * Operational unblocking: `tks admin reverify` and administrative rollback (`tks admin revert`)

8. [Web Explorer & Real-Time Observability](file:///workspaces/tks/docs/user-guide/08-web-explorer-and-observability.md)
   * Serving the zero-CDN embedded Cytoscape.js/Dagre visualizer: `tks explorer serve`
   * Live Server-Sent Events (SSE) telemetry stream: `GET /api/v1/explorer/events`
   * Sub-millisecond event demultiplexing via in-process `GraphEventBus`
   * Interactive branch exploration and live invalidation storm visualization

9. [Model Context Protocol (MCP) Integration for AI Agents](file:///workspaces/tks/docs/user-guide/09-mcp-agent-integration.md)
   * Connecting Claude Code, Cursor, Windsurf, and autonomous agent harnesses
   * Local stdio proxy mode: `tks mcp-stdio`
   * Remote HTTP/SSE endpoint: `POST /mcp` and `GET /mcp/sse`
   * Complete reference of exposed MCP tools and argument schemas

10. [CLI Reference & Quick Cheat Sheet](file:///workspaces/tks/docs/user-guide/10-cli-reference.md)
    * Comprehensive syntax and flag reference for all `tks` subcommands
    * Common administrative recipes and troubleshooting patterns

---

## 5-Minute Quickstart

Here is how you can spin up TKS, ingest the governing specification of this project, and query an architectural context envelope in under 5 minutes:

### 1. Build and Boot the Daemon

Ensure Docker Compose is running PostgreSQL and the Ollama embedding service (or local FastEmbed CPU provider). Then launch the server:

```bash
# Build release or debug binary
cargo build

# Run embedded migrations against $DATABASE_URL and start daemon on port 8080
cargo run --bin tks -- serve
```

### 2. Provision an Identity

In a separate terminal, provision an agent identity for autonomous coding sessions:

```bash
# Create an agent identity
cargo run --bin tks -- identity create \
  --name "gemini-developer" \
  --role "AGENT" \
  --description "Autonomous engineering agent"
```

The output provides your bearer authentication token:

```text
Identity created:
  ID:    c24e9c71-3cb5-4702-8610-d8d17a3f4e22
  Name:  gemini-developer
  Role:  AGENT
  Token: tks_sec_9f81a7b...
```

Export this token to your shell environment:

```bash
export TKS_AUTH_TOKEN="tks_sec_9f81a7b..."
export TKS_SERVER_URL="http://127.0.0.1:8080"
```

### 3. Ingest `docs/vision/vision.md`

Submit the project's strategic vision document to the ingestion gateway:

```bash
curl -s -X POST "$TKS_SERVER_URL/api/v1/documents/ingest" \
  -H "Authorization: Bearer $TKS_AUTH_TOKEN" \
  -H "Content-Type: application/json" \
  -d "{
    \"doc_path\": \"specs/vision.md\",
    \"content\": $(jq -Rs . < docs/vision/vision.md)
  }"
```

The server commits `docs/vision/vision.md` directly into the bare Git repository at `.substrate/git/tks.git` on branch `refs/heads/specs`, queues the job, and the background worker decomposes it into candidate requirements:

```json
{"job_id":"8fb41f11-76a0-48e2-9b24-77a83d739811","status":"QUEUED"}
```

### 4. Review and Approve Drafts

Inspect the candidate requirement drafts generated by the mechanical Markdown AST parser:

```bash
# List candidate nodes staged for review
cargo run --bin tks -- staging list 8fb41f11-76a0-48e2-9b24-77a83d739811

# Inspect a specific node and view its verbatim text slice from Git ODB
cargo run --bin tks -- staging inspect 8fb41f11-76a0-48e2-9b24-77a83d739811 <NODE_ID>

# Approve and promote all drafts to ACTIVE state
cargo run --bin tks -- staging approve 8fb41f11-76a0-48e2-9b24-77a83d739811
```

Upon approval, candidate drafts are promoted to `ACTIVE`, initial immutable events are written to `audit_ledger`, and 384-dimensional dense vectors are generated asynchronously into `node_embeddings`.

### 5. Retrieve a Bounded Context Envelope

Now an agent can query the governing graph for architectural context:

```bash
# Query requirements matching "invariant"
curl -s -X POST "$TKS_SERVER_URL/api/v1/requirements/query" \
  -H "Authorization: Bearer $TKS_AUTH_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"query": "invariant single engine", "limit": 5}'
```

```bash
# Retrieve the graph-bounded context envelope for a requirement
curl -s -X POST "$TKS_SERVER_URL/api/v1/context/envelope" \
  -H "Authorization: Bearer $TKS_AUTH_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"requirement_id": "<REQUIREMENT_UUID>", "depth": 3}'
```

The resulting envelope returns all upstream governing invariants, sibling constraints, and topologically ranked dependencies in $<50\text{ ms}$, bounded strictly within token limits.

### 6. View the Live Web Explorer

Open your browser to:

```text
http://localhost:8080/explorer
```

You will see an interactive Cytoscape.js directed acyclic graph rendered entirely from self-contained assets with zero external CDN dependencies, connected to a live SSE stream showing real-time updates.
