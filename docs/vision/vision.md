# Technical Vision: The Knowledge Substrate for Agentic Software Engineering

---

## 1. The Problem Space & Current Paradigm Limitations

Autonomous AI agents are increasingly capable of generating functional software modules, yet the medium through which software engineering is organized—unstructured text documents, linear source files, and flat issue trackers—was engineered for human visual scanning, not distributed machine reasoning. Current industry paradigms exhibit structural failure modes that prevent autonomous agents from operating reliably at enterprise scale:

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
    classDef tertiary fill:#1d2c44,stroke:#61afef,stroke-width:1.5px,color:#e4f0fc;

    F1["The Context Window Collapse<br/><i>(Code-Level Context Blindness)</i>"]:::primary
    F2["The Specification Drift Trap<br/><i>(Decoupled Documentation)</i>"]:::primary
    F3["The Monolithic Diff Dilemma<br/><i>(Unverifiable Agent Output)</i>"]:::primary
    F4["The Procedural & Topological Void<br/><i>(Missing Process & Gap Blindness)</i>"]:::primary
    F5["The Planning Desert & Cognitive Rail Vacuum<br/><i>(Strategic Elaboration Blindness)</i>"]:::primary

    C1["Loss of Architectural Invariants<br/><i>(Local optimization violates global contracts)</i>"]:::tertiary
    C2["Silent Requirement Decay<br/><i>(Code mutates away from baseline intent)</i>"]:::tertiary
    C3["Human Supervisory Exhaustion<br/><i>(Blind rubber-stamping of massive PRs)</i>"]:::tertiary
    C4["Ungoverned Operations & Structural Disconnects<br/><i>(Skipped procedural vetting & unlinked cross-layer workflows)</i>"]:::tertiary
    C5["Ungrounded Architecture & Schema Violations<br/><i>(Hallucinated web patterns & foreign key breakages)</i>"]:::tertiary

    F1 --> C1
    F2 --> C2
    F3 --> C3
    F4 --> C4
    F5 --> C5
