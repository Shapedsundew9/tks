# Chapter 6: Multi-Agent Workspaces & Collaborative Co-Evolution

This chapter details how multiple AI agents collaborate concurrently without lock contention, using branch-isolated workspace containers and three-way topological merges.

---

## The Challenge of Multi-Agent Co-Evolution

When scaling to multiple autonomous agents working on the same codebase:

1. **Global Lock Bottlenecks:** If every agent's candidate tasks require acquiring the global structural advisory lock (`pg_advisory_xact_lock`), concurrent agents block each other and throughput plummets.
2. **Draft Pollution:** Tentative or experimental architectural proposals must not pollute production queries or confuse peer agents.
3. **Merge Conflicts & Broken Lineage:** If Agent A modifies a requirement while Agent B elaborates tasks under an older version of that requirement, merging Agent B's branch must detect divergence and heal parent references without crashing.

---

## Workspace Containers & Invariant I-7

TKS implements **Branch Workspace Containers** (`workspaces` table, Migration V4). A workspace is an ephemeral, isolated branch context snapshotting the active substrate state:

* **Caller Draft Confidentiality (`INV-7`):** Draft entities created within a workspace are tagged with `attributes->'workspace_id'`. They are strictly invisible to other agents and production queries.
* **Advisory-Lock-Free Elaboration (D-88):** Agents elaborate candidate drafts inside their workspace container without acquiring the global structural advisory lock.
* **Dynamic Query Overlays:** When an agent queries the substrate with its `workspace_id`, TKS dynamically overlays committed `ACTIVE` nodes with that workspace's candidate drafts.

---

## Step 1: Creating a Workspace Container

Agent Alpha creates a workspace container to work on the Vector Search feature:

```bash
cargo run --bin tks -- workspace create \
  --name "feature/vector-search" \
  --description "Add HNSW cosine indexing to node_embeddings"
```

Output:

```text
Workspace successfully created:
  ID:          a1b2c3d4-e5f6-4a1b-8c2d-3e4f5a6b7c8d
  Name:        feature/vector-search
  Owner:       gemini-agent-alpha
  Snapshot:    Revision 42 (active graph snapshot)
  Status:      ACTIVE
```

---

## Step 2: Elaborating Tasks in Isolation

Agent Alpha elaborates candidate execution subtasks under Invariant I-3 within the workspace:

### Via REST API

```bash
curl -s -X POST "$TKS_SERVER_URL/api/v1/workspaces/a1b2c3d4-e5f6-4a1b-8c2d-3e4f5a6b7c8d/elaborate-task" \
  -H "Authorization: Bearer $TKS_AUTH_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "parent_id": "2c33de54-3422-49f0-ba73-de2340612c3c",
    "title": "Configure HNSW Cosine Index in src/storage/envelope.rs",
    "description": "Ensure partial index only indexes status = COMPLETED"
  }'
```

Response:

```json
{
  "node_id": "99eeaa11-2233-4455-6677-8899aabbccdd",
  "workspace_id": "a1b2c3d4-e5f6-4a1b-8c2d-3e4f5a6b7c8d",
  "lifecycle_state": "DRAFT",
  "governance_policy": "AUTONOMOUS_ELABORATION"
}
```

Meanwhile, Agent Beta working in `feature/markdown-parser` can concurrently elaborate tasks under other requirements without taking any database locks against Agent Alpha.

---

## Step 3: Inspecting Workspace Drafts

List and inspect candidate entities within a workspace:

```bash
# List all workspaces
cargo run --bin tks -- workspace list

# Inspect candidate draft nodes in the workspace
cargo run --bin tks -- workspace inspect a1b2c3d4-e5f6-4a1b-8c2d-3e4f5a6b7c8d
```

Output:

```text
Workspace:    feature/vector-search (a1b2c3d4-e5f6-4a1b-8c2d-3e4f5a6b7c8d)
Owner:        gemini-agent-alpha
Status:       ACTIVE
Draft Nodes (1):
  - [TASK] 99eeaa11-2233-4455-6677-8899aabbccdd: Configure HNSW Cosine Index in src/storage/envelope.rs
Draft Edges (1):
  - 99eeaa11... -[FULFILLS]-> 2c33de54... (Invariant I-3)
```

---

## Step 4: Three-Way Topological Merging

When Agent Alpha's feature is complete, the workspace is merged into the live substrate:

```bash
cargo run --bin tks -- workspace merge a1b2c3d4-e5f6-4a1b-8c2d-3e4f5a6b7c8d
```

Output:

```text
Workspace a1b2c3d4-e5f6-4a1b-8c2d-3e4f5a6b7c8d successfully merged:
  Promoted Nodes: 1
  Promoted Edges: 1
  Audit Batch ID: 77bb22aa-3344-5566-7788-99aabbccddee
```

### The Merge Engine Protocol

During `workspace merge`, TKS executes the following verification pipeline:

1. **Canonical Lock Serialization (DEC-3.8):** Transactions acquire locks in deterministic canonical order (sorted `workspace_id`, then sorted `node_id`), eliminating deadlock cascades (`40P01`).
2. **Three-Way Topological Merge CTE (DEC-3.6):** Identifies the common ancestor snapshot between the workspace and the production graph, detecting deleted base nodes or conflicting edge additions.
3. **Combined Subgraph Cycle Detection:** Verifies that promoting the workspace's draft edges will not create circular dependencies across the entire combined graph.
4. **Deterministic Auto-Reparenting Lineage Resolution (DEC-3.7):** If an upstream parent node was superseded during branch development, TKS automatically traces `replaces_node_id` pointers to reparent the child task to the active successor. If the parent was deleted, the child node is staged in `NEEDS_REVERIFICATION` with diagnostic parent pointers rather than corrupting the graph.
5. **Atomic Promotion & Event Emission:** Drafts transition to `ACTIVE`, and the in-process `GraphEventBus` emits real-time merge notifications to live listeners.

---

## Discarding a Workspace

If an experimental branch is abandoned, discard it cleanly:

```bash
cargo run --bin tks -- workspace discard a1b2c3d4-e5f6-4a1b-8c2d-3e4f5a6b7c8d
```

All candidate drafts tagged with `workspace_id` are purged, leaving zero residue in the production graph.
