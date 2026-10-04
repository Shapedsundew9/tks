# Chapter 4: Context Envelopes & Requirement Retrieval

This chapter explains how developers and AI agents query the Knowledge Substrate for architectural context without overwhelming LLM context windows.

---

## The Retrieval Problem: Token Flooding vs. Missing Context

When coding agents generate or modify software, they need relevant architectural context. Traditional approaches fail in two distinct ways:

* **Naïve Whole-Document Dumping:** Passing entire 50-page markdown documents into prompts causes token exhaustion, high inference latency, and instruction neglect.
* **Pure Semantic Vector Search (Standard RAG):** Chunk-based vector search returns fragmented paragraphs that lack causal relationships. For example, an agent might retrieve a code snippet about a database transaction without retrieving the governing security invariant that prohibits plain-text credentials.

---

## The TKS Approach: Bounded Topological Context Envelopes

TKS solves this with **Graph-Bounded Context Envelopes**. A context envelope is an assembled subgraph containing:

1. **Topological Ancestors:** The chain of governing requirements and specifications leading upward from the target node to the root.
2. **Sibling Constraints:** Cross-cutting invariants and constraints (`CONSTRAINED_BY`) linked to the target node or its ancestors.
3. **Semantic Vector Neighbors:** Related nodes retrieved via cosine similarity search in `pgvector` (`status = 'COMPLETED'`).

All retrieval is strictly bounded:

* **Depth Limit:** Traversal depth is clamped to $k \le 3$ hops.
* **Node Quota:** Total returned entities are strictly capped at **40 nodes**.
* **Dynamic Budget Partitioning:** 30 nodes reserved for deterministic topological hierarchy, with up to 10 slots allocated for vector neighbors. If embeddings are pending, the full 40-node quota reallocates to topological traversal.
* **Latency SLA:** The entire query resolves in $<50\text{ ms}$ under `READ COMMITTED` (SLA-1).

---

## Step 1: Full-Text Requirement Search

To discover relevant requirements, agents invoke `query_requirements`:

### Search via REST API

```bash
curl -s -X POST "$TKS_SERVER_URL/api/v1/requirements/query" \
  -H "Authorization: Bearer $TKS_AUTH_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "query": "ancestry verification active root",
    "limit": 5
  }'
```

### Search via MCP Tool

```json
{
  "name": "query_requirements",
  "arguments": {
    "query": "ancestry verification active root",
    "limit": 5
  }
}
```

### Search Response Payload

```json
{
  "matches": [
    {
      "id": "0a11bc32-1200-47de-9851-bc0128490a1a",
      "node_key": "INV-1",
      "title": "Invariant I-1 (Ancestry Verification)",
      "node_type": "REQUIREMENT",
      "lifecycle_state": "ACTIVE",
      "governance_policy": "HUMAN_REVIEW_REQUIRED",
      "rank": 0.892
    }
  ]
}
```

Full-text queries execute against a PostgreSQL native `tsvector` column with a partial GIN index restricted to `lifecycle_state = 'ACTIVE'`, executing in $<5\text{ ms}$ with built-in hyphen sanitization.

---

## Step 2: Assembling a Context Envelope

Once the agent has identified a target requirement or task ID, it requests a bounded context envelope:

### Envelope via REST API

```bash
curl -s -X POST "$TKS_SERVER_URL/api/v1/context/envelope" \
  -H "Authorization: Bearer $TKS_AUTH_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "requirement_id": "0a11bc32-1200-47de-9851-bc0128490a1a",
    "depth": 3
  }'
```

### Envelope via MCP Tool

```json
{
  "name": "get_context_envelope",
  "arguments": {
    "target_node_id": "0a11bc32-1200-47de-9851-bc0128490a1a",
    "depth": 3
  }
}
```

### Structure of the Returned Envelope

```json
{
  "target_node": {
    "id": "0a11bc32-1200-47de-9851-bc0128490a1a",
    "node_key": "INV-1",
    "title": "Invariant I-1 (Ancestry Verification)",
    "lifecycle_state": "ACTIVE"
  },
  "ancestors": [
    {
      "id": "f109aa12-0001-4411-9123-aa01928374bb",
      "title": "Core System Invariants",
      "node_type": "REQUIREMENT",
      "hop_distance": 1
    }
  ],
  "constraints": [
    {
      "id": "1b22cd43-2311-48ef-a962-cd1239501b2b",
      "node_key": "INV-2",
      "title": "Invariant I-2 (Append-Only State Reconstruction)"
    }
  ],
  "vector_neighbors": [
    {
      "id": "8c44de99-5511-4011-8812-ef44910283aa",
      "node_key": "D-8",
      "title": "D-8: Transactional Cycle Prevention via Recursive CTEs",
      "similarity": 0.841
    }
  ],
  "edges": [
    {
      "from_node_id": "0a11bc32-1200-47de-9851-bc0128490a1a",
      "to_node_id": "f109aa12-0001-4411-9123-aa01928374bb",
      "edge_type": "DERIVED_FROM"
    }
  ]
}
```

---

## Step 3: Verbatim Source Span Retrieval

If an agent needs to inspect the exact original text of a requirement without relying on summaries, it calls `get_document_span`:

### Span via MCP Tool

```json
{
  "name": "get_document_span",
  "arguments": {
    "node_id": "0a11bc32-1200-47de-9851-bc0128490a1a"
  }
}
```

### Verbatim Span Response

```json
{
  "node_id": "0a11bc32-1200-47de-9851-bc0128490a1a",
  "doc_path": "specs/vision.md",
  "doc_hash": "f821e29c0a37e193301a9101...",
  "byte_start": 1420,
  "byte_end": 1760,
  "content": "* **Invariant I-1 (Ancestry Verification):** The core database engine shall reject any transaction attempting to transition a task or specification to `ACTIVE` if the entity lacks an unbroken upward path of active governance edges leading to an active root requirement."
}
```

Because source documents reside in the bare Git repository, concurrent reads execute lock-free via `tokio::task::spawn_blocking` and libgit2's immutable Object Database (`git2::Odb::read`). The database stores only 8-byte integers for span offsets, eliminating text duplication and table bloat.