```

### The Context Window Collapse (Code-Level Context Blindness)

Contemporary AI-native engineering environments assemble context dynamically via Abstract Syntax Tree (AST) parsing, type-system traversal, symbol grepping, and iterative file exploration. While these techniques significantly surpass naive vector similarity, code-level context assembly remains structurally blind to the intent and requirement layer. Code-level tools can discover what existing code *does*, but they cannot infer what the software is *supposed to do*, why specific architectural invariants exist, or which upstream business requirements govern a component. Lacking a topological requirement substrate, agents assemble context that is locally coherent at the syntax level but blind to systemic architectural contracts, generating code that passes unit tests while violating high-level system invariants.

### The Specification Drift Trap (Decoupled Intent)

In modern delivery lifecycles, project vision documents, product requirements (PRDs), and architectural specifications are write-once artifacts. Once engineering starts, implementation rapidly decouples from documentation. When autonomous agents operate within this disconnected paradigm, every development loop compounds intent decay. Lacking an immutable, queryable link between functional requirements and execution tasks, agents introduce unintended scope, duplicate capabilities, and silently drop edge-case requirements.

### The Monolithic Diff Dilemma (The Human Review Bottleneck)

As agent synthesis speed outpaces human reading capacity, human supervisors face an unscalable verification burden: reviewing massive, multi-file code diffs generated in seconds. Humans cannot verify whether thousands of lines of synthesized code adhere to twenty subtle non-functional constraints, cross-cutting security policies, and accepted product requirements. Software engineering risks shifting from an intentional design discipline into an opaque quality-assurance bottleneck. Crucially, this verification crisis cannot be resolved by simply displacing cognitive fatigue from code diffs to hundreds of isolated, atomized graph candidate nodes; human oversight must be grounded in tractable supervisory granularity.

### The Procedural & Topological Void (Absence of Process and Cross-Layer Gap Blindness)

Software engineering is fundamentally more than a static cascade of functional requirements; it is equally governed by operational procedures, engineering workflows, and cross-layer architectural coherence. When an engineering task requires introducing a new third-party dependency, for example, execution is governed not only by functional utility, but by mandatory institutional procedures: vetting licensing compatibility, evaluating commit frequency and maintenance health, verifying CVE management history, and auditing supply chain blast radius. In flat issue trackers and code-centric agent environments, these operational procedures, checklists, and institutional policies are completely severed from execution context.

Furthermore, existing paradigms lack the topological reachability to detect *architectural gaps*. When traversing a user workflow, engineering teams expect presentation-layer actions to link systematically to corresponding functional specifications, domain logic, and service contracts. Current tooling cannot determine whether those cross-layer links exist, whether critical workflows were ever formally considered, or where systemic omissions lie. Autonomous agents generate code in a procedural and topological vacuum, leaving critical organizational processes unexecuted and architectural discontinuities unflagged until production failure.

### The Specification Void & The Cold-Start Adoption Barrier (The Upstream Intent Prerequisite)

Autonomous agents cannot align with intent if no externalized intent exists. Today, many engineering organizations lack pristine requirement graphs, relying instead on ephemeral chat threads, fragmented issue tickets, and implicit developer knowledge. TKS is fundamentally an intent provenance substrate, not an autonomous mind-reader: it solves the structural problem of anchoring agent execution to specifications, but it presupposes that engineering teams possess—or are willing to articulate—externalized intent.

However, real-world repositories are rarely completely void of intent; rather, intent is embedded across structured brownfield artifacts: Architecture Decision Records (ADRs), API schemas (OpenAPI, AsyncAPI, Protobuf), and Behavior-Driven Development (BDD) feature files. TKS directly confronts the cold-start adoption barrier through **Brownfield Intent Scaffolding**: leveraging zero-token mechanical parsers to extract baseline requirement, decision, and specification nodes directly from these existing artifacts. This dramatically compresses Time-to-First-Value (`CAL-TTFV`) without compromising the foundational boundary: TKS operates on externalized intent artifacts, avoiding the fragility and hallucination risks of speculative code-level reverse-engineering.

### The Planning Desert & Cognitive Rail Vacuum (Strategic Elaboration Blindness)

When autonomous agents transition from executing narrow leaf tasks to participating in strategic planning, epic decomposition, or phase elaboration, passive context retrieval models fail completely. If an agent is provided only a sparse summary node stripped of governing architectural invariants, prior design decisions, physical schema contracts, and established codebase blueprints, it finds itself in an "information desert."

Deprived of normative rails, commodity LLMs inevitably fall back to their broad internet pre-training weights. They hallucinate ungrounded architectural components, guess brittle regex patterns, and formulate mutations that violate relational schemas (such as treating entity references as arbitrary raw strings). Crucially, the substrate's objective is not to transform an LLM into an unguided, autonomous engineering executive inventing high-level roadmaps from an empty prompt; strategic planning remains a deliberative, human-led endeavor. Rather, the substrate must supply **cognitive planning rails**: structured multi-axis planning context that bounds and grounds human-directed task elaboration, ensuring that agent proposals adhere strictly to architectural invariants, schema contracts, and implementation precedents.

### The Build-vs-Leverage Imperative (Operational Intent Substrate vs. Code-Level Orchestration & Retrospective ALM)

The contemporary engineering ecosystem presents two divergent paradigms, neither of which addresses the foundational intent-fidelity gap:

1. **AI-Native Development Tools (Thin Code Orchestration):** AI-native IDEs (e.g., Cursor, Windsurf, Devin Desktop) and terminal-first agentic harnesses (e.g., Claude Code, Codex CLI) operate as thin orchestration layers over raw code artifacts. Through Language Server Protocol (LSP) integration, Abstract Syntax Tree (AST) search, and multi-tool agentic retrieval, they excel at discovering and manipulating existing syntax. Even when augmented with automated codebase graph generators (e.g., Graphify, AST knowledge graphs), code-derived graphs merely reflect current syntactic implementation; they remain structurally blind to governing business intent, non-functional requirements, architectural contracts, and organizational procedures. Ad-hoc Model Context Protocol (MCP) bridges to issue trackers or flat documentation stores offer only fragmented, unverified context without topological coherence or mutation governance.
2. **Legacy ALM Platforms (Retrospective Compliance):** Traditional Application Lifecycle Management suites (e.g., IBM DOORS Next, Jama Connect, Siemens Polarion) and issue-tracker traceability matrix plugins treat requirements traceability as a human-facing, retrospective reporting and compliance exercise. They were never designed to serve as an active, sub-second operational substrate for machine agents requiring topological graph traversals, co-located vector search, and transactional mutation governance.

TKS occupies the critical gap between these two extremes: it is neither a code-editing assistant nor a compliance database. It is a purpose-built **knowledge and intent provenance substrate** that anchors external agent reasoning directly to a version-governed property graph of requirements, operational processes, and architectural relationships, supplying the institutional guidance and topological coherence that code-level tooling structurally lacks.

---

## 2. The North Star Vision

### Core Vision Statement

> **Engineer an agent-agnostic knowledge substrate that anchors external AI agents and human teams to a unified, version-governed property graph, establishing end-to-end traceability and bidirectional coherence across institutional knowledge, operational processes, requirements, and executable code.**

### The Core Metaphor: The Substrate as Organization, The Agent as Employee

To crystallize the architectural purpose and operational scope of The Knowledge Substrate, consider an organizational mental model:

* **The Knowledge Substrate is the Organization and its Institutional Memory:** In an engineering enterprise, the organization embodies collective institutional memory, architectural guidelines, standard operating procedures (SOPs), compliance checklists, domain knowledge, regulatory policies, and topological requirement maps. It defines *how* the enterprise builds software, *what* architectural invariants must be preserved, and *which* procedures must be followed. The substrate is durable, authoritative, version-governed, and structured.
* **The LLM Agent is the Employee / Knowledge Worker:** External LLMs play the role of employees operating within the firm. The LLM is not built *into* the substrate's operational engine; rather, it consults the organization. It is guided through the operational landscape by the organization's processes, procedures, checklists, and topological context envelopes, which direct it on how to navigate the engineering terrain and what standards must be satisfied.
* **The Agent as Creative Generator:** Crucially, while the substrate provides the governing scaffolding and institutional guardrails, the LLM remains the *creative generator*—the flexible engine of synthesis, code construction, and novel problem-solving. By decoupling authoritative institutional scaffolding (the substrate) from creative cognitive labor (the external agent), the system empowers autonomous agents to synthesize complex, novel solutions without violating institutional norms, dropping compliance checks, or wandering into architectural drift.

### Core Philosophy: Minimal LLM Reliance & Maximum Mechanical Reliance

To ensure scalability, economic sustainability, and operational determinism, TKS adheres strictly to a foundational principle: **minimal LLM reliance and maximum reliance on mechanical processes**.

* **Minimizing Expensive Output Tokens:** In modern LLM economics, output tokens are substantially more expensive in latency and cost than input tokens. While document ingestion context cannot be compressed arbitrarily, the LLM's output burden must be radically minimized. TKS achieves this by delegating the initial 80/20 (or greater) of parsing, structural segmentation, and boundary extraction to deterministic, zero-cost mechanical tools (CommonMark AST parsers, deterministic regex extractors, and recursive relational queries). LLM inference is invoked only when mechanical rules encounter semantic ambiguity, and then solely to emit compact relational tuples rather than echoing large text blocks.
* **Mechanical Scaffolding over Cognitive Guesswork:** Invariant validation, dependency graph reachability, foreign key integrity, and ancestral lineage checking are enforced entirely by deterministic relational constraints and transactional CTEs in PostgreSQL, completely bypassing probabilistic LLM judgment for structural verification.

### Key Capabilities

1. **Unified Graph-Relational Substrate:** A PostgreSQL storage layer combining native property graph structures with vector embeddings, modeling vision, requirements, specifications, tasks, and artifacts as a strongly typed, navigable topology with strict relational edge integrity.
2. **Document Provenance, Multi-Tier Analysis & Contradiction-First Quality Ranking:** Ingestion of text-based specifications and structured engineering artifacts (OpenAPI/AsyncAPI/Protobuf schemas, Architecture Decision Records, BDD feature files) into a Git-backed, content-addressed document store coupled to PostgreSQL. Operates across a strictly ordered multi-tier analysis pipeline: Tier 1 (Mechanical CommonMark AST parsing achieving 80/20 structural extraction and byte spans at zero token cost, with a principled structural heading taxonomy—classifying top-level structural containers, Level-1/Level-2 headings, or frontmatter-annotated sections as `REQUIREMENT` anchors rather than relying on brittle keyword regexes); Tier 2 (Local LLM pre-analysis if available and requested, evaluating contradiction risk against active approved requirements and preliminary quality markers); Tier 3 (User-invoked commercial LLM escalation via open protocols for complex classification); and Tier 4 (Human-in-the-loop review). Candidate requirements are evaluated by a Quality Index that places **Contradiction Risk against existing approved nodes first**, followed by ambiguity and testability scores. Crucially, candidate nodes are surfaced within their native document narrative hierarchy using inline visual badges and callouts to preserve supervisory context, with flat risk-sorted views provided as a secondary triage view. Supports incremental multi-document ingestion and a formal Document Re-Ingestion Delta Reconciliation Contract that preserves active graph topology, elaborated child tasks, and downstream verification edges through three-way delta reconciliation against active state.
3. **Open Protocol Integration Gateway:** Standardized Model Context Protocol (MCP) and REST interfaces enabling external agent runtimes to retrieve bounded context envelopes and submit proposed graph mutations. Enforces strict protocol conformance across tool schemas and message contracts to guarantee seamless client tool handshakes.
4. **Immutable Audit Ledger & Historical Reversibility:** A clear operational distinction between active, approved entities and isolated drafts. Approved requirements, specifications, and tasks are strictly immutable and point-in-time reconstructible via an append-only audit ledger. Intermediate candidate mutations undergo lifecycle compaction upon approval, collapsing exploratory draft churn into canonical audit milestones to prevent ledger bloat while preserving drafting rationale.
5. **Attribute-Based Node Governance & Tactical Decision Capture:** Granular per-node governance attributes that dictate whether an entity is open for autonomous agent elaboration or strictly gated by human authorization. When planning or implementation agents make inconsequential tactical decisions (such as parameter ranges or CLI flags), those choices are automatically captured back into the substrate with full audit attribution, while permission scoping prevents unauthorized upward drift into vision-level requirements.
6. **Self-Referential Bootstrapping & Stratified Model Downgrading Verification:** The capability of the Knowledge Substrate to manage its own development lifecycle via a phased transition (Phase 1 read-only self-hosting, Phase 2 autonomous self-evolution). Grounded by a formal performance benchmark stratified into economic tiers: Tier 1 demonstrates commercial utility model arbitrage—proving that an external agent powered by high-speed, cost-effective commercial models (e.g., Claude 3.5 Haiku, Gemini Flash, GPT-4o-mini, delivering a 10x–20x cost reduction) guided by TKS cognitive rails successfully implements substantial new feature modules in realistic codebases without architectural contract violations on the Real-World Feature Extension Benchmark (Phase 5 Gate 4). Tier 2 evaluates local edge commodity 8B models (e.g., Qwen 8B) as an aspirational research evaluation in Phase N+, alongside complete self-reconstruction from scratch.
7. **Tractable Supervisory Granularity:** Structuring human verification gates around cohesive functional modules, document sections, and hierarchical batches rather than isolated relational micro-nodes. The verification interface preserves the source document's narrative flow as its primary layout, embedding rubric scores and contradiction warnings as contextual inline callouts to ensure supervisory review remains cognitively tractable.
8. **Institutional Process, Procedural Knowledge & Modular Governance Profiles:** First-class modeling of engineering processes, SOPs, compliance workflows, and verification checklists as strongly typed entities. Organizations encapsulate high-leverage institutional and architectural standards into **Modular Governance Profiles** (e.g., third-party dependency onboarding checklists, licensing compatibility rules, CVE supply-chain auditing, EU Cyber Resilience Act compliance, and architectural boundary invariants—relying on standard off-the-shelf tools for static file formatting and syntax linting). These profiles link automatically into new projects, creating a continuous **Quality Verification Loop** tying commits, tasks, and test artifacts directly to requirements and governance standards. Policy updates enforce severity classification (Breaking vs. Advisory) and blast-radius throttling with digest work-package batching to prevent supervisory alert floods across large multi-project repositories.
9. **Topological Gap Detection & Scheduled Graph Maintenance:** Algorithmic and agentic evaluation of graph reachability, layer bindings, and procedural completeness (e.g., discovering presentation actions lacking functional backend bindings). Augmented by scheduled or on-demand background audit sweeps (`tks graph audit`) that run deterministic topological linting first, invoke local LLMs second for indirect contradiction analysis, and strictly operate in a read-only advisory capacity, enqueueing candidate recommendations into a review backlog without mutating active topology.
10. **Multi-Axis Cognitive Planning Rails (`get_elaboration_context`):** A dedicated planning dossier synthesis endpoint distinct from leaf-level context envelopes. When agents participate in human-directed phase elaboration or epic decomposition, TKS delivers four cohesive informational axes: Normative Boundaries (systemic invariants), Architectural Precedents (relevant architectural decisions), Physical Schema Contracts (node/edge enums and relational constraints), and Implementation Blueprints (canonical codebase patterns).
11. **Actionable Invariant Diagnostics & Self-Healing Remediation Envelopes:** Replacement of opaque, cryptic error codes with structured remediation envelopes. When an invariant is violated (e.g., an ancestor chain missing an active `REQUIREMENT`), TKS delivers a machine-readable diagnostic specifying the broken link alongside deterministic remediation options (including deterministic ancestor promotion suggestions when authorized), empowering agents to resolve structural issues within governance rules without resorting to unauthorized database tampering.
12. **Human-Readable Document Generation & Reverse Projection:** Dynamic synthesis of human-readable documentation directly from the living property graph. External agents or users can query broad syntheses (high-level vision or architectural overviews) or targeted vertical slices (how and why a specific UI workflow operates), orchestrating an LLM to assemble graph nodes and edges into cohesive Markdown adhering to project documentation standards.
13. **External Artifact Validation & Knowledge Base Review:** Utilizing TKS as an authoritative knowledge base to validate external content presented to it (such as executive summary slide decks, architectural RFCs, or PRDs). TKS analyzes external claims against the grounded graph topology, flagging factual errors, ungrounded abstractions, and requirement contradictions.
14. **Recursive Standards & Level-Appropriate Testing Policies:** Recursive application of formal systems engineering standards (e.g., NASA, INCOSE) to TKS's own requirements graph. Enforces layered testing policies across architectural levels: tactical decisions are verified via unit tests, architectural decisions via integration tests, with test coverage rules dynamically scaled to requirement criticality.

### Operational Timescales

| Horizon | Operational Cadence | System Operation |
| --- | --- | --- |
| **Micro-Reflex** | Real-Time / Sub-Second | Graph traversal queries, vector similarity neighbor lookups, and transactional edge validation within PostgreSQL. |
| **Agent Cycle** | Iterative Execution | External agents query context envelopes via open protocols, execute local code synthesis, and submit candidate graph mutations. |
| **Scheduled Audit & Maintenance** | Scheduled / On-Demand Sweeps | Scheduled audit sweeps (`tks graph audit`) execute deterministic topological linting and optional local LLM analysis, enqueueing non-destructive recommendations into the relationship review backlog. |
| **Supervisory Review** | Deliberative Oversight | Human engineers review extracted requirement drafts in narrative document context, inspect topological impact analyses, and sign off on gated nodes. |
| **Systemic Evolution** | Strategic Progression | Strategic iteration on product roadmaps, high-level vision revisions, governance profile updates, and schema attribute extensions across the project lifecycle. |

### Conceptual Grounding

To maintain engineering precision, all structural concepts are anchored in standard software engineering and project management terminology:

| System Concept | Concrete Engineering Specification |
| --- | --- |
| **Knowledge Substrate** | The authoritative living property graph uniting graph topologies, dense vector indices (`pgvector`), and relational audit tables in an ACID-compliant PostgreSQL store. |
| **Document Ledger** | A Git-backed, content-addressed artifact store serving as an immutable historical intake ledger and baseline reference archive for text-based specifications, as well as a projection target for graph-exported artifacts. |
| **Brownfield Intent Scaffolding** | Mechanical, zero-token ingestion adapters that extract baseline requirement and architectural decision nodes from existing structured repository artifacts (OpenAPI/Protobuf schemas, ADRs, BDD feature files). |
| **Topological Context Envelope** | A directed subgraph query centered on an assigned task node, aggregating ancestor requirements, applicable procedural checklists, and sibling constraints into a bounded prompt context (`get_context_envelope`). |
| **Multi-Axis Planning Dossier** | A specialized planning envelope (`get_elaboration_context`) synthesizing normative invariants, architectural decisions, physical schema rules, and codebase precedents for bounded epic and phase elaboration. |
| **Governance Profile & Modular Policy Pack** | Reusable, modular graph clusters encapsulating high-leverage institutional, security, and architectural policies (dependency onboarding, licensing compatibility, CVE audits, EU CRA compliance, architectural boundary invariants) with severity tiers and blast-radius throttling linked systematically to project vision nodes. |
| **Quality Verification Loop** | Automated validation pipelines tying VCS commits, code artifacts, and test runs directly to leaf requirements and linked governance profile standards. |
| **Actionable Remediation Envelope** | Structured JSON diagnostic payload emitted on invariant violations, providing the precise failure point and deterministic options for structural repair. |
| **Relationship Maintenance Backlog** | An advisory-only, ranked queue of suspect or evolving graph relationships populated by scheduled or on-demand graph audits (`tks graph audit`), requiring explicit supervisory review before mutation. |
| **Tactical Decision Capture** | Automated recording of agent-specified implementation parameters (ranges, flags, types) into node attributes, bounded by upward permission scoping. |
| **Real-World Feature Extension Benchmark** | The primary empirical evaluation demonstrating that an external agent powered by a commercial utility model (Haiku/Flash/mini, 10x–20x cost reduction) guided by TKS cognitive rails implements complex new feature modules in realistic codebases without invariant violations, proving model downgrading efficiency. |
| **Self-Reimplementation Benchmark** | An aspirational post-v1.0 research evaluation (Phase N+) exploring complete self-reconstruction using constrained models. |
| **Reverse Document Projection** | Dynamic aggregation and synthesis of graph subgraphs into cohesive, human-readable Markdown documents matching project style guidelines. |
| **Level-Appropriate Verification Policy** | Rule set mapping verification rigor to architectural depth: unit tests for tactical choices, integration tests for architectural decisions, and criticality-weighted test gates. |
| **Per-Node Governance Policy** | Metadata attributes stored on individual graph nodes designating the operational authorization level (`AUTONOMOUS_ELABORATION`, `HUMAN_REVIEW_REQUIRED`, `LOCKED`). |
| **Self-Referential Engine** | The application of the Knowledge Substrate to its own codebase and lifecycle, establishing a closed feedback loop where the tool governs its own evolution. |
| **Tractable Supervisory Unit** | A cohesive, hierarchical batch of candidate graph nodes presented in source document narrative context with inline quality badges, preventing supervisory review exhaustion. |
| **Procedural Entity & Checklist Node** | Strongly typed graph nodes representing operational workflows, Standard Operating Procedures (SOPs), and compliance checklists required to authorize task completion. |
| **Topological Gap Analysis** | Structural and semantic graph traversal algorithms paired with external LLM auditing to detect architectural discontinuities or omitted procedural steps. |
| **Organizational Knowledge Metaphor** | The foundational design paradigm wherein the substrate acts as the authoritative organization/playbook and external agents act as employees/creative generators navigating its guidance. |

---

## 3. Environmental & Boundary Contracts

The Knowledge Substrate operates strictly within the following architectural boundaries:

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
flowchart LR
    classDef secondary fill:#1b3528,stroke:#73c991,stroke-width:1.5px,color:#e6f7ee;
    classDef tertiary fill:#1d2c44,stroke:#61afef,stroke-width:1.5px,color:#e4f0fc;

    subgraph External["External Runtime Boundary"]
        Agents["External Agent Ecosystem<br/><i>(Claude Code, Custom LLM Runners, Aider)</i>"]:::tertiary
        Users["Human Engineering Teams<br/><i>(Web UI, CLI, IDE Extensions)</i>"]:::tertiary
        LocalLLM["Local LLM Service (Ollama)<br/><i>(Asynchronous Background & Audit Worker)</i>"]:::tertiary
    end

    subgraph Substrate["The Knowledge Substrate"]
        Gateway["Integration Gateway<br/><i>(MCP Server & REST API)</i>"]:::secondary
        Governance["Governance & Lineage Engine"]:::secondary
        PostgresStorage["PostgreSQL Substrate<br/><i>(Property Graph + pgvector + Audit Ledger)</i>"]:::secondary
        GitStore["Git Document Ledger<br/><i>(Text Specifications & Revision History)</i>"]:::secondary

        Gateway --> Governance --> PostgresStorage
        Gateway --> GitStore
        PostgresStorage -.->|"Relational Refs & Hashes"| GitStore
    end

    Agents <-->|"Model Context Protocol / JSON-RPC"| Gateway
    Users <-->|"REST API / HTTPS"| Gateway
    LocalLLM <-->|"OpenAI-Compatible REST API"| Gateway
```

