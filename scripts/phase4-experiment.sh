#!/usr/bin/env bash
#
# scripts/phase4-experiment.sh: Experimental setup, governance ingestion,
# task elaboration, and autonomous implementation workflow for Phase 4.
#
# Workflow:
#   1. Clean up running server, wipe bare Git substrate and PostgreSQL database.
#   2. Build TKS release binary and apply initial database migrations.
#   3. Start TKS server daemon (`tks serve`) and provision an agent identity.
#   4. Launch the Web Explorer (`http://localhost:8080/explorer`) and wait for human browser opening.
#   5. Ingest governance documents (vision, architecture, technical backlog, strategic planning backlog).
#   6. Address governance policy (Invariant INV-5) on the Phase 4 parent node.
#   7. Register TKS MCP server with Antigravity CLI (`agy`).
#   8. Launch agy agent to autonomously elaborate Phase 4 tasks in TKS property graph.
#   9. Launch agy agent to implement Phase 4 using TKS as its sole source of truth (zero docs/vision/ access).
#

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

# -----------------------------------------------------------------------------
# Configuration & Defaults
# -----------------------------------------------------------------------------
TKS_PORT="${TKS_PORT:-8080}"
TKS_SERVER_URL="${TKS_SERVER_URL:-http://127.0.0.1:${TKS_PORT}}"
TKS_GIT_DIR="${TKS_GIT_DIR:-${WORKSPACE_ROOT}/.substrate/git/tks.git}"
TKS_BIN="${WORKSPACE_ROOT}/target/release/tks"
AGENT_NAME="${AGENT_NAME:-phase4-autonomous-agent}"
AGENT_MODEL="${AGENT_MODEL:-gemini-3.8-flash-high}"
EXEC_MODE="${EXEC_MODE:-print}" # 'print' (non-interactive) or 'interactive' (TUI)
AUTO_SET_POLICY="${AUTO_SET_POLICY:-true}"

# Resolve database URL (devcontainer Docker compose hostname vs localhost)
if [[ -n "${TKS_DATABASE_URL:-}" ]]; then
    DB_URL="${TKS_DATABASE_URL}"
elif [[ -n "${DATABASE_URL:-}" ]]; then
    DB_URL="${DATABASE_URL}"
elif getent hosts postgres >/dev/null 2>&1; then
    DB_URL="postgresql://postgres:postgres@postgres:5432/postgres"
else
    DB_URL="postgresql://postgres:postgres@localhost:5432/postgres"
fi
export DATABASE_URL="${DB_URL}"
export TKS_DATABASE_URL="${DB_URL}"

LOG_FILE="/tmp/tks-serve.log"
SERVER_PID=""

# Cleanup trap on unexpected exit
cleanup() {
    local exit_code=$?
    if [[ -n "${SERVER_PID:-}" ]] && kill -0 "${SERVER_PID}" 2>/dev/null; then
        echo ""
        echo "==> Notice: TKS server daemon is still running in background (PID: ${SERVER_PID})."
        echo "    To stop it: kill ${SERVER_PID} or fuser -k -TERM ${TKS_PORT}/tcp"
    fi
    exit "${exit_code}"
}
trap cleanup EXIT

echo "=========================================================================="
echo "          TKS PHASE 4 EXPERIMENTAL END-TO-END WORKFLOW                    "
echo "=========================================================================="
echo " Workspace Root: ${WORKSPACE_ROOT}"
echo " Database URL:   ${DB_URL}"
echo " Server URL:     ${TKS_SERVER_URL}"
echo " Git Substrate:  ${TKS_GIT_DIR}"
echo " Agent Model:    ${AGENT_MODEL}"
echo " Execution Mode: ${EXEC_MODE}"
echo " Auto Policy:    ${AUTO_SET_POLICY}"
echo "=========================================================================="
echo ""

# -----------------------------------------------------------------------------
# Step 1: Clean Up Existing Processes & Wipe Substrate / Database
# -----------------------------------------------------------------------------
echo "==> [1/9] Cleaning up existing services, Git substrate, and database..."

# Stop any running server instance
if fuser "${TKS_PORT}/tcp" >/dev/null 2>&1; then
    echo "    Stopping existing process bound to port ${TKS_PORT}..."
    fuser -k -TERM "${TKS_PORT}/tcp" 2>/dev/null || true
    sleep 1
fi
pkill -f "tks serve" 2>/dev/null || true

# Clear out the Git substrate
if [[ -d "${WORKSPACE_ROOT}/.substrate/git" ]]; then
    echo "    Wiping bare Git repository at ${WORKSPACE_ROOT}/.substrate/git..."
    rm -rf "${WORKSPACE_ROOT}/.substrate/git"
