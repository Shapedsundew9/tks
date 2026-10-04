# Chapter 8: Web Explorer & Real-Time Observability

This chapter covers the real-time visual explorer and observability architecture of the Knowledge Substrate.

---

## Zero-CDN Embedded Architecture

To satisfy **Constraint C-2** (strict offline operation and zero external network dependencies), the TKS Web Explorer is compiled directly into the Rust binary:

* **Embedded Frontend Assets (DEC-3.10):** HTML, CSS, Cytoscape.js, and the Dagre hierarchical graph layout engine are embedded via `rust-embed` and `include_str!`.
* **Zero External CDNs:** The web interface loads instantaneously even in completely air-gapped environments or offline development containers without internet access.
* **Single Daemon Serving:** The web UI is hosted on the same HTTP gateway as the REST API and MCP server (`GET /explorer`).

---

## Launching the Explorer

### Serving via CLI

To launch or print connection instructions for the explorer:

```bash
cargo run --bin tks -- explorer serve
```

Output:

```text
Knowledge Substrate Web Explorer running:
  URL:            http://127.0.0.1:8080/explorer
  Live SSE Stream: http://127.0.0.1:8080/api/v1/explorer/events
  Status:         Active (listening for graph events)
```

Open `http://localhost:8080/explorer` in your browser.

---

## Real-Time Graph Telemetry via SSE

Rather than polling the database, the Web Explorer connects to a persistent Server-Sent Events (SSE) telemetry stream:

* **Endpoint:** `GET /api/v1/explorer/events`
* **Driver:** In-process [`GraphEventBus`](file:///workspaces/tks/src/gateway/event_bus.rs) utilizing a high-throughput Tokio broadcast channel.
* **Performance (DEC-3.12):** Events are demultiplexed and dispatched to browser clients with sub-millisecond latency.

### Live Event Stream Wire Format

```text
event: graph_event
data: {
  "event_type": "invalidation_cascade",
  "source_node_id": "0a11bc32-1200-47de-9851-bc0128490a1a",
  "affected_nodes": [
    "4d99aa12-5501-44bb-9901-aa1122334455",
    "99eeaa11-2233-4455-6677-8899aabbccdd"
  ],
  "max_depth": 2,
  "timestamp": "2026-10-04T15:25:00Z"
}
```

When an invalidation cascade fires or an agent merges a branch workspace, the Web Explorer animates the affected nodes dynamically, changing node border colors to amber (`NEEDS_REVERIFICATION`) or green (`ACTIVE`) without requiring a browser refresh.

---

## Subgraph Extraction API

The explorer visualizer queries the backend via the Subgraph Extraction endpoint (`GET /api/v1/explorer/graph`):

```bash
curl -s -X GET "$TKS_SERVER_URL/api/v1/explorer/graph?depth=4&include_drafts=true" \
  -H "Authorization: Bearer $TKS_AUTH_TOKEN"
```

### Response Payload

The endpoint returns Cytoscape-compliant JSON elements:

```json
{
  "elements": [
    {
      "data": {
        "id": "0a11bc32-1200-47de-9851-bc0128490a1a",
        "label": "INV-1: Ancestry Verification",
        "type": "REQUIREMENT",
        "state": "ACTIVE"
      }
    },
    {
      "data": {
        "id": "edge-1",
        "source": "4d99aa12-5501-44bb-9901-aa1122334455",
        "target": "0a11bc32-1200-47de-9851-bc0128490a1a",
        "label": "FULFILLS"
      }
    }
  ]
}
```

---

## Observability & Key Operational Metrics

The substrate daemon emits structured JSON logs via `tracing` with span context (request ID, agent ID, operation type) strictly to `stderr`.

Key operational metrics emitted in logs include:

* **CTE Traversal Latency:** Time spent executing recursive graph traversals (target: $<20\text{ ms}$).
* **Lock Wait Time:** Time spent waiting for PostgreSQL transaction advisory and row-level locks.
* **Worker Queue Depth:** Pending and processing counts in `ingestion_jobs` and `node_embeddings`.
* **Cascade Sweep Duration:** Number of nodes evaluated and updated during invalidation cascades.