* **Resource-Constrained Execution Model:** The project operates under an explicitly constrained resource model. The phased roadmap is engineered so that each progression phase delivers standalone operational value and can be evaluated or falsified independently. Phase 0 and Phase 1 are scoped to be achievable by a small team (or individual developer) utilizing commodity infrastructure. Subsequent milestones are aspirational targets whose investment is strictly contingent on the demonstrated viability and dogfooding adoption of earlier phases.
* **Specification Prerequisite, Brownfield Scaffolding & Cold-Start Boundary:** The Knowledge Substrate operates on the foundational premise that human engineering teams provide externalized intent. TKS is not an autonomous requirements generator or speculative code reverse-engineering scanner; it anchors execution to externalized intent artifacts. For teams operating in brownfield repositories lacking formal PRDs, TKS attacks the cold-start adoption barrier through mechanical **Brownfield Intent Scaffolding**, extracting baseline requirement, architectural decision, and specification nodes directly from structured artifacts (OpenAPI/Protobuf schemas, ADRs, and BDD feature files) at zero token cost. The structural boundary remains absolute: TKS scaffolds from documented intent artifacts, avoiding speculative reverse-engineering of raw code syntax.
* **Single-Engine Operational Footprint:** All topology data, vector embeddings, relational metadata, and audit records reside in a single PostgreSQL instance. Distributed multi-database setups (e.g., maintaining an external vector database or separate graph DBMS alongside PostgreSQL) are prohibited, eliminating distributed transaction failures and synchronization drift. Raw text specifications are version-governed via Git and referenced relationally.
* **Externalized Cognitive Compute & Advisory Maintenance Boundaries:** The core Knowledge Substrate database engine and gateway services never directly invoke LLM inference for internal state transactions; the engine remains strictly deterministic and inference-free. External agents supply their own compute and models. When local LLM services are utilized for semantic pre-analysis or scheduled graph relationship audits (`tks graph audit`), they operate strictly as externalized, read-only advisory processes emitting candidate proposals to review queues without direct write access to active topology. Model attributes (context window, model identifier) are logged alongside recorded decisions to preserve audit context around automated evaluations.
* **Stateless Gateway Boundary & Strict Protocol Conformance:** The MCP and REST interfaces maintain no persistent session memory. Each operation is an authenticated, isolated transaction targeting explicit node identifiers and payloads. The gateway validates caller identity on every request, strictly adhering to official open protocol specifications (such as Model Context Protocol JSON-RPC tool schemas) to guarantee deterministic client interoperability.
* **Document Immutability, Bidirectional Projection Guarantee & Re-Ingestion Delta Reconciliation:** Uploaded text specifications (Markdown, plain text) are stored in a Git-backed document repository and content-addressed via cryptographic hashes, serving as an immutable historical intake ledger and baseline reference archive. The PostgreSQL Property Graph is the sole authoritative living substrate for active project intent, requirements, governance states, and execution tasks. Text specifications in Git represent seed artifacts and point-in-time projection targets (i.e., human-readable Markdown can be dynamically synthesized and exported *from* the living graph), eliminating the fragility of bidirectional synchronization. When updated specification documents are re-ingested from Git, re-ingestion operates under a formal **Document Re-Ingestion Delta Reconciliation Contract**: re-ingestion acts as an additive delta proposal rather than a destructive wholesale replacement. Candidate sections are matched against existing active graph entities via AST anchors and cryptographic hashes. Unchanged entities retain their active state, unique identifiers, and downstream execution links; modified sections enter the staging lifecycle for supervisory three-way delta reconciliation, triggering targeted invalidation cascades (`NEEDS_REVERIFICATION`) rather than clobbering active topologies; and omitted sections are flagged for supervised deprecation rather than hard orphan cascades. This guarantees that ongoing tasks, tactical decisions, and verification edges elaborated in the graph are preserved across document updates.
* **Physical Schema Contracts & Pre-Merge Verification Gating:** In the property graph, topology is governed by strict relational edge integrity between valid, registered node entities; treating edge targets as raw external string primitives is prohibited at both gateway and database levels. Intermediate execution references (such as branch-level Git commit SHAs) are captured in task execution metadata. For pre-merge continuous integration (CI) gating, automated test verification runs can bind directly to active `TASK` entities via `VERIFIED_BY` edges carrying candidate pull request branch commit SHAs in verification metadata (or link to provisional `CODE_COMMIT` entities), allowing CI pipelines to certify PR branch readiness *prior* to merging into `main`. Canonical merge commits to `main` or signed release tags are permanently materialized as canonical `CODE_COMMIT` graph nodes linked via explicit `IMPLEMENTED_BY` edges. All edge mutations are pre-validated before database transactions execute.
* **Hierarchical Ingestion Authority & Precedence:** Ingestion pipelines must evaluate a document's hierarchical position and authority relative to existing knowledge before permitting mutations. For example, lower-level implementation notes or tactical proposals cannot overwrite upstream architectural invariants or vision requirements without explicit governance authorization. When top-level governance or architecture updates occur, changes cascade downstream into affected requirement nodes, generating updated work packages for human review.
* **Schema Flexibility via Progressive Layering:** Core system tables enforce only foundational structural and procedural edges (`DERIVED_FROM`, `CONSTRAINED_BY`, `FULFILLS`, `VERIFIED_BY`, `IMPLEMENTED_BY`, `GOVERNED_BY_PROCEDURE`, `BINDS_LAYER`). Domain-specific attributes, checklists, and evolving project taxonomy are managed via typed JSONB fields to avoid costly schema migrations during early project phases.
* **Bootstrap Boundary Contract & Benchmark Validation:** Initial system design and Phase 0 development occur using conventional developer tooling. The bootstrapping transition proceeds across distinct gates:
  1. *Phase 1 Dogfooding Gate (Read-Only Self-Hosting):* Documentation (`vision.md`, backlogs) is ingested; developers and agents retrieve context envelopes via the read-only MCP gateway to implement Phase 2 tasks.
  2. *Phase 2 Dogfooding Gate (Autonomous Self-Evolution):* All subsequent feature requirements, architectural adjustments, and tasks are authored, reviewed, and governed directly within the substrate itself.
  3. *Gate 4 Dogfooding (Bounded Planning Rails):* Autonomous agents, bounded by multi-axis planning dossiers (`get_elaboration_context`), elaborate schema-compliant tasks for new deliverables under human direction without accessing raw vision markdown.
  4. *Benchmark Validation:* Progression and model-downgrading efficiency are verified against the Real-World Feature Extension Benchmark (with complete self-reconstruction preserved as an aspirational post-v1.0 research horizon in Phase N+).