fi
mkdir -p "${WORKSPACE_ROOT}/.substrate/git"

# Clear out the PostgreSQL database
echo "    Wiping PostgreSQL schema (public cascade reset)..."
psql "${DB_URL}" -v ON_ERROR_STOP=1 -c \
    "DROP SCHEMA IF EXISTS public CASCADE; CREATE SCHEMA public; CREATE EXTENSION IF NOT EXISTS vector;" >/dev/null

echo "    Substrate and database reset complete."
echo ""

# -----------------------------------------------------------------------------
# Step 2: Build TKS Binary & Apply Migrations
# -----------------------------------------------------------------------------
echo "==> [2/9] Building TKS release binary and applying migrations..."

cd "${WORKSPACE_ROOT}"
cargo build --release --bin tks

echo "    Running standalone database migrations (V1..V4)..."
"${TKS_BIN}" --migrate-only

echo "    Database schema initialized with pgvector and initial seeds."
echo ""

# -----------------------------------------------------------------------------
# Step 3: Start TKS Server Daemon & Provision Identity
# -----------------------------------------------------------------------------
echo "==> [3/9] Starting TKS server daemon in background..."

nohup "${TKS_BIN}" serve \
    --port "${TKS_PORT}" \
    --git-dir "${TKS_GIT_DIR}" \
    > "${LOG_FILE}" 2>&1 &
SERVER_PID=$!

echo "    Server process spawned (PID: ${SERVER_PID}). Waiting for health check..."

HEALTH_OK=false
for i in {1..30}; do
    if curl -s -f "${TKS_SERVER_URL}/health" >/dev/null 2>&1; then
        HEALTH_OK=true
        break
    fi
    sleep 0.5
done

if [[ "${HEALTH_OK}" != "true" ]]; then
    echo "ERROR: TKS server failed to become healthy within 15 seconds." >&2
    echo "--- Server Logs (${LOG_FILE}) ---" >&2
    cat "${LOG_FILE}" >&2
    exit 1
fi

echo "    TKS server is healthy at ${TKS_SERVER_URL}."

# Provision agent identity for autonomous workflows
echo "    Provisioning agent identity '${AGENT_NAME}'..."
CREATE_IDENTITY_OUT=$("${TKS_BIN}" identity create \
    --name "${AGENT_NAME}" \
    --role "AGENT" \
    --server-url "${TKS_SERVER_URL}" \
    --auth-token "tks_dev_token")

AGENT_TOKEN=$(echo "${CREATE_IDENTITY_OUT}" | grep -E "token:" | awk '{print $2}' | tr -d ' \r\n')
if [[ -z "${AGENT_TOKEN}" ]]; then
    echo "    Falling back to development seed token 'tks_dev_token'..."
    AGENT_TOKEN="tks_dev_token"
else
    echo "    Generated bearer token: ${AGENT_TOKEN:0:16}..."
fi

export TKS_AUTH_TOKEN="${AGENT_TOKEN}"
export TKS_SERVER_URL="${TKS_SERVER_URL}"
echo ""

# -----------------------------------------------------------------------------
# Step 4: Prompt Human to Open Web Explorer
# -----------------------------------------------------------------------------
echo "=========================================================================="
echo " 🌐 [4/9] WEB EXPLORER IS LIVE AND READY FOR OBSERVATION"
echo "=========================================================================="
echo " URL:             http://localhost:${TKS_PORT}/explorer"
echo " SSE Event Stream: ${TKS_SERVER_URL}/api/v1/explorer/events"
echo ""
echo " Action Required:"
echo " 1. Open your browser to http://localhost:${TKS_PORT}/explorer"
echo " 2. Verify that the Cytoscape canvas and 'LIVE SSE' indicator are connected."
echo " 3. Leave the browser tab open to watch nodes and edges appear in real time."
echo "=========================================================================="
echo ""
read -r -p "Press [ENTER] to begin governance document ingestion..."
echo ""

# -----------------------------------------------------------------------------
# Step 5: Ingest Governance Documents via CLI & REST
# -----------------------------------------------------------------------------
echo "==> [5/9] Ingesting governance documents into TKS..."

