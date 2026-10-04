# Chapter 3: Ingesting Governing Documents

This chapter demonstrates how to ingest governing documents into the Knowledge Substrate using [`docs/vision/vision.md`](file:///workspaces/tks/docs/vision/vision.md) and [`docs/vision/architecture.md`](file:///workspaces/tks/docs/vision/architecture.md) as concrete examples.

---

## The Ingestion Pipeline

When a human or CI pipeline submits a Markdown document for ingestion, TKS executes a multi-stage pipeline designed for 100% mechanical determinism and cryptographic provenance:

```mermaid
%%{init: {
  'theme': 'base',
  'themeVariables': {
    'darkMode': true,
    'background': '#161922',
    'mainBkg': '#1e2230',
    'nodeBorder': '#434c5e',
    'textColor': '#e2e8f0',
    'fontFamily': 'ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif',
    'fontSize': '14px',
    'lineColor': '#8892b0',
    'primaryColor': '#422026',
    'primaryTextColor': '#fde8ec',
    'primaryBorderColor': '#e06c75',
    'secondaryColor': '#1b3528',
    'secondaryTextColor': '#e6f7ee',
    'secondaryBorderColor': '#73c991',
    'tertiaryColor': '#1d2c44',
    'tertiaryTextColor': '#e4f0fc',
    'tertiaryBorderColor': '#61afef',
    'clusterBkg': '#13161f',
    'clusterBorder': '#373e51',
    'noteBkgColor': '#2e271a',
    'noteTextColor': '#fdf4db',
    'noteBorderColor': '#e5c07b',
    'edgeLabelBackground': '#1a1d27'
  }
}}%%
flowchart TD
    classDef primary fill:#422026,stroke:#e06c75,stroke-width:1.5px,color:#fde8ec;
    classDef secondary fill:#1b3528,stroke:#73c991,stroke-width:1.5px,color:#e6f7ee;
    classDef tertiary fill:#1d2c44,stroke:#61afef,stroke-width:1.5px,color:#e4f0fc;
    classDef note fill:#2e271a,stroke:#e5c07b,stroke-width:1.5px,color:#fdf4db;

    DOC["📄 Markdown Document<br/><i>(e.g. docs/vision/vision.md)</i>"]:::secondary
    API["🌐 POST /api/v1/documents/ingest<br/><i>(Stateless Ingestion Gateway)</i>"]:::secondary

    subgraph StorageTier["Persistence Layer"]
        GIT["🗄️ Bare Git Repository<br/>• Commit to refs/heads/specs<br/>• Extract Git blob hash"]:::tertiary
        DB["🗄️ PostgreSQL Database<br/>• Insert ingestion_jobs (QUEUED)<br/>• Supersede prior open jobs"]:::tertiary
    end

    WORKER["⚡ Background Worker<br/>• Claims job (status = 'PROCESSING')<br/>• Mechanical AST Parsing (pulldown-cmark)<br/>• Heading stack generates DERIVED_FROM edges<br/>• Exact 0-based byte spans extracted<br/>• Candidate drafts written to graph_nodes (DRAFT)<br/>• Transitions ingestion_jobs to STAGED"]:::secondary

    CLI["🔍 Staging Review & CLI<br/>• tks staging list &lt;job_id&gt;<br/>• tks staging inspect &lt;job_id&gt; &lt;node_id&gt; (verbatim Git slice)"]:::note
    APPROVE["🛡️ Staging Approval<br/>• tks staging approve &lt;job_id&gt;<br/>• Promotes candidate nodes to ACTIVE<br/>• Commits events to audit_ledger<br/>• Enqueues vector generation into node_embeddings"]:::primary

    DOC --> API
    API --> GIT
    API --> DB
    GIT --> WORKER
    DB --> WORKER
    WORKER --> CLI
    CLI --> APPROVE
```

---

## Step 1: Submitting a Document for Ingestion

Submit the document content to the ingestion endpoint:

```bash
# Ingest docs/vision/vision.md
curl -s -X POST "$TKS_SERVER_URL/api/v1/documents/ingest" \
  -H "Authorization: Bearer $TKS_AUTH_TOKEN" \
  -H "Content-Type: application/json" \
  -d "{
    \"doc_path\": \"specs/vision.md\",
    \"content\": $(jq -Rs . < docs/vision/vision.md)
  }"
```

Response:

```json
{
  "job_id": "8fb41f11-76a0-48e2-9b24-77a83d739811",
  "status": "QUEUED"
}
```

Behind the scenes:

1. **Git Commit:** The Git write actor commits the document to the bare Git repository at `refs/heads/specs:specs/vision.md`. This gives the document an immutable Git commit hash and blob hash.
2. **Job Record:** A row is inserted into `ingestion_jobs` with status `QUEUED`.
3. **Supersession Sweep:** If a prior open ingestion job existed for `specs/vision.md`, its partial drafts are cleaned up to prevent orphaned candidate rows.

---

## Step 2: Mechanical AST Parsing & Candidate Staging

The background decomposition worker continuously polls for `QUEUED` jobs. Once claimed:

1. **Streaming CommonMark Parser:** The worker processes the markdown text using `pulldown-cmark`. It mechanically parses headings (`H1` through `H4`), code blocks, bullet lists, and paragraphs.
2. **Heading Stack Hierarchy:** The parser tracks the heading stack to build parent-child upward hierarchy edges (`child -DERIVED_FROM-> parent_heading`).
3. **Deterministic Heading Anchors:** Each section receives a normalized anchor (e.g. `ast_anchor = "core-invariants"`).
4. **Exact Byte-Span Coordinates:** Exact 0-based byte offsets (`byte_start`, `byte_end`) are computed directly on the raw UTF-8 byte stream.
5. **Zero Token Cost:** Over 80% of the document decomposition is performed mechanically with zero LLM inference tokens.
6. **Draft Isolation:** Extracted atomic requirements are inserted into `graph_nodes` with `lifecycle_state = 'DRAFT'`, leaving production queries completely unaffected.

---

## Step 3: Staging Review via CLI

Administrators and developers review candidate drafts before they enter the active knowledge graph:

### List Candidate Drafts

```bash
cargo run --bin tks -- staging list 8fb41f11-76a0-48e2-9b24-77a83d739811
```

Output:

```text
Ingestion Job: 8fb41f11-76a0-48e2-9b24-77a83d739811
Document Path: specs/vision.md
Status:        STAGED
Candidate Nodes (7):
  - [REQUIREMENT] 0a11bc32-1200-47de-9851-bc0128490a1a: Invariant I-1 (Ancestry Verification)
  - [REQUIREMENT] 1b22cd43-2311-48ef-a962-cd1239501b2b: Invariant I-2 (Append-Only State Reconstruction)
  - [REQUIREMENT] 2c33de54-3422-49f0-ba73-de2340612c3c: Invariant I-3 (Zero In-Database Agent Execution)
  - [REQUIREMENT] 3d44ef65-4533-40a1-cb84-ef3451723d4d: Invariant I-4 (Cryptographic Source Anchoring)
  - [REQUIREMENT] 4e55fa76-5644-41b2-dc95-fa4562834e5e: Invariant I-5 (Explicit Per-Node Governance Authority)
  - [REQUIREMENT] 5f66ab87-6755-42c3-ed06-ab5673945f6f: Invariant I-6 (Self-Referential Bootstrapping)
  - [REQUIREMENT] 6a77bc98-7866-43d4-fe17-bc6784056a7a: Invariant I-7 (Agent Identity Attribution)
```

### Inspect Verbatim Source Span

To ensure accuracy, inspect a specific node. TKS reads the byte coordinates (`byte_start..byte_end`) and fetches the verbatim text slice directly from the Git ODB:

```bash
cargo run --bin tks -- staging inspect \
  8fb41f11-76a0-48e2-9b24-77a83d739811 \
  0a11bc32-1200-47de-9851-bc0128490a1a
```

Output:

```text
Node ID:        0a11bc32-1200-47de-9851-bc0128490a1a
Title:          Invariant I-1 (Ancestry Verification)
Type:           REQUIREMENT
Lifecycle:      DRAFT
Governance:     HUMAN_REVIEW_REQUIRED
Document Path:  specs/vision.md
Git Blob Hash:  f821e29c0a...
Byte Span:      [1420..1760] (340 bytes)

--- Verbatim Document Span ---
* **Invariant I-1 (Ancestry Verification):** The core database engine shall reject any
  transaction attempting to transition a task or specification to `ACTIVE` if the entity
  lacks an unbroken upward path of active governance edges leading to an active root
  requirement.
------------------------------
```

---

## Step 4: Staging Approval & Promotion

Once the candidate drafts are verified, approve the job:

```bash
cargo run --bin tks -- staging approve 8fb41f11-76a0-48e2-9b24-77a83d739811
```

Output:

```text
Successfully approved ingestion job 8fb41f11-76a0-48e2-9b24-77a83d739811:
  Promoted Nodes: 7
  Promoted Edges: 6
  Audit Batch ID: e41b77a0-0012-4211-9a71-c01178229a1b
  Vector Tasks:   7 enqueued
```

### What Happens During Approval?

1. **State Promotion:** All candidate draft nodes transition to `ACTIVE` in `graph_nodes`.
2. **Edge Activation:** Upward hierarchy edges (`DERIVED_FROM`) activate in `graph_edges`.
3. **Audit Ledger Commit:** Canonical audit records are appended to `audit_ledger` with monotonic sequence numbers (`event_seq`) and a shared `batch_id`.
4. **Vector Queueing:** Rows are upserted into `node_embeddings` with `status = 'PENDING'`. The background embedding worker immediately computes 384-dimensional dense vectors using FastEmbed/all-MiniLM-L6-v2 (<15ms per node) and updates `status = 'COMPLETED'`.
5. **Full-Text GIN Index:** PostgreSQL automatically updates the partial GIN index on `graph_nodes(search_tsv)` for sub-5ms full-text keyword retrieval.