---

## 4. System Topology & Information Flow

The architecture is organized around four decoupled operational loops: the **Document Ingestion & Governance Pipeline**, the **Agent Execution & Planning Rails Loop**, the **Topological Coherence & Background Maintenance Loop**, and the **Graph Projection & Document Generation Loop**.

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

    Doc["Raw Specifications, SOPs & Governance Profiles"]:::tertiary
    GitLedger["Git Document Ledger<br/><i>(Versioned Text & Object Hashes)</i>"]:::secondary
    ASTParser["Stage 1: Mechanical AST Structural Extractor<br/><i>(80/20 Parsing & Span Resolution at 0 Tokens)</i>"]:::note
    Classifier["Stage 2: Targeted Semantic Classifier<br/><i>(Compact Relational Tuples & Heading Promotion)</i>"]:::note
    HumanReview["Human Staging & Verification Gate<br/><i>(Tractable Supervisory Batches)</i>"]:::primary

    Doc -->|"1. Ingest & Version"| GitLedger
    GitLedger -->|"2. Stream Document Text"| ASTParser
    ASTParser -->|"3. Structural Blocks & Exact Spans"| Classifier
    Classifier -->|"4. Candidate Requirement Tuples"| HumanReview

    subgraph Core["PostgreSQL Knowledge Substrate"]
        GraphStore["Graph Topology & pgvector Embeddings"]:::secondary
        AuditTrail["Immutable Audit Ledger & Draft Compaction"]:::secondary
        Backlog["Relationship Maintenance Backlog"]:::secondary
    end

    HumanReview -->|"5. Commit Verified Graph Nodes"| AuditTrail
    AuditTrail -->|"6. Materialize Topology"| GraphStore

    ExtAgent["External Autonomous Agent"]:::tertiary
    MCPGateway["MCP Protocol Gateway<br/><i>(Strict Protocol Conformance)</i>"]:::note
    GovCheck["Per-Node Governance Policy Filter"]:::note
    LocalWorker["Scheduled Graph Maintenance Worker<br/><i>(Read-Only Advisory Audits)</i>"]:::note

    GraphStore -.->|"7a. Leaf Execution Context (get_context_envelope)"| MCPGateway
    GraphStore -.->|"7b. Multi-Axis Planning Rails (get_elaboration_context)"| MCPGateway
    MCPGateway -.-> ExtAgent
    ExtAgent -->|"8. Propose Node Mutation / Subtasks"| MCPGateway
    MCPGateway -->|"9. Validate Schema & Invariants"| GovCheck
    GovCheck -->|"10a. Authorized Mutation"| AuditTrail
    GovCheck -.->|"10b. Invariant Violation (Remediation Envelope)"| ExtAgent

    GraphStore -.->|"11. Scheduled Audit (tks graph audit)"| LocalWorker
    LocalWorker -->|"Advisory Proposals"| Backlog

    DocGen["Graph-to-Document Generator<br/><i>(Targeted & High-Level Markdown Projection)</i>"]:::note
    GraphStore -->|"12. Query Subgraph"| DocGen
    DocGen -->|"13. Formatted Spec Export"| GitLedger