ingest_and_approve() {
    local doc_file="$1"
    local doc_path="$2"

    if [[ ! -f "${doc_file}" ]]; then
        echo "ERROR: Document file not found: ${doc_file}" >&2
        return 1
    fi

    echo "--- Submitting: ${doc_file} -> ${doc_path} ---"

    # Construct JSON payload using jq --rawfile to avoid argument limits
    local payload
    payload=$(jq -n --arg path "${doc_path}" --rawfile content "${doc_file}" \
        '{doc_path: $path, content: $content}')

    local resp
    resp=$(curl -s -f -X POST "${TKS_SERVER_URL}/api/v1/documents/ingest" \
        -H "Authorization: Bearer ${TKS_AUTH_TOKEN}" \
        -H "Content-Type: application/json" \
        -d "${payload}")

    local job_id
    job_id=$(echo "${resp}" | jq -r '.job_id')
    echo "    Ingestion Job ID: ${job_id} (Status: QUEUED)"

    # Poll until background decomposition worker parses CommonMark AST into candidate drafts
    echo "    Waiting for mechanical decomposition worker..."
    local status="QUEUED"
    local retries=40
    while [[ "${status}" != "STAGED" && $retries -gt 0 ]]; do
        sleep 0.5
        local job_info
        job_info=$(curl -s -f "${TKS_SERVER_URL}/api/v1/documents/ingest/${job_id}" \
            -H "Authorization: Bearer ${TKS_AUTH_TOKEN}")
        status=$(echo "${job_info}" | jq -r '.status')

        if [[ "${status}" == "FAILED" ]]; then
            local err_msg
            err_msg=$(echo "${job_info}" | jq -r '.error_message // "unknown error"')
            echo "ERROR: Decomposition failed for ${doc_path}: ${err_msg}" >&2
            return 1
        fi
        retries=$((retries - 1))
    done

    if [[ "${status}" != "STAGED" ]]; then
        echo "ERROR: Timeout waiting for job ${job_id} to reach STAGED (current: ${status})" >&2
        return 1
    fi

    echo "    Decomposition complete (Status: STAGED). Approving candidate drafts..."
    "${TKS_BIN}" staging approve "${job_id}" \
        --server-url "${TKS_SERVER_URL}" \
        --auth-token "${TKS_AUTH_TOKEN}"

    echo "    ✓ Successfully promoted and broadcast via SSE: ${doc_path}"
    sleep 1 # Allow browser SSE animation to settle
}

# 1. Technical Vision
ingest_and_approve "${WORKSPACE_ROOT}/docs/vision/vision.md" "specs/vision.md"

# 2. System Architecture
ingest_and_approve "${WORKSPACE_ROOT}/docs/vision/architecture.md" "specs/architecture.md"

# 3. Technical Backlog
ingest_and_approve "${WORKSPACE_ROOT}/docs/vision/technical-backlog.md" "specs/technical-backlog.md"

# 4. Strategic Planning Backlog (Contains Phase 4 specifications)
ingest_and_approve "${WORKSPACE_ROOT}/docs/vision/strategic-planning-backlog.md" "specs/strategic-planning-backlog.md"

echo ""
echo "    All governance documents ingested and activated in TKS."
echo ""

# -----------------------------------------------------------------------------
# Step 6: Invariant INV-5 Governance Policy Preparation
# -----------------------------------------------------------------------------
echo "==> [6/9] Resolving Phase 4 parent requirement in TKS property graph..."

PHASE4_NODE_ID=$(psql "${DB_URL}" -t -A -c \
    "SELECT id FROM graph_nodes WHERE title ILIKE '%Phase 4%' AND lifecycle_state = 'ACTIVE' ORDER BY created_at ASC LIMIT 1;")

if [[ -z "${PHASE4_NODE_ID}" ]]; then
    echo "WARNING: Could not automatically locate active Phase 4 node. Searching by content..."
    PHASE4_NODE_ID=$(psql "${DB_URL}" -t -A -c \
        "SELECT id FROM graph_nodes WHERE content ILIKE '%Closed-Loop%' AND lifecycle_state = 'ACTIVE' LIMIT 1;")
fi

echo "    Phase 4 Parent Node ID: ${PHASE4_NODE_ID:-<NOT_FOUND>}"

