# Chapter 10: CLI Reference & Command Cheat Sheet

This chapter provides a complete command reference for all `tks` subcommands, arguments, and common operational recipes.

---

## Global Options

The following flags apply to all commands or can be set via environment variables:

| Flag | Environment Variable | Default | Description |
| :--- | :--- | :--- | :--- |
| `--server-url <URL>` | `TKS_SERVER_URL` | `http://127.0.0.1:8080` | URL of the running `tks serve` daemon. |
| `--auth-token <TOKEN>` | `TKS_AUTH_TOKEN` | (none) | Bearer authentication token. |
| `--database-url <URL>` | `DATABASE_URL` / `TKS_DATABASE_URL` | `postgres://postgres:postgres@localhost:5432/tks` | Direct database connection string. |
| `--migrate-only` | (none) | (none) | Run pending migrations and exit. |

---

## Subcommand Reference

### 1. `tks serve`

Starts the unified Knowledge Substrate daemon server.

```bash
tks serve [--git-dir <DIR>]
```

* `--git-dir <DIR>`: Path to bare Git repository (default: `.substrate/git/tks.git`).

---

### 2. `tks identity`

Provision and revoke authentication identities.

```bash
# Provision a new identity
tks identity create --name <NAME> --role <ROLE> [--description <DESC>]

# Revoke an identity immediately
tks identity revoke <ID>
```

* `--name`: Unique identity name (e.g. `gemini-agent`).
* `--role`: `HUMAN` or `AGENT`.

---

### 3. `tks staging`

Inspect and approve candidate drafts from document ingestion.

```bash
# List candidate draft nodes for a job
tks staging list <JOB_ID>

# Inspect node metadata and verbatim Git ODB source span
tks staging inspect <JOB_ID> <NODE_ID>

# Approve candidate drafts and promote to ACTIVE
tks staging approve <JOB_ID> [--only <NODE_IDS>] [--exclude <NODE_IDS>]

# Reject candidate drafts
tks staging reject <JOB_ID> [--node-id <NODE_ID>]
```

---

### 4. `tks task`

Create, inspect, and update autonomous execution tasks.

```bash
# Create an execution task
tks task create --parent <PARENT_ID> --title <TITLE> [--content <DESC>] [--assignee <AGENT>]

# Update task lifecycle status
tks task update <TASK_ID> --status <OPEN|IN_PROGRESS|BLOCKED|COMPLETED>

# List tasks
tks task list [--parent <PARENT_ID>] [--status <STATUS>] [--limit <N>]
```

---

### 5. `tks workspace`

Manage branch-isolated multi-agent workspace containers.

```bash
# Create an isolated workspace branch
tks workspace create --name <NAME> [--description <DESC>]

# List active workspaces
tks workspace list

# Inspect candidate drafts in a workspace
tks workspace inspect <WORKSPACE_ID>

# Merge workspace drafts into active substrate
tks workspace merge <WORKSPACE_ID>

# Rebase workspace against latest substrate
tks workspace rebase <WORKSPACE_ID>

# Discard an abandoned workspace
tks workspace discard <WORKSPACE_ID>
```

---

### 6. `tks admin`

Administrative graph repair, rollback, and reverification.

```bash
# Revert mutations with dry-run preview
tks admin revert [--batch-id <UUID>] [--agent-id <UUID>] [--dry-run] [--force]

# Reverify a degraded node in NEEDS_REVERIFICATION
tks admin reverify <NODE_ID> [--rationale <TEXT>] [--reparent-to <NEW_PARENT_ID>]
```

---

### 7. `tks explorer`

Launch or inspect the Real-Time Substrate Web Explorer.

```bash
# Display explorer connection details
tks explorer serve
```

---

### 8. `tks mcp-stdio`

Stdio streaming JSON-RPC proxy for external MCP agent harnesses.

```bash
tks mcp-stdio
```

---

## Operational Cheat Sheet & Recipes

### Recipe 1: Ingesting & Activating a Specification

```bash
# 1. Ingest Markdown file
JOB_ID=$(curl -s -X POST "$TKS_SERVER_URL/api/v1/documents/ingest" \
  -H "Authorization: Bearer $TKS_AUTH_TOKEN" \
  -H "Content-Type: application/json" \
  -d "{\"doc_path\": \"specs/vision.md\", \"content\": $(jq -Rs . < docs/vision/vision.md)}" \
  | jq -r .job_id)

# 2. Wait for worker to stage decomposition
sleep 2

# 3. Review staged drafts
tks staging list "$JOB_ID"

# 4. Approve drafts to ACTIVE
tks staging approve "$JOB_ID"
```

### Recipe 2: Feature Branch Development Loop

```bash
# 1. Create feature workspace
WS_ID=$(tks workspace create --name "feature/auth-jwt" | grep "ID:" | awk '{print $2}')

# 2. Agent elaborates candidate tasks inside workspace via REST / MCP
curl -s -X POST "$TKS_SERVER_URL/api/v1/workspaces/$WS_ID/elaborate-task" \
  -H "Authorization: Bearer $TKS_AUTH_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"parent_id": "<REQ_ID>", "title": "Implement JWT Middleware"}'

# 3. Inspect candidate drafts
tks workspace inspect "$WS_ID"

# 4. Merge into production substrate
tks workspace merge "$WS_ID"
```

### Recipe 3: Invalidation Storm Recovery

```bash
# 1. Find degraded nodes
tks task list --status "NEEDS_REVERIFICATION"

# 2. Reverify node against active parent
tks admin reverify "<NODE_ID>" \
  --rationale "Verified logic conforms to updated governing invariant"
```