```

### Conceptual Operational Loops

1. **Document & Institutional Knowledge Ingestion Loop:**
   Ingests text specifications, engineering SOPs, modular governance profiles, and structured brownfield intent artifacts (OpenAPI schemas, ADRs, BDD feature files) into the Git document ledger as immutable baseline references. Decomposes documents through a strictly ordered multi-tier pipeline:
   * *Tier 1 (Mechanical AST Structural Extraction & Brownfield Scaffolding):* Deterministically segments structural blocks and resolves exact source character spans at zero token cost, completing 80%+ of the parsing work mechanically. Scaffolds baseline entities from structured artifacts and classifies top-level document sections, Level-1/Level-2 headings, or frontmatter-annotated sections as `REQUIREMENT` anchors under a principled structural hierarchy convention rather than relying on brittle keyword regexes, ensuring an intact ancestral hierarchy for Invariant INV-1 out-of-the-box.
   * *Tier 2 (Local LLM Pre-Analysis & Contradiction Risk Check):* Executed if available and requested, evaluating candidate chunks against active approved nodes with **contradiction risk evaluated first**, followed by preliminary ambiguity and testability scoring.
   * *Tier 3 (User-Invoked Commercial LLM Escalation):* If escalated by the user, an external commercial LLM is directed via open protocols to inspect candidates, resolve semantic ambiguities, and classify compact relational tuples.
   * *Tier 4 (Human Supervisory Gate):* Candidate nodes are presented within their source document narrative hierarchy using inline visual badges and callouts for contradiction risk and quality scores, preventing cognitive disorientation. For re-ingested documents, Tier 4 executes three-way delta reconciliation against active graph topology, preserving active node UUIDs, elaborated child tasks, and downstream verification links while staging only detected deltas. Verified nodes are committed to PostgreSQL with cryptographic source span links. A flattened risk-sorted view is provided as a secondary triage option.
   * *Tier Ordering Invariant:* Any combination of tiers is valid, but execution must always follow this strict sequence.
2. **Agent Execution, Governed Mutation & Cognitive Planning Rails Loop:**
   Provides external autonomous agents with tailored context along two distinct pathways:
   * *Leaf-Task Implementation:* Retrieves immediate upward parent requirements, sibling constraints, and semantic vector neighbors via `get_context_envelope`.
   * *Strategic Phase & Task Elaboration:* Retrieves a multi-axis planning dossier via `get_elaboration_context`, providing normative invariants, historical decisions, physical schema rules, and codebase implementation blueprints to bound agent task elaboration under human-directed deliverable directives.
   * Agents execute planning or coding in external runtimes and propose mutations back through the gateway. Mutations are checked against per-node governance policies, deliverable contracts, and physical edge constraints (verifying that edge endpoints exist as valid node UUIDs before committing).
   * Inconsequential tactical decisions made by the agent (e.g., parameter ranges, CLI flags) are automatically captured into node attributes with full caller provenance, bounded by permission scoping that prohibits unauthorized upward propagation.
   * If an invariant is violated, TKS emits a structured **Remediation Envelope** detailing the broken ancestor chain and deterministic repair options, enabling autonomous self-healing.
3. **Topological Coherence, Scheduled Graph Maintenance & Gap Auditing Loop:**
   Traverses the property graph via explicit edge traversals and vector neighborhoods to identify systemic omissions and discontinuities. Augmented by scheduled or on-demand audit sweeps (`tks graph audit`):
   * Executes deterministic structural graph linting (reachability, orphan detection, broken transitive closures) as a mandatory first pass before invoking local LLM inference for subtle contradiction analysis.
   * Operates strictly in a read-only advisory capacity, enqueueing candidate relationship adjustments and pruning recommendations into `relationship_review_backlog` without mutating active topology.
   * Evaluates ingestion hierarchy authority: when top-level policies or architectural specifications update, the cascade engine automatically flags affected downstream requirements, generating updated work packages for human review.
4. **Graph Projection & Human-Readable Document Generation Loop:**
   Solves the machine-versus-human representation gap. While graph nodes, edge vectors, and JSONB attributes are optimized for agentic traversals, human comprehension requires narrative structure.
   * Traverses and aggregates diverse graph nodes and relationships across broad perspectives (high-level vision and architecture overviews) or targeted vertical slices (end-to-end trace of a single user workflow).
   * Orchestrates an LLM to query the substrate, assemble the retrieved context, and format the output into structured, linted Markdown adhering to project documentation standards.
   * Exports the generated documents back to the Git repository as versioned, point-in-time reference artifacts.

*(Note: Detailed step-by-step API message protocols and interaction sequences are cataloged in the [Strategic Planning Backlog](docs/vision/strategic-planning-backlog.md).)*

---

## 5. Invariants, Boundary Constraints & Hypotheses

### Architectural Invariants (Engineering Directives)

* **Invariant I-1 (Strict Bidirectional Traceability & Procedural Grounding):** Every functional specification, implementation task, and code artifact reference must maintain a valid, directed edge path terminating at an active requirement node. Furthermore, execution tasks involving institutional procedures (e.g., introducing third-party dependencies, altering security boundaries) must maintain explicit dependency edges to applicable procedural policy and checklist nodes. Verification of tasks must adhere to level-appropriate policies (unit tests for tactical choices, integration tests for architectural decisions). Orphan execution tasks and ungoverned procedural actions are rejected at the database constraint level.
* **Invariant I-2 (Auditability & Reversible Lineage):** Destructive in-place updates (`UPDATE` or `DELETE`) on approved requirements, specifications, and topological edges are strictly prohibited. State transitions across approved entities must be recorded such that every state mutation is fully auditable, reversible, and point-in-time reconstructible without data loss. A strict distinction is enforced between active, approved entities and candidate drafts: candidate entities and exploratory agent proposals reside in an isolated draft lifecycle and are subject to lifecycle event compaction upon promotion to active status, collapsing drafting churn into a single canonical audit milestone to eliminate ledger bloat while preserving active lineage.
* **Invariant I-3 (Zero In-Database Agent Execution & Advisory Maintenance):** The core database engine and gateway services shall never execute autonomous agent cognitive loops internally. The substrate functions strictly as a deterministic state store and protocol gateway. Auxiliary background audit workers (`tks graph audit`) or decomposition pipelines connecting to local or external LLMs operate strictly as externalized clients through standard authenticated APIs, functioning in a read-only advisory capacity without direct write access to active topology.
* **Invariant I-4 (Cryptographic Source Anchoring):** Every requirement derived via the decomposition pipeline must store a persistent cryptographic reference (Git commit/blob hash) and source span coordinates pointing to the original document artifact.
* **Invariant I-5 (Explicit Per-Node Governance Authority & Bounded Tactical Delegation):** Permissions to alter or elaborate a node are governed by explicit per-node metadata attributes (`governance_policy`). Node-level policies take absolute precedence over global agent roles. External agents may be granted bounded delegation to resolve inconsequential tactical decisions (such as parameter ranges or CLI flags), which are captured in node attributes with complete provenance, but policies strictly restrict how far up the requirement hierarchy an agent can propagate changes without human authorization.
* **Invariant I-6 (Self-Referential Bootstrapping & Governed Co-Evolution):** The Knowledge Substrate must be used to manage its own development through a phased bootstrapping transition. Upon completing Phase 1 foundational ingestion and read-only context retrieval, the substrate self-hosts its own documentation for context querying. Upon completing Phase 2 mutation tooling and governance, all subsequent feature requirements, architectural adjustments, and development tasks are authored, reviewed, and tracked directly within the substrate itself. The substrate supports bounded, interactive task elaboration under human-directed deliverable directives, concurrent multi-agent co-evolution within branch workspace containers, automated invalidation cascades, and reverification recovery under topological conflict resolution (Gate 3 and Gate 4). Progression is validated against the Real-World Feature Extension Benchmark (with complete self-reconstruction preserved as an aspirational post-v1.0 research horizon).
* **Invariant I-7 (Agent Identity Attribution & Workspace Draft Isolation):** Every mutation and candidate draft submitted through the Integration Gateway must be attributable to a verified external identity (human user or autonomous agent instance) stamped with `created_by` and recorded in the audit ledger (`actor_id`, `actor_type`, `token_fingerprint`). Candidate entities elaborated within ephemeral branch workspace containers are strictly confidential and isolated from the live substrate and peer workspaces, visible dynamically only to the authoring agent session until verified and atomically promoted under canonical lock serialization. Identity credentials must be validated on every mutation request; revocation must immediately invalidate local cache entries across running services.
* **Invariant I-8 (Actionable Diagnostics & Self-Healing Remediation Envelopes):** Opaque, uninformative constraint error strings (such as bare `ERR_INVALID_ANCESTOR_PATH`) are strictly prohibited in public gateway responses. Whenever a structural invariant or ancestor path validation fails, the gateway must return a structured JSON Remediation Envelope detailing the precise point of failure (e.g., terminal node identity and type) and enumerated, deterministic remediation actions (e.g., ancestor promotion options when an elaboration requires an ancestor `REQUIREMENT`), enabling autonomous agents to self-correct within the governance framework.
* **Invariant I-9 (Physical Edge Relational Integrity & Pre-Merge Verification Gating):** Graph edge relationships enforce strict relational integrity: all edges must link valid, registered node entities in the property graph. Treating edge targets as external string primitives (such as raw Git commit SHAs) is prohibited at both gateway and database levels. Intermediate execution references (such as branch-level commits) are recorded within task execution metadata. To resolve the pre-merge verification paradox, continuous integration (CI) test runs may link directly to active `TASK` entities via `VERIFIED_BY` edges (with candidate branch commit SHAs stored in verification attributes) or to provisional commit nodes, enabling PR branch readiness certification *before* merging into `main`. Canonical release milestones and merge commits to `main` are permanently materialized as typed `CODE_COMMIT` nodes linked via explicit `IMPLEMENTED_BY` edges. All edge mutations must be validated against schema and relationship contracts prior to transaction execution.

### Strategic Hypotheses (Scientific Bets to De-Risk)

* **Hypothesis H-1 (Governed Requirement Topologies vs. Ad-Hoc Agentic Retrieval):**
  *We hypothesize that* anchoring external agents to a *purpose-built, version-governed, human-curated* requirement graph provides decisive advantage over best-available code-level agentic context assembly—including multi-tool code exploration, AST symbol traversal, and ad-hoc or auto-generated code knowledge graphs lacking requirement provenance—by supplying authoritative intent provenance, operational policies, and audit lineage that code-level syntax exploration structurally lacks, thereby significantly reducing downstream architectural contract violations and intent drift.
* **Hypothesis H-2 (Sublinear Human Oversight Overhead):**
  *We hypothesize that* managing autonomous agents through structured requirement graphs and topological impact analyses significantly reduces human supervisory overhead compared to manual inspection of agent-generated code diffs.
* **Hypothesis H-3 (Single-Engine Relational Scalability):**
  *We hypothesize that* a single PostgreSQL instance combining graph query patterns and `pgvector` scales comfortably to support large-scale enterprise project graphs without requiring dedicated graph or vector database clusters.
* **Hypothesis H-4 (Assisted Ingestion Accuracy & 80/20 Mechanical Efficiency):**
  *We hypothesize that* a decomposition pipeline combining mechanical AST structural parsing for the initial 80/20 extraction with commodity LLMs for narrow semantic classification achieves high-fidelity requirement extraction from technical markdown at a fraction of the token expenditure required by monolithic LLM ingestion prompts.
* **Hypothesis H-5 (Topological Gap Detection & Procedural Scaffolding):**
  *We hypothesize that* structuring institutional procedures (such as dependency onboarding, licensing verification, and CVE audits) and cross-layer architectural contracts as a navigable property graph allows structural traversals combined with external LLM-in-the-loop auditing to detect systemic gaps (such as unlinked UI-to-backend workflows or unvetted third-party libraries) significantly earlier than conventional PR reviews, while reducing agent procedural non-compliance.
* **Hypothesis H-6 (Substrate-Guided Model Downgrading & Cognitive Rail Efficiency):**
  *We hypothesize that* providing an autonomous agent with a multi-axis planning dossier (`get_elaboration_context`) and strict topological rails narrows the required degrees of freedom sufficiently that a high-speed, cost-effective commercial utility model (such as Claude 3.5 Haiku, Gemini Flash, or GPT-4o-mini, achieving 10x–20x cost reduction) can match the task implementation and bounded elaboration fidelity of an unguided frontier model on the Real-World Feature Extension Benchmark (Phase 5 Gate 4), with commodity local 8B models evaluated on an exploratory research horizon (Phase N+).
* **Hypothesis H-7 (Modular Governance Profiles & Cascading Compliance):**
  *We hypothesize that* encapsulating high-leverage institutional, security, and architectural standards into modular, reusable governance profiles, and automatically cascading policy updates downstream into affected requirement nodes with severity classification (Breaking vs. Advisory) and blast-radius throttling, eliminates institutional compliance drift and prevents supervisory alert exhaustion across large codebases.

*(Note: Quantitative calibration targets and metric benchmarks for each hypothesis are cataloged in the [Strategic Planning Backlog](docs/vision/strategic-planning-backlog.md).)*

### Inside-Out Mechanics to Outside-In Strategic Leverage

| Architectural Mechanism (Inside-Out) | Strategic & Economic Leverage (Outside-In) |
| --- | --- |
| **Agent-Agnostic Protocol Gateway (MCP / REST)** | **Vendor Decoupling & Model Arbitrage:** The enterprise can adopt emerging LLM models and specialized external agent frameworks without modifying the underlying project knowledge base or business logic. |
| **Unified PostgreSQL Engine** | **Operational Economy & Zero Infrastructure Sprawl:** Utilizes proven enterprise database tooling for backup, replication, security, and compliance, avoiding the operational cost of multi-database synchronization. |
| **Git-Backed Document Ledger** | **Proven Versioning & Transparent Text Diffing:** Leverages industry-standard VCS infrastructure for text artifact versioning and delta compression, seamlessly integrating with existing developer workflows. |
| **Per-Node Governance Controls & Tactical Delegation** | **Controlled Scaling of Autonomous Capacity:** Engineering leadership can progressively delegate lower-risk system layers and tactical parameters to autonomous agents while enforcing strict human oversight over mission-critical components. |
| **Self-Referential Architecture & Feature Extension Benchmark** | **Accelerated Dogfooding & Commercial Model Arbitrage:** Forcing the system to manage its own lifecycle and verifying that cognitive rails empower commercial utility models (10x–20x cheaper) to extend complex real-world software modules ensures the product solves genuine engineering problems. |
| **First-Class Procedural & Gap Modeling** | **Institutional Continuity & Proactive Quality Assurance:** Encodes corporate development standards and cross-layer architectural checks into the active operational graph, preventing autonomous agents from cutting procedural corners or generating disconnected software layers. |
| **Multi-Axis Planning Rails (`get_elaboration_context`)** | **Cognitive Grounding & Elimination of Planning Deserts:** Prevents autonomous agents from hallucinating ungrounded architectural patterns by binding normative invariants, architectural decisions, and physical schemas into strategic planning steps. |
| **Modular Governance Profiles** | **Enterprise Policy Portability & Throttled Compliance:** Allows organizations to define institutional compliance standards once (dependency vetting, EU CRA, security) and bind them across disparate software projects with blast-radius throttling to avoid alert floods. |
| **Draft Event Compaction** | **Scalable Audit Ledger Maintenance:** Preserves full exploratory freedom for agents during draft elaboration while eliminating audit log bloat upon formal approval. |
| **Scheduled Graph Maintenance Auditor (`tks graph audit`)** | **Zero-Cost Scheduled Graph Hygiene:** Enables non-destructive link refinement, contradiction detection, and backlog pruning on scheduled sweeps without modifying active topology or incurring expensive external API fees. |
| **Dynamic Graph-to-Document Projection** | **Human-Centric Transparency & Bidirectional Utility:** Translates machine-optimized graph topologies into tailored, human-readable documentation without maintaining fragile dual-write synchronization. |
| **Level-Appropriate Verification Policies** | **Targeted Quality Assurance:** Balances verification rigor by enforcing unit testing for tactical decisions and integration test suites for core architectural contracts. |

---

## 6. Multi-Axis Progression Model

Program advancement is organized across two orthogonal vectors: **Governance & Provenance Maturity** and **Agent Autonomy & Integration Breadth**.

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
quadrantChart
    title Capability Progression Matrix
    x-axis Low Agent Autonomy --> High Agent Autonomy
    y-axis Basic Ingestion --> Deep Semantic Governance
    quadrant-1 "Phase 6: Governance Profiles & Maintenance"
    quadrant-2 "Phase 5: Cognitive Planning Rails (Gate 4)"
    quadrant-3 "Phases 1-3: Substrate Core & Impact (Done)"
    quadrant-4 "Phase 4: Ingestion & Closed-Loop Traceability"
    "Completed: Phases 0-3 (Substrate Core & Impact)": [0.4, 0.4]
    "Phase 4: Ingestion & Closed-Loop Traceability": [0.45, 0.6]
    "Phase 5: Cognitive Planning Rails (Gate 4)": [0.75, 0.75]
    "Phase 6: Governance Profiles & Maintenance": [0.85, 0.9]
```