if [[ -n "${PHASE4_NODE_ID}" ]]; then
    CURRENT_POLICY=$(psql "${DB_URL}" -t -A -c \
        "SELECT governance_policy FROM graph_nodes WHERE id = '${PHASE4_NODE_ID}';")
    echo "    Current Governance Policy: ${CURRENT_POLICY}"

    # NOTE ON INVARIANT INV-5 / EXPERIMENTAL HOOP:
    # Under Invariant INV-5, any attempt by an autonomous agent to create subtasks (TASK)
    # directly in ACTIVE state requires the parent node's governance_policy to be
    # 'AUTONOMOUS_ELABORATION'. If it remains 'HUMAN_REVIEW_REQUIRED', the mutation engine
    # strictly rejects the elaboration with ERR_GOVERNANCE_REJECTED.
    if [[ "${AUTO_SET_POLICY}" == "true" ]]; then
        echo "    Updating governance_policy to 'AUTONOMOUS_ELABORATION' (Invariant INV-5 compliance)..."
        psql "${DB_URL}" -c \
            "UPDATE graph_nodes SET governance_policy = 'AUTONOMOUS_ELABORATION' WHERE id = '${PHASE4_NODE_ID}';" >/dev/null
        echo "    ✓ Policy updated. Autonomous agent can elaborate tasks directly under this node."
    else
        echo "    [Experimental Note] AUTO_SET_POLICY is false. Leaving policy as '${CURRENT_POLICY}' to observe governance rejection."
    fi
fi
echo ""

# -----------------------------------------------------------------------------
# Step 7: Configure MCP Server in Antigravity CLI (`agy`)
# -----------------------------------------------------------------------------
echo "==> [7/9] Registering TKS MCP server in Antigravity CLI (agy)..."

# Remove existing stale registration if present
agy mcp remove tks 2>/dev/null || true

# Register tks stdio proxy with environment variables
agy mcp add \
    --env "TKS_SERVER_URL=${TKS_SERVER_URL}" \
    --env "TKS_AUTH_TOKEN=${TKS_AUTH_TOKEN}" \
    tks "${TKS_BIN}" mcp-stdio

echo "    Current agy MCP server list:"
agy mcp list
echo ""

# -----------------------------------------------------------------------------
# Step 8: Task 1 - Launch `agy` Agent for Phase 4 Task Elaboration
# -----------------------------------------------------------------------------
echo "=========================================================================="
echo " 🤖 [8/9] TASK 1: AUTONOMOUS AGENT TASK ELABORATION FOR PHASE 4"
echo "=========================================================================="
echo " Objective:"
echo " The agent will connect via MCP, query Phase 4 deliverables from TKS,"
echo " and elaborate concrete execution tasks in the database."
echo ""
echo " STRICT GOVERNANCE RULE ENFORCED ON AGENT:"
echo " - The agent is FORBIDDEN from reading files in 'docs/vision/'."
echo " - TKS is the SOLE source of truth for all requirements and architecture."
echo "=========================================================================="
echo ""
read -r -p "Press [ENTER] to launch the Elaboration Agent... "
echo ""

# Multi-line elaboration prompt
read -r -d '' ELABORATION_PROMPT <<EOF || true
You are an autonomous software systems architect operating via the Antigravity CLI (agy) and the Knowledge Substrate (TKS) MCP server.

CRITICAL INSTRUCTIONS & STRICT BOUNDARIES:
1. SOLE SOURCE OF TRUTH: You must use the TKS MCP tools (query_requirements, get_context_envelope, get_document_span) as your SOLE source of truth for what to plan and implement.
2. STRICT DIRECTORY FORBIDDEN: You are STRICTLY FORBIDDEN from reading, opening, or grepping files in the "docs/vision" folder. Any violation violates test isolation.
3. GOAL: Elaborate the detailed plan and execution tasks for Phase 4 of this project directly into the TKS property graph database.

WORKFLOW STEPS:
Step 1: Use MCP tool "query_requirements" with query "Phase 4" or "Closed-Loop Lifecycle Verification" to locate the Phase 4 governing requirement node.
Step 2: Use MCP tool "get_context_envelope" on the Phase 4 requirement node ID to inspect its specifications, deliverables, and upward constraints.
Step 3: If necessary, use "get_document_span" to read the exact verbatim Markdown span directly from the Git object database via TKS.
Step 4: Analyze the Phase 4 deliverables described in TKS:
   - Deliverable 1: VCS Commit Linking (webhooks linking commits/PRs to task nodes via IMPLEMENTED_BY edges).
   - Deliverable 2: Automated Test Verification (CI mapping test suite results to verification nodes via VERIFIED_BY edges).
   - Deliverable 3: Spec-Driven Release Readiness Webhooks & Status API (GET /api/v1/release/readiness).
Step 5: Elaborate concrete, atomic execution subtasks (TASK) for each deliverable using the MCP tool "create_subtask" (or "propose_node_mutation" with mutation_type="TASK"):
   - parent_node_id: <The Phase 4 requirement node ID found in Step 1>
   - title: Clear, descriptive task title (e.g. "Phase 4: Implement VCS Commit Linking Webhook Route")
   - content: Detailed technical specification, endpoint signature, schema requirements, and verification criteria.
   - attributes: Include progressive metadata such as {"node_key": "TASK-PHASE4-001"} or similar.
