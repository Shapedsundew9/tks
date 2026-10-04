# Chapter 9: Model Context Protocol (MCP) Integration

This chapter covers integrating the Knowledge Substrate with autonomous AI coding agents such as Claude Code, Cursor, Windsurf, and custom agent harnesses using the **Model Context Protocol (MCP)**.

---

## What is MCP in TKS?

The **Model Context Protocol (MCP)** is an open standard that allows Large Language Models (LLMs) to securely access external tools and data stores.

Instead of writing custom prompts or hard-coding API integrations, any MCP-compliant agent harness can connect directly to TKS to discover requirements, query bounded context envelopes, elaborate execution tasks, and submit mutations.

---

## Transport Modes

TKS provides two MCP transport mechanisms:

1. **Stdio Streaming Proxy (`tks mcp-stdio`):** Ideal for local developer tools and command-line agent harnesses (Claude Code, Cursor, Windsurf). The subcommand acts as a lightweight proxy, streaming JSON-RPC over `stdin`/`stdout` while communicating with `tks serve` over HTTP.
2. **HTTP / SSE Transport (`/mcp` and `/mcp/sse`):** Ideal for remote agent runners, multi-tenant clouds, or containerized agents connecting over the network.

---

## Configuring Agent Environments

### 1. Claude Code Configuration

To configure Claude Code, add TKS to your `~/.claude.json` or project-level `.mcp.json`:

```json
{
  "mcpServers": {
    "tks": {
      "command": "/workspaces/tks/target/release/tks",
      "args": ["mcp-stdio"],
      "env": {
        "TKS_SERVER_URL": "http://127.0.0.1:8080",
        "TKS_AUTH_TOKEN": "tks_sec_38fe912a77bc40..."
      }
    }
  }
}
```

### 2. Cursor IDE Configuration

In Cursor Settings $\to$ Features $\to$ MCP, click **+ Add New MCP Server**:

* **Name:** `tks`
* **Type:** `command`
* **Command:** `cargo run --bin tks -- mcp-stdio`
* **Environment Variables:**
  * `TKS_SERVER_URL=http://127.0.0.1:8080`
  * `TKS_AUTH_TOKEN=tks_sec_38fe912a77bc40...`

---

## MCP Tools Reference

TKS exposes a complete suite of MCP tools for autonomous coding workflows:

### 1. `query_requirements`

Performs high-speed keyword search across active requirements.

* **Arguments:**
  * `query` (string, required): Search query keywords (e.g. `"ancestry verification"`).
  * `limit` (integer, optional): Maximum results to return (default: `5`).

### 2. `get_context_envelope`

Assembles a bounded context envelope containing upward ancestors, sibling constraints, and semantic vector neighbors.

* **Arguments:**
  * `target_node_id` (UUID, required): The target requirement, specification, or task ID.
  * `depth` (integer, optional): Maximum traversal depth (clamped $\le 3$, default: `3`).

### 3. `get_document_span`

Slices and returns the verbatim source Markdown text from the bare Git ODB using cryptographic byte coordinates.

* **Arguments:**
  * `node_id` (UUID, required): Target requirement node ID.

### 4. `elaborate_task`

Autonomously creates an execution subtask under a governing requirement.

* **Arguments:**
  * `parent_id` (UUID, required): Active governing requirement or specification ID.
  * `title` (string, required): Short descriptive title of the task.
  * `description` (string, optional): Detailed technical implementation specification.
  * `auth_token` (string, optional if Bearer header is set): Caller credentials.

### 5. `update_task_state`

Updates the execution status of an active task using native row-level locks.

* **Arguments:**
  * `task_id` (UUID, required): Task node ID.
  * `status` (string, required): One of `"OPEN"`, `"IN_PROGRESS"`, `"COMPLETED"`, `"BLOCKED"`.

### 6. `inspect_node`

Inspects detailed attributes, draft revisions, and parent relationships for a node.

* **Arguments:**
  * `node_id` (UUID, required): Node ID to inspect.

### 7. `revert_mutations`

Administrative tool to preview or execute compensating rollbacks.

* **Arguments:**
  * `batch_id` (UUID, optional): Specific mutation batch to roll back.
  * `dry_run` (boolean, optional): If `true`, returns blast radius preview without modifying data.
  * `force` (boolean, optional): If `true`, overrides cross-agent safety aborts.

### 8. `reverify_node`

Restores a node in `NEEDS_REVERIFICATION` state back to `ACTIVE`.

* **Arguments:**
  * `node_id` (UUID, required): Degraded node ID.
  * `reparent_to` (UUID, optional): New parent requirement ID if parent was superseded.
  * `rationale` (string, optional): Justification recorded in the audit ledger.