### Capability Progression Dimensions

1. **Governance & Provenance Maturity (Y-Axis):**
   Advances from basic text specification ingestion and cryptographic anchoring, through closed-loop commit and test verification, to modular governance profile binding, procedural checklist enforcement, automated invalidation cascading, and scheduled background graph hygiene.
2. **Agent Autonomy & Integration Breadth (X-Axis):**
   Advances from read-only topological context retrieval via open protocols, to closed-loop leaf task execution, bounded agent elaboration of sub-tasks guided by multi-axis planning rails (`get_elaboration_context`), tactical decision capture, self-healing remediation envelopes, and distributed multi-agent collaborative execution.

### The Bootstrapping Progression Strategy

To honor the principle to "start small" under a constrained resource model, development follows a strict self-referential bootstrapping path where each milestone delivers standalone utility before subsequent phases are attempted:

* **Phase 0 Baseline (Completed):** Initial architecture and storage foundations built externally. Spikes 0 and 8 validated H-1 and local embedded vector generation.
* **Phase 1 Self-Hosting Gate (Completed):** Core ingestion and read-only MCP gateway self-hosted project documentation for context querying.
* **Phase 2+ Evolution Gate (Completed):** Mutation tools, draft lifecycle event compaction, and per-node governance enabled autonomous self-evolution.
* **Phase 3 Maturity (Completed):** Impact analysis, invalidation cascading, and branch workspace isolation.
* **Phase 4 Foundational Ingestion & Closed-Loop Verification:** Mechanical heading promotion, protocol compliance, edge pre-validation, brownfield intent scaffolding, VCS commit linking, and CI test verification to deliver core end-to-end requirement-to-code traceability early.
* **Phase 5 Cognitive Planning Rails & Governed Dogfooding:** Multi-axis planning dossiers (`get_elaboration_context`), actionable remediation envelopes, and bounded interactive task elaboration templates, achieving Gate 4 dogfooding under human deliverable directives.
* **Phase 6 Modular Governance Profiles & Scheduled Graph Maintenance:** Enterprise policy packs, policy invalidation cascades, and scheduled/on-demand read-only graph audits (`tks graph audit`).
* **Phase N+ Advanced Strategic Horizons:** Long-term research benchmarks (Self-Reimplementation Benchmark), reverse document projection, and external artifact validation.