Step 6: Output a clear summary table listing all created task IDs, titles, and keys so they can be executed by the implementation agent.
EOF

if [[ "${EXEC_MODE}" == "interactive" ]]; then
    echo "Starting interactive agy session for task elaboration (model: ${AGENT_MODEL})..."
    agy --model "${AGENT_MODEL}" --prompt-interactive "${ELABORATION_PROMPT}"
else
    echo "Running agy in print mode (--dangerously-skip-permissions, model: ${AGENT_MODEL})..."
    agy --model "${AGENT_MODEL}" -p "${ELABORATION_PROMPT}" --dangerously-skip-permissions
fi

echo ""
echo "✓ Phase 4 Task Elaboration complete. Tasks are committed in TKS property graph."
echo ""

# -----------------------------------------------------------------------------
# Step 9: Task 2 - Launch `agy` Agent for Phase 4 Implementation
# -----------------------------------------------------------------------------
echo "=========================================================================="
echo " 🛠️ [9/9] TASK 2: AUTONOMOUS IMPLEMENTATION OF PHASE 4"
echo "=========================================================================="
echo " Objective:"
echo " The agent will query TKS for the elaborated Phase 4 tasks, update their"
echo " status in TKS, implement the code in Rust, and verify with tests."
echo ""
echo " STRICT GOVERNANCE RULE ENFORCED ON AGENT:"
echo " - The agent is FORBIDDEN from reading files in 'docs/vision/'."
echo " - TKS is the SOLE source of truth for what to implement."
echo "=========================================================================="
echo ""
read -r -p "Press [ENTER] to launch the Implementation Agent... "
echo ""

read -r -d '' IMPLEMENTATION_PROMPT <<EOF || true
You are an autonomous systems developer operating via the Antigravity CLI (agy) and the Knowledge Substrate (TKS) MCP server.

CRITICAL INSTRUCTIONS & STRICT BOUNDARIES:
1. SOLE SOURCE OF TRUTH: You must query the TKS MCP tools (query_requirements, get_context_envelope, update_node_status) as your SOLE source of truth for all specifications, requirements, and tasks.
2. STRICT DIRECTORY FORBIDDEN: You are STRICTLY FORBIDDEN from reading, opening, or searching files in "docs/vision/". Any attempt to read that folder violates the experimental protocol.
3. GOAL: Implement the Phase 4 deliverables in the repository based strictly on the tasks elaborated in TKS.

WORKFLOW STEPS:
Step 1: Use MCP tool "query_requirements" with query "Phase 4" or use "get_context_envelope" to discover all active tasks (node_type = "TASK") elaborated under Phase 4.
Step 2: For each task identified in TKS:
   a. Call MCP tool "update_node_status" with status="IN_PROGRESS" to claim the task.
   b. Read the task content and context envelope to understand the technical requirements.
   c. Implement the necessary Rust code in src/ (e.g. routes, models, webhook handlers, verification edges) and migrations if needed.
   d. Add unit and integration tests verifying the feature.
   e. Run "cargo check" and "cargo test" to verify that your implementation compiles and tests pass.
   f. Call MCP tool "update_node_status" with status="COMPLETED" and notes="Implemented and verified with unit tests."
Step 3: Run "cargo clippy --all-targets --all-features -- -D warnings" and "cargo fmt --check" to ensure code standards.
Step 4: Report your implementation results, files modified, and verification status.
EOF

if [[ "${EXEC_MODE}" == "interactive" ]]; then
    echo "Starting interactive agy session for implementation (model: ${AGENT_MODEL})..."
    agy --model "${AGENT_MODEL}" --prompt-interactive "${IMPLEMENTATION_PROMPT}"
else
    echo "Running agy in print mode (--dangerously-skip-permissions, model: ${AGENT_MODEL})..."
    agy --model "${AGENT_MODEL}" -p "${IMPLEMENTATION_PROMPT}" --dangerously-skip-permissions
fi

echo ""
echo "=========================================================================="
echo " 🎉 EXPERIMENTAL RUN COMPLETE"
echo "=========================================================================="
echo " - Review real-time graph state at: http://localhost:${TKS_PORT}/explorer"
echo " - TKS daemon log:                  ${LOG_FILE}"
echo " - To terminate daemon:             kill ${SERVER_PID} or fuser -k -TERM ${TKS_PORT}/tcp"
echo "=========================================================================="
