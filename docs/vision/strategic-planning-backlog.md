# Strategic Planning Backlog: Architecture, Roadmap, and Implementation Tactics

---

## 1. Executive Summary & Strategic Context

This document is the execution, tactical, and architectural companion to the [Technical Vision](file:///workspaces/tks/docs/vision/vision.md). While the vision document defines the enduring North Star, foundational principles, environmental boundaries, and falsification criteria, this backlog details the phased execution roadmap, technical evaluation spikes, operational metrics, and concrete implementation workflows.

This backlog is specifically organized around the **"Start Small"**, **"Constrained Resource Model"**, and **"Self-Referential Dogfooding"** directives:

- Minimize premature architectural complexity during early phases.
- Build the simplest viable mechanisms that satisfy the core vision invariants.
- Scope Phase 0 and Phase 1 to be fully achievable by a small team or solo developer using commodity infrastructure, ensuring each phase delivers standalone value.
- Reach self-hosting as early as possible so the system defines, tracks, and governs its own ongoing development.

---

## 2. Phased Execution Roadmap

```mermaid
%%{init: {
  'theme': 'base',
  'themeVariables': {
    'darkMode': true,
    'background': '#161922',
    'mainBkg': '#1e2230',
    'nodeBorder': '#434c5e',
    'textColor': '#e2e8f0',
    'fontFamily': 'ui-sans-serif, system-ui, sans-serif',
    'lineColor': '#8892b0',
    'primaryColor': '#1b3528',
    'primaryBorderColor': '#73c991',
    'secondaryColor': '#1d2c44',
    'secondaryBorderColor': '#61afef'
  }
}}%%
flowchart LR
    classDef phase fill:#1b3528,stroke:#73c991,stroke-width:1.5px,color:#e6f7ee;
    classDef gate fill:#1d2c44,stroke:#61afef,stroke-width:1.5px,color:#e4f0fc;

    P0["Phase 0: Pre-Construction Spikes<br/><i>(Lightweight H-1 & H-4 De-risking)</i>"]:::phase
    P1["Phase 1: Substrate Core & Read Context<br/><i>(Git Store + PG Graph + Read MCP)</i>"]:::phase
    G1["Dogfooding Gate 1:<br/><i>Self-Ingest Vision & Backlog</i>"]:::gate
    P2["Phase 2: Bounded Mutation & Governance<br/><i>(Write MCP + Audit Log + Node Policies)</i>"]:::phase
    G2["Dogfooding Gate 2:<br/><i>Manage Phase 3 Tasks via Substrate</i>"]:::gate
    P3["Phase 3: Impact Analysis & Observability<br/><i>(Web Portal + Invalidation Cascading)</i>"]:::phase
    P4["Phase 4: Closed-Loop Traceability<br/><i>(Git Commits + Automated Verification)</i>"]:::phase

    P0 --> P1 --> G1 --> P2 --> G2 --> P3 --> P4
```

### Phase 0: Pre-Construction Hypothesis De-risking & Prototype Spikes

- **Primary Objective:** Empirically validate the foundational scientific premise (Hypothesis H-1) and baseline assisted extraction feasibility (Hypothesis H-4) using lightweight, throwaway prototypes before committing to Phase 1 infrastructure construction.
- **Core Deliverables:**
  1. **Spike 0 (H-1 Directional Validation Spike):** Rapid throwaway test comparing graph-bounded context retrieval against a competent multi-tool agentic retrieval baseline (file reading, grep, AST symbol search, and semantic search without graph-structured requirement context) on an in-memory graph of hand-curated requirement nodes (~50–100 nodes), measuring constraint violation reduction during agentic code synthesis.
  2. **Early Extraction Prompting Spike (H-4 Pre-Validation):** Empirical benchmarking of commodity LLM decomposition prompts against representative technical Markdown specs to verify character offset extraction accuracy.
  3. **Foundational Architecture Scaffolding:** Initial repository setup, developer tooling, Docker compose definition for PostgreSQL with `pgvector`, and baseline migration harness.

### Phase 1: Substrate Core, Git Document Ingestion & Context Gateway

- **Primary Objective:** Deliver a functioning read-only pipeline that ingests Markdown specifications into a Git-backed document repository, decomposes them into structured requirement nodes in PostgreSQL, and serves bounded context envelopes to external agents via the Model Context Protocol (MCP).
- **Core Deliverables:**
  1. **Git-Backed Document Store:** Local/server-side Git repository integration for storing raw Markdown/text documents, generating commit and blob object references.
  2. **PostgreSQL Relational & Graph Schema:** Core tables for requirement entities, structural edges (`DERIVED_FROM`, `CONSTRAINED_BY`), source text span references (document hash, character start/end), and vector embeddings (`pgvector`).
  3. **Assisted Decomposition Pipeline:** Ingestion REST API endpoint orchestrating prompt-based requirement extraction via external LLM APIs, featuring a deterministic span re-anchoring post-processor (resolving exact byte/character offsets in source documents from extracted requirement text) and a staging table for human review before graph insertion.
  4. **Read-Only MCP Server & Identity Scaffolding:** MCP endpoint exposing tool `get_context_envelope` that performs directed graph traversals and vector neighbor lookups, with stateless caller identity verification scaffolding (Invariant I-7).
- **Dogfooding Milestone (Gate 1):** Ingest the project's own `vision.md` and `strategic-planning-backlog.md` into the Phase 1 substrate, verifying that atomic requirements and constraints can be queried via MCP.

### Phase 2: Bounded Agent Mutation & Per-Node Governance

- **Primary Objective:** Enable external AI agents to submit candidate graph mutations (sub-tasks, refined specifications, test definitions) while enforcing per-node governance policies, non-negotiable identity attribution, and reversible, append-only auditability.
- **Core Deliverables:**
  1. **Mutation-Enabled MCP Tools:** MCP tools `propose_node_mutation`, `create_subtask`, and `update_node_status`, with caller authentication and token verification.
  2. **Append-Only Audit Ledger:** Change-log mechanism recording all entity and edge mutations as discrete, reversible delta events, capturing caller identity (`agent_instance_id`, `caller_type`, `auth_fingerprint`) alongside mutation payloads.
  3. **Governance Policy Engine:** Database triggers and gateway filters enforcing `governance_policy` flags (`AUTONOMOUS_ELABORATION`, `HUMAN_REVIEW_REQUIRED`, `LOCKED`).
  4. **Rollback & Reversion Utility:** Administrative tooling to cleanly revert a sequence of agent mutations without leaving orphan nodes or dangling relationships.
  5. **Semantic Corruption Defenses & DAG Cycle Validation:** Database-level DAG constraints preventing circular dependency creation on structural edges (`CONSTRAINED_BY`, `DERIVED_FROM`), combined with agent-instance-scoped batch rollback tooling to cleanly neutralize corrupted or adversarial mutation bursts.
  6. **Node Lifecycle State Architecture (Phase 2+ Consideration):** Introduce explicit entity lifecycle states (`ACTIVE`, `SUPERSEDED`, `ARCHIVED`) stored in node attributes to prevent graph growth and obsolete requirement versions from degrading context envelope precision and relevance over time.
- **Dogfooding Milestone (Gate 2):** Use external coding agents (e.g., Claude Code, Aider, custom runners) operating through MCP to claim Phase 3 preparation tasks, generate implementation sub-specs, and commit them directly to the substrate.

### Phase 3: Topological Impact Analysis & Human Supervisory Portal

- **Primary Objective:** Provide human engineering leads with high-level observability and automated impact analysis when upstream requirements change.
- **Core Deliverables:**
  1. **Web-Based Supervisory Portal:** Interactive UI for visual graph exploration, node status inspection, and human review queues for `PENDING_REVIEW` mutations.
  2. **Automated Invalidation Cascading:** Graph traversal algorithms that detect modifications to upstream requirements, recursively mark downstream specifications and tasks as `NEEDS_REVERIFICATION`, and block dependent agent execution until resolved.
  3. **Impact Analysis Dashboard:** Visual diff tool showing the topological blast radius of a proposed requirement revision.

### Phase 4: Closed-Loop Lifecycle Verification & Source Code Mapping

- **Primary Objective:** Establish bidirectional synchronization between leaf tasks in the knowledge graph, source code repository commits, and automated test execution results.
- **Core Deliverables:**
  1. **VCS Commit Linking:** Webhooks connecting repository commits and pull requests directly to task nodes (`IMPLEMENTED_BY` edges).
  2. **Automated Test Verification:** CI pipeline integration mapping test suite execution results to verification nodes (`VERIFIED_BY` edges), enforcing automated proof of requirement satisfaction prior to release sign-off.
  3. **Spec-Driven Release Gateways:** Formal release validation gates preventing deployment if unresolved or invalidated requirement paths remain in the graph.

---

## 3. Self-Referential Bootstrapping & Dogfooding Strategy

A foundational principle of the project is that the system must reach self-hosting capability, analogous to a self-hosting compiler. This addresses the classic bootstrapping tension ("start small" while aiming to manage complex lifecycles).

```mermaid
%%{init: {
  'theme': 'base',
  'themeVariables': {
    'darkMode': true,
    'background': '#161922',
    'mainBkg': '#1e2230',
    'nodeBorder': '#434c5e',
    'textColor': '#e2e8f0',
    'fontFamily': 'ui-sans-serif, system-ui, sans-serif',
    'lineColor': '#8892b0',
    'primaryColor': '#1b3528',
    'primaryBorderColor': '#73c991',
    'secondaryColor': '#1d2c44',
    'secondaryBorderColor': '#61afef',
    'tertiaryColor': '#422026',
    'tertiaryBorderColor': '#e06c75'
  }
}}%%
flowchart TD
    classDef external fill:#422026,stroke:#e06c75,stroke-width:1.5px,color:#fde8ec;
    classDef transition fill:#1d2c44,stroke:#61afef,stroke-width:1.5px,color:#e4f0fc;
    classDef internal fill:#1b3528,stroke:#73c991,stroke-width:1.5px,color:#e6f7ee;

    subgraph Phase0["Phase 0: External Bootstrap & Early De-risking"]
        E1["Manual Git Repository Management"]:::external
        E2["Spike 0: Rapid H-1 & H-4 Directional Validation"]:::external
        E3["Human-Driven Architecture & Coding"]:::external
    end

    subgraph Transition["Phase 1 Completion: Self-Hosting Transition"]
        T1["Ingest vision.md & backlog.md into Substrate"]:::transition
        T2["Decompose Core Invariants into Graph Nodes"]:::transition
        T3["Verify Self-Querying via MCP Server"]:::transition
    end

    subgraph Phase2Plus["Phase 2+: Autonomous Self-Evolution"]
        I1["Agents Claim Substrate Tasks via MCP"]:::internal
        I2["Agent Output Governed by Node Policies"]:::internal
        I3["Upstream Vision Revisions Invalidate Dependent Tasks"]:::internal
    end

    Phase0 --> Transition --> Phase2Plus
```

### Bootstrap Phasing Plan

1. **Phase 0 (Current Baseline & Hypothesis De-risking):**
   - The project is designed and bootstrapped using conventional development environments, standard Git workflows, and manually maintained documentation (`vision.md`, `strategic-planning-backlog.md`).
   - Early hypothesis de-risking: Execute Spike 0 (in-memory graph-bounded retrieval vs. competent agentic code retrieval on coding tasks) and early decomposition prompt tests (H-4) to establish directional confidence before committing to Phase 1 infrastructure.
   - Scope is intentionally constrained: build only the storage layer, Git connection, basic ingestion parser, and read MCP server.
2. **Phase 1 Transition (Dogfooding Activation):**
   - Upon completing Phase 1, the development team executes the first real-world ingestion: uploading the project's own documentation files into the Git document ledger.
   - The decomposition pipeline breaks the technical vision into atomic requirement nodes.
   - Verification test: developers and agents retrieve context for Phase 2 implementation tasks using the substrate's own MCP server.
3. **Phase 2+ (Substrate-Governed Evolution):**
   - All subsequent feature backlogs, bug tracking, and architectural enhancements are tracked as nodes in the substrate.
   - When an agent is tasked with writing code for the substrate, it must query its context envelope from the substrate itself.

---

## 4. Minimum Viable Demonstration (MVD) Acceptance Test Scripts

### Phase 1 MVD: Ingestion, Git Anchoring, Decomposition, and Read Context

- **Preconditions:** Fresh PostgreSQL instance with schema applied; local Git repository configured.
- **Test Procedure:**
  1. Submit a multi-page Markdown specification (e.g., `vision.md`) to `POST /api/v1/documents/ingest`.
  2. Verify that the document is committed to the Git document repository, yielding a verifiable commit SHA and blob hash.
  3. Verify that the decomposition pipeline extracts $\ge 10$ distinct requirement nodes with valid character span offsets pointing into the source text.
  4. Human reviewer approves the staged nodes via `POST /api/v1/staging/approve`.
  5. Connect an MCP client (e.g., Claude Code or test harness) and invoke:

     ```json
     {
       "tool": "get_context_envelope",
       "arguments": {
         "target_node_id": "REQ-002",
         "depth": 2
       }
     }
     ```

  6. Verify that the response returns the target node, its ancestor requirements, immediate sibling constraints, and source text snippets within $< 100\text{ ms}$.

### Phase 2 MVD: Bounded Mutation, Governance Enforcement, and Clean Rollback

- **Preconditions:** Phase 1 graph loaded; node `REQ-005` set to `AUTONOMOUS_ELABORATION`; node `REQ-006` set to `HUMAN_REVIEW_REQUIRED`; authenticated agent session initialized with verified identity (`agent_instance_id: "agent-runner-42"`).
- **Test Procedure:**
  1. External agent calls `propose_node_mutation` targeting `REQ-005` to create three child sub-tasks, passing bearer authorization credentials.
  2. Verify immediate database write, creation of valid directed edges, and audit ledger entry capturing verified `agent_instance_id` and auth token fingerprint (Invariant I-7).
  3. External agent attempts to introduce an invalid circular constraint edge targeting `REQ-005`; verify that database-level DAG cycle constraints reject the mutation with error code `ERR_GRAPH_CYCLE_DETECTED`.
  4. External agent calls `propose_node_mutation` targeting `REQ-006` to modify requirement text.
  5. Verify that the mutation is intercepted, placed in `PENDING_REVIEW`, attributed to the agent identity, and no live graph edges are updated.
  6. Administrative user triggers rollback for the agent session:

     ```json
     {
       "command": "revert_mutation_batch",
       "arguments": {
         "batch_id": "BATCH-2026-001"
       }
     }
     ```

  7. Verify that all three child sub-tasks under `REQ-005` are reverted to historical snapshot without leaving orphan edges or corrupting table state.

### Phase 3 MVD: Automated Invalidation Cascading

- **Preconditions:** Hierarchical graph with top-level requirement `REQ-001` linked to 5 downstream specifications and 12 implementation tasks.
- **Test Procedure:**
  1. User updates `REQ-001` via supervisory portal, modifying a core constraint.
  2. Background cascade job traverses downstream dependency edges (`CONSTRAINED_BY`, `FULFILLS`).
  3. Verify that all 5 child specifications and 12 implementation tasks transition to status `NEEDS_REVERIFICATION`.
  4. External agent attempts to complete an associated task via MCP; verify that the gateway blocks task completion with an error requiring upstream reverification.

---

## 5. Architectural Evaluation Spikes & Technical Investigations

### Spike 0: Graph-Bounded Context Envelopes vs. Multi-Tool Agentic Retrieval Baseline (Hypothesis H-1 De-risking)

- **Context:** Hypothesis H-1 (graph-bounded context envelopes significantly outperform code-level retrieval for preserving architectural invariants) is the foundational scientific bet of TKS. While Microsoft GraphRAG (2024) demonstrated multi-hop reasoning gains for text summarization, evaluating requirement graphs solely against naive flat-vector search is a strawman: modern 2026 agentic workflows (e.g., Cursor, Claude Code) already employ multi-tool retrieval (file inspection, symbol grep, AST code exploration). The genuine scientific test is whether supplying a graph-bounded requirement context envelope significantly reduces contract violations compared to an agent equipped with state-of-the-art code-level tools but lacking topological requirement provenance. Phase 1 infrastructure must not proceed without early directional signal.
- **Experimental Setup & Scoping:**
  - Throwaway prototype testable in days, requiring zero production database setup.
  - In-memory graph representation with a hand-curated requirement tree (~50–100 nodes) modeling a realistic modular software component with explicit hierarchical constraints and sibling invariants.
  - Standard commodity embedding model (e.g., `text-embedding-3-small`) and LLM coding agent (e.g., Claude 3.5 Sonnet / GPT-4o).
- **Evaluation Conditions:**
  - *Condition A (Competent Multi-Tool Agentic Baseline):* External agent equipped with standard code-level tooling (file read/grep, AST-based symbol navigation, and semantic search over flat documentation) operating without topological requirement graph context.
  - *Condition B (Topological Context Envelope):* The same agent provided with a graph-bounded context envelope (target task + ancestor requirements + sibling architectural constraints and non-functional rules).
- **Measurement:** Rate of invariant violations (missed architectural contracts, violated interfaces, dropped non-functional constraints) across $\ge 20$ controlled synthetic coding tasks.
- **Decision Thresholds:**
  - $\ge 30\%$ reduction in constraint violations provides strong directional greenlight for Phase 1 construction (reflecting meaningful intent preservation against a competent baseline).
  - $15\% - 29\%$ reduction indicates partial advantage; refine envelope assembly logic and narrow domain scope before full build.
  - $\le 0\%$ or non-significant difference signals failure of H-1 premise; halts Phase 1 build and triggers immediate strategic re-evaluation.

### Spike 1: Graph Storage & Query Strategy in PostgreSQL

- **Context:** SQL/PGQ (SQL:2023 Part 16) was reverted from the PostgreSQL 19 release cycle due to design, catalog stability, and security concerns. Native SQL/PGQ support may reappear in a future major release (earliest PostgreSQL 20). The project requires an alternative graph query approach for initial phases. Re-evaluate native SQL/PGQ feasibility when PostgreSQL 20 reaches beta.
- **Options Under Evaluation:**
  1. **Apache AGE (openCypher Extension):**
     - *Pros:* Rich Cypher query syntax, native graph storage model, optimized for multi-hop graph traversals.
     - *Cons:* External C extension; compatibility currently spans PostgreSQL 11–18 (with PG 19 support pending stable release); added deployment complexity.
  2. **Standard Recursive CTEs (`WITH RECURSIVE` on Relational Schema):**
     - *Pros:* Zero external dependencies, runs on any vanilla PostgreSQL version, completely stable, fully covered by standard ACID guarantees.
     - *Cons:* More verbose SQL queries for complex path matching; performance requires careful indexing on adjacency tables.
  3. **Hybrid Adjacency List with JSONB Path Materialization:**
     - *Pros:* Extremely fast ancestor-lookup queries; simple table structure (`nodes`, `edges`).
     - *Cons:* Higher write amplification on deep graph hierarchy reorganizations.
- **Evaluation Criteria:** Simplicity, operational reliability, query latency on $k$-hop traversals ($k \le 4$), and ease of initial setup.

### Spike 2: State Versioning Mechanism (Simplicity vs. Bitemporality)

- **Context:** The vision mandates auditability, reversibility, and point-in-time state reconstruction (Invariant I-2). Full bitemporal data modeling (valid time + transaction time across all entities and edges) introduces extreme schema and query complexity.
- **Options Under Evaluation:**
  1. **Append-Only Event Ledger with Materialized Current State:**
     - State mutations write an immutable event record to `audit_ledger`. Live graph tables maintain the current snapshot, updated within the same transaction.
     - *Pros:* Conceptual simplicity, fast read performance on current graph, straightforward rollback via inverse events.
  2. **System-Versioned Temporal Tables (`system_time`):**
     - Using standard PostgreSQL range types and historical archive tables triggered on update.
     - *Pros:* Standardized query interface using timestamp ranges.
  3. **Full Bitemporal Property Graph:**
     - Tracking both `asserted_at` and `effective_at` timestamps on every node and edge.
     - *Pros:* Complete academic auditability across retrospective corrections.
     - *Cons:* Severe query verbosity, complex recursive graph traversal over bitemporal edges, excessive initial engineering burden.
- **Recommendation:** Adopt the **Append-Only Event Ledger** for Phase 1 and 2 to honor the "Start Small" directive while strictly preserving reversibility.

### Spike 3: Git-PostgreSQL Storage Coupling Architecture

- **Context:** Text specification documents are versioned in Git, while metadata, graph topology, and requirement mappings live in PostgreSQL.
- **Design Challenges:**
  1. **Identity & Referencing:**
     - Anchor documents to Git commit hashes and blob SHAs (`git hash-object`).
     - Store document metadata (file path, commit hash, author, timestamp) in PostgreSQL table `document_artifacts`.
  2. **Span Stability Across Revisions:**
     - Character offset spans (`char_start`, `char_end`) become invalid if a source document is edited upstream.
     - *Mitigation strategy:* Pin extracted requirements strictly to an immutable Git commit/blob hash. When a document revision is ingested via a new commit, run a diff-based span relocation algorithm or flag affected requirements for re-annotation.
  3. **Retrieval Interface:**
     - REST API provides raw text retrieval by resolving the Git reference (`git cat-file` or `libgit2` bindings) for external agents and UI rendering.

### Spike 4: Assisted Decomposition Prompting & Deterministic Span Re-Anchoring

- **Context:** Automated requirement extraction from unstructured Markdown documents must produce coherent, atomic graph nodes with high precision. External research (e.g., LLMStructBench 2026) and practitioner consensus indicate that while LLMs reliably generate schema-compliant JSON, they remain fundamentally challenged by character-level coordinate precision due to subword tokenization boundaries, UTF-8 multibyte characters, and whitespace normalization artifacts. Relying solely on raw LLM offsets risks violating Invariant I-4 (Cryptographic Source Anchoring).
- **Prior Art to Analyze:**
  - **SARA:** Analyzes requirements as knowledge graphs using Git and Markdown. Study its document parsing conventions and edge taxonomies.
  - **Proj-Theseus:** Multi-level requirement traceability models; inspect its schema conventions for specification decomposition.
- **Evaluation Tasks & Deterministic Fallback Architecture:**
  - Benchmark few-shot prompts against diverse Markdown structures (tables, bulleted lists, RFC 2119 keyword sections).
  - Measure LLM character span precision/recall against ground-truth source offsets across UTF-8 multibyte text and Markdown formatting.
  - **Deterministic Span Re-Anchoring Fallback:** If direct LLM-produced character offsets fall below the CAL-H4 threshold ($\ge 95\%$), implement a deterministic post-processing pipeline. Under this pattern, the LLM outputs extracted verbatim requirement text snippets and structural section headers, and a deterministic text-search / fuzzy byte alignment algorithm (e.g., Myers diff, Boyer-Moore, or localized string search) maps the extracted snippet back to the source document byte stream to re-derive verified `[char_start, char_end]` coordinates. This preserves Invariant I-4 without requiring LLMs to calculate token-to-byte offsets.

### Spike 5: Model Context Protocol (MCP) Tool Schema Design

- **Context:** External agents interact with the substrate through a standardized MCP server with stateless identity verification.
- **Proposed Tool Definitions:**
  - `get_context_envelope(node_id: string, depth: int, include_vectors: bool)`: Returns structured context JSON including target node, ancestor requirements, linked constraints, and sibling context.
  - `propose_node_mutation(target_node_id: string, mutation_type: string, payload: object)`: Submits candidate changes accompanied by caller identity credentials; returns status (`COMMITTED` or `PENDING_REVIEW`).
  - `query_requirements(query_text: string, limit: int)`: Performs hybrid vector + keyword search over requirement nodes.
  - `get_document_span(document_hash: string, start_offset: int, end_offset: int)`: Retrieves verified source text from Git backend.
  - `revert_mutation_batch(batch_id: string)`: Administrative tool to roll back a specific batch of mutations.
  - `revert_agent_session(agent_instance_id: string, since_timestamp: string)`: Administrative tool to roll back all mutations executed by a specific agent instance.

### Spike 6: Agent Identity, Workload Authentication & Semantic Defenses (Invariant I-7)

- **Context:** Invariant I-7 mandates that all mutations submitted through the Integration Gateway are attributable to a verified external identity recorded in the audit ledger. The MCP July 2026 specification standardizes on OAuth 2.1 with PKCE, and the Agent Identity Working Group is establishing workload identity federation and DPoP (Demonstrating Proof-of-Possession). The architecture must also defend against policy-compliant semantic corruption and circular dependencies.
- **Investigation Areas:**
  1. **Stateless Identity & Token Verification:**
     - Validate incoming OAuth 2.1 / DPoP tokens at the gateway without persisting session state.
     - Extract verified caller claims (`caller_id`, `caller_type: HUMAN | AGENT_WORKLOAD`, `agent_instance_id`, `workload_issuer`).
  2. **Audit Ledger Identity Attribution Schema:**
     - Design immutable audit columns (`actor_id`, `actor_type`, `token_fingerprint`, `agent_runtime_metadata`) to guarantee cryptographic non-repudiation.
  3. **Structural DAG Cycle Prevention:**
     - Enforce database-level cycle detection on structural edges (`CONSTRAINED_BY`, `DERIVED_FROM`) via PostgreSQL triggers or CTE validation to deterministically prevent hallucinating agents from introducing cyclic dependencies.
  4. **Instance-Scoped Blast Radius Containment:**
     - Tooling to isolate, quarantine, and batch-revert all mutations from a compromised or misbehaving agent instance without disrupting concurrent valid contributions from other agents or humans.

### Spike 7: Node Lifecycle State Management & Context Envelope Pruning (Phase 2+ Investigation)

- **Context:** As a software system evolves, requirements and architectural decisions are superseded, deprecated, or archived. Without explicit lifecycle state filtering, historical or superseded requirement nodes remain connected in the graph topology, causing context envelope dilution, token waste, and potential agent hallucination.
- **Investigation Areas:**
  1. **Lifecycle State Taxonomy:** Model node states (`ACTIVE`, `SUPERSEDED`, `ARCHIVED`) within typed JSONB metadata attributes, ensuring backward compatibility with Phase 1 nodes.
  2. **Traversal Filter Semantics:** Update CTE and graph traversal queries in `get_context_envelope` to filter out `ARCHIVED` and `SUPERSEDED` nodes by default while allowing historical and audit queries to traverse them explicitly.
  3. **Automated Supersession Cascades:** Design the operational mechanism for marking a requirement `SUPERSEDED` when a replacing requirement node is approved, cleanly repointing dependent edges to avoid breaking topological invariants.
- **Evaluation Criteria:** Latency impact on traversal queries when filtering by lifecycle state, and prevention of deprecated constraints bleeding into agent prompt contexts.

---

## 6. Quantitative Operational Targets & Metric Calibrations

While the technical vision defines qualitative hypotheses, this backlog establishes the specific numerical targets used for calibration and validation during execution:

| Metric Identifier | Metric Name | Strategic Calibration Target | Validation Horizon |
| --- | --- | --- | --- |
| **SLA-1** | Micro-Reflex Graph Traversal Latency | $< 50\text{ ms}$ for $k \le 3$ hop topological queries | Phase 1 Benchmark |
| **SLA-2** | Context Envelope Assembly Latency | $< 100\text{ ms}$ at $10^5$ nodes in PostgreSQL | Phase 2 Benchmark |
| **CAL-H1** | Contract Violation Reduction (Hypothesis H-1) | $\ge 40\%$ fewer architectural violations vs. competent agentic baseline | Phase 2 Controlled Trial |
| **CAL-H2** | Human Review Overhead Reduction (Hypothesis H-2) | $\ge 50\%$ reduction in supervisory review time per feature | Phase 3 User Study |
| **CAL-H3** | Single-Engine Scalability Bound (Hypothesis H-3) | Sustained $< 100\text{ ms}$ query latency at $10^6$ nodes | Phase 3 Stress Test |
| **CAL-H4** | Extraction Fidelity Benchmark (Hypothesis H-4) | $\ge 95\%$ precision/recall on atomic requirement spans | Phase 1 Ingestion Eval |

### Graduated Evaluation Framework & Calibration Interpretation

In alignment with the Technical Vision's graduated response model (§7), empirical evaluation outcomes are evaluated across three operational tiers to prevent premature program termination when partial validation occurs:

| Metric Identifier | Target Validation Band (Full Success) | Graduated Scope Adjustment Band (Partial Validation) | Falsification / Kill Band (Termination / Pivot) |
| --- | --- | --- | --- |
| **CAL-H1** (Constraint Preservation) | $\ge 40\%$ violation reduction vs. competent agentic baseline | **$20\% - 39\%$ reduction:** Narrow domain to deeply coupled architectures or modular microservices; refine envelope filtering and hybridize topological envelopes with local code search. | $\le 0\%$ or non-significant improvement vs. competent agentic baseline (Triggers Kill #2). |
| **CAL-H2** (Supervisory Review Overhead) | $\ge 50\%$ review time reduction | **$25\% - 49\%$ reduction:** Streamline supervisory UI staging workflows and enrich topological blast-radius visualizations. | $\le 0\%$ reduction (supervisory graph review equals or exceeds diff review time; Triggers Kill #1). |
| **CAL-H3** (Single-Engine Scalability) | Sustained $< 100\text{ ms}$ at $10^6$ nodes | **$< 100\text{ ms}$ at $10^5$ nodes, degrading at $10^6$:** Satisfies small-to-mid enterprise repos; apply read-replica offloading, partition audit ledger, and optimize CTE indexes. | $> 500\text{ ms}$ latency at $\le 10^5$ nodes despite index optimization (Triggers Kill #3). |
| **CAL-H4** (Assisted Ingestion Fidelity) | $\ge 95\%$ precision/recall on spans | **$80\% - 94\%$ precision/recall:** Engage deterministic span re-anchoring post-processor (fuzzy byte alignment against source) to correct offset drift; enforce structured Markdown specification templates and mandatory human-in-the-loop staging corrections. | $< 60\%$ precision/recall or severe span hallucination despite deterministic re-anchoring (Triggers Kill #1). |

---

## 7. Operational Workflow Specifications & Interaction Sequences

```mermaid
%%{init: {
  'theme': 'base',
  'themeVariables': {
    'darkMode': true,
    'background': '#161922',
    'mainBkg': '#1e2230',
    'nodeBorder': '#434c5e',
    'textColor': '#e2e8f0',
    'fontFamily': 'ui-sans-serif, system-ui, sans-serif',
    'lineColor': '#8892b0',
    'primaryColor': '#1b3528',
    'primaryBorderColor': '#73c991',
    'secondaryColor': '#1d2c44',
    'secondaryBorderColor': '#61afef',
    'gateColor': '#2e271a',
    'gateBorder': '#e5c07b'
  }
}}%%
sequenceDiagram
    autonumber
    actor Engineer as Human Engineer / Lead
    participant Git as Git Document Repository
    participant API as Ingestion REST API
    participant LLM as Assisted LLM Parser
    participant DB as PostgreSQL Substrate
    participant MCP as MCP Gateway
    actor Agent as External Autonomous Agent

    Note over Engineer, DB: 1. Document Ingestion & Decomposition Loop
    Engineer->>API: Upload specification (Markdown)
    API->>Git: Commit raw document (mint commit & blob SHA)
    Git-->>API: Return Git reference
    API->>LLM: Stream document text with decomposition prompt
    LLM-->>API: Return candidate requirement nodes with text spans
    API->>DB: Write to staging table (status: PENDING_STAGING)
    Engineer->>DB: Review, adjust metadata, approve staged nodes
    DB->>DB: Commit verified nodes & edges to live graph + audit log

    Note over Agent, DB: 2. Agent Execution & Governed Mutation Loop
    Agent->>MCP: Request context envelope (target: Task-101)
    MCP->>DB: Execute graph traversal + vector neighbor search
    DB-->>MCP: Return bounded ancestor & sibling envelope
    MCP-->>Agent: Deliver context envelope JSON
    Agent->>Agent: Execute local synthesis (code, sub-specs)
    Agent->>MCP: Submit proposed mutation with caller auth token
    MCP->>MCP: Validate caller identity (Invariant I-7)
    MCP->>DB: Check target node governance_policy & DAG acyclicity
    alt Policy = AUTONOMOUS_ELABORATION & Valid Acyclic Topology
        DB->>DB: Commit mutation with agent identity attribution to audit ledger & update graph
        DB-->>MCP: Mutation accepted (COMMITTED)
        MCP-->>Agent: Success
    else Policy = HUMAN_REVIEW_REQUIRED
        DB->>DB: Park mutation in staging table with agent identity (PENDING_REVIEW)
        DB-->>MCP: Mutation queued for review
        MCP-->>Agent: Queued for human approval
    else Structural Cycle Detected
        DB-->>MCP: Reject mutation (ERR_GRAPH_CYCLE_DETECTED)
        MCP-->>Agent: Rejected: cyclic dependency violation
    end
```

### Detailed Sequence Description

1. **Upload & Versioning:** The human engineer uploads a Markdown specification to the REST API. The document is immediately committed to the underlying Git repository, preserving byte-for-byte fidelity and generating cryptographic commit and blob identifiers.
2. **Parsing & Character Span Tagging:** The ingestion service formats the document for the decomposition LLM prompt. The model segments the text into atomic requirement items, returning extracted text and candidate spans. A deterministic post-processing step matches the extracted requirement text against the source document byte stream, resolving verified character coordinate spans (`[start, end]`) pointing to the immutable Git source blob.
3. **Staging & Human Verification:** Parsed requirements enter a staging state. The engineer reviews the extracted nodes, adjusts governance flags, and approves insertion.
4. **Graph Materialization:** Approved items are committed to PostgreSQL, writing records to the live graph topology, embedding tables, and append-only audit ledger.
5. **Context Request:** External agents connecting over MCP request context for assigned tasks. The substrate executes a directed graph query retrieving parent requirements, linked architectural constraints, and vector-similar contextual nodes.
6. **Execution, Identity Validation & Governed Mutation:** The agent runs locally, then submits proposed graph updates via MCP presenting its workload credentials. The MCP gateway statelessly validates caller identity (Invariant I-7). The substrate verifies structural integrity (rejecting circular dependency chains) and inspects the target node's governance policy. If autonomous elaboration is authorized and structural invariants hold, the change commits immediately to the graph and immutable audit ledger with verified caller attribution; if human review is required, it is parked in a review queue for human sign-off; if a cyclic dependency or invariant violation is detected, it is deterministically rejected.