### Milestone Proof-of-Concept & Implementation Benchmarks

To empirically validate the substrate against real-world engineering challenges, the roadmap includes distinct validation benchmarks:

1. **Open-Source Tool Reproduction POC:** Demonstrating that an autonomous agent guided solely by specifications and context retrieved from TKS can implement a functional, mature open-source tool end-to-end without unguided internet searching.
2. **Complex Systems & Formal Standards Benchmark:** Implementing a complex system governed by rigorous, multi-tiered specifications (such as reproducing compiler specifications or safety-critical software modules) adhering to INCOSE/NASA requirement standards and level-appropriate testing policies.
3. **Dual Execution Mode Evaluation:**
   * *Mode A (Repo Ingestion & Gap Discovery):* Ingesting an existing software repository alongside its documentation and brownfield intent artifacts (ADRs, OpenAPI) to discover missing cross-layer bindings, unvetted dependencies, and undocumented architectural invariants.
   * *Mode B (Specification-First Greenfield Construction):* Constructing a complete software implementation purely from ingested standard documentation and modular governance profiles.
4. **The Real-World Feature Extension Benchmark:** Demonstrating that an autonomous agent, guided by TKS planning rails and context envelopes, can successfully implement a substantial, new feature module within an existing realistic codebase without architectural violations, empirically proving that cognitive rails enable model downgrading. (The complete Self-Reimplementation Benchmark is maintained as an aspirational post-v1.0 research horizon).

*(Note: Specific phase deliverables, engineering schedules, and operational dependencies are detailed in the [Strategic Planning Backlog](docs/vision/strategic-planning-backlog.md).)*

---

## 7. Directional Gating & Governance

### Key Observables

Progression across capability milestones is evaluated using eleven directional metrics:

* **Retrieval Boundedness & Relevance:** Ratio of required context tokens delivered to external agents versus irrelevant noise, measuring the efficiency of the topological context envelope.
* **Decomposition Lineage Fidelity:** Percentage of extracted requirement nodes that correctly resolve to exact, verifiable source spans within the signed source documents.
* **Token Output Reduction Efficiency:** Percentage reduction in LLM output tokens achieved by delegating structural decomposition to 80/20 mechanical AST extractors compared to end-to-end generative LLM parsing.
* **Drift & Invalidation Velocity:** Latency from the moment a parent requirement or governance policy is modified to the complete identification and flagging of all invalidated downstream tasks and generation of updated work packages.
* **Supervisory Decision Latency:** Time required for an engineering lead to evaluate, approve, or reject an agent-proposed requirement mutation via the supervisory interface.
* **Planning Remediation Rate:** Frequency and success rate of autonomous agents successfully recovering from invariant validation failures using structured remediation envelopes without manual developer intervention.
* **Topological Gap Detection Yield & Precision:** Ratio of verified architectural gaps (e.g., missing layer bindings, orphan workflows, unconsidered edge cases) identified by graph traversal and background auditing relative to total flagged anomalies.
* **Procedural Compliance Fidelity:** Percentage of agent-elaborated tasks (e.g., dependency additions, architecture extensions) that demonstrably satisfy institutional checklists and governance profiles before promotion to human review.
* **Model Downgrading Margin:** Comparative task completion success rate of commercial utility models (Haiku/Flash/mini, 10x–20x cost reduction) guided by multi-axis planning rails versus unguided frontier models on complex implementation tasks, followed by exploratory local 8B edge evaluation.
* **Time-to-First-Value Latency:** Elapsed wall-clock and supervisory time from initial repository deployment to the first verified agent task execution guided by a substrate context envelope, measured starting from unstructured seed text or zero ingested specifications, quantifying and bounding the cold-start adoption barrier.
* **Reverse Projection Fidelity:** Accuracy and structural completeness of human-readable documentation synthesized from graph subgraphs as evaluated by human engineering leads.

### Demonstration Milestones (MVD Overview)

The path to the North Star is gated by four Minimum Viable Demonstrations:

* **Milestone 1 (Ingest, Version, and Retrieve - Phase 1, Completed):** Ingestion of a Markdown specification into the Git document store, assisted decomposition via 80/20 mechanical AST parsing into graph requirement nodes with cryptographic parent links and semantic milestone heading classification (`REQUIREMENT`), and successful retrieval of bounded topological context envelopes by an external agent via an MCP server with verified protocol conformance.
* **Milestone 2 (Bounded Mutation, Draft Compaction & Clean Rollback - Phase 2, Completed):** An external agent performs subtask elaboration under `AUTONOMOUS_ELABORATION` nodes, commits draft changes subject to event compaction, and a human supervisor executes a clean graph rollback to a historical snapshot.
* **Milestone 3 (Closed-Loop Traceability & Ingestion Quality - Phase 4):** Ingestion with heading promotion and brownfield scaffolding, narrative-preserving staging review with inline quality badges, and bidirectional mapping linking Git commits and automated test results to leaf requirements under level-appropriate testing policies.
* **Milestone 4 (Cognitive Planning Rails & Governed Dogfooding - Phase 5 & 6):** Bounded interactive phase elaboration via `get_elaboration_context` under human deliverable directives (Gate 4), modular governance profiles, scheduled graph maintenance (`tks graph audit`), and empirical verification via the Real-World Feature Extension Benchmark.

*(Note: Detailed execution scripts and verification procedures for each demonstration are documented in the [Strategic Planning Backlog](docs/vision/strategic-planning-backlog.md).)*

### Falsification & Termination Criteria (Kill Conditions)

The program should be halted, redirected, or fundamentally restructured if any of the following failure conditions occur:

1. **The Ingestion Friction & Cold-Start Falsification:**
   If the total lifecycle overhead of formulating, ingesting, decomposing, and verifying specifications in the graph—measured from zero structured intent to the first usable context envelope—consistently exceeds the manual effort required for engineering teams to write tickets, prompt agents ad-hoc, and supervise code, the core economic premise of an automated knowledge substrate is disproven.
2. **The Graph RAG Inefficacy Falsification (Hypothesis H-1 Failure):**
   If controlled benchmarks reveal that external agents operating over governed, graph-structured context envelopes exhibit no statistically significant advantage in preserving architectural contracts and preventing intent drift compared to agents using best-available multi-tool retrieval or auto-generated code graphs lacking requirement provenance, the graph-native intent thesis is falsified.
3. **The Single-Engine Relational Bottleneck (Hypothesis H-3 Failure):**
   If graph traversals over versioned tables in PostgreSQL fail to maintain acceptable interactive latencies at scale, and this bottleneck cannot be resolved through index optimization, the single-engine architectural boundary must be abandoned in favor of a specialized graph database.
4. **The Bootstrapping Failure Falsification:**
   If the development team cannot dogfood the Knowledge Substrate to manage its own post-Phase-1 development tasks and specifications due to operational friction or semantic inadequacy, the core premise of an agentic engineering substrate is falsified.
5. **The Cognitive Rail Inefficacy Falsification:**
   If providing external agents with multi-axis planning dossiers (`get_elaboration_context`), physical schema contracts, and actionable remediation envelopes fails to measurably reduce architectural hallucinations, generic web guessing, and schema-violating mutations compared to unguided single-node prompt envelopes when evaluated using commercial utility models (Claude 3.5 Haiku, Gemini Flash, GPT-4o-mini) on the Real-World Feature Extension Benchmark, the active cognitive rails thesis is disproven. (Local 8B edge model evaluation is conducted as an exploratory research horizon in Phase N+ and does not serve as a gating kill condition for Phase 5).

### Graduated Response to Partial Validation

If empirical results partially validate a strategic hypothesis—delivering measurable but below-target improvements—the appropriate response is scope adjustment rather than program termination. Partial validation may warrant narrowing the target domain (e.g., focusing on projects with specific structural characteristics where graph retrieval demonstrably excels), adjusting calibration targets, or hybridizing approaches. Kill conditions are triggered only when results show no statistically significant improvement over the baseline, or when the overhead of the substrate demonstrably exceeds the value it provides.

### Strategic Planning Handoff

Detailed database table definitions, API route contracts, specific embedding model selections, architectural evaluation spikes, and phased implementation roadmaps are maintained in the [Strategic Planning Backlog](docs/vision/strategic-planning-backlog.md).
