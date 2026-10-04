# New Vision

Here are some additional thoughts from the visionary that harden what TKS and elaborate what it should be able to do.

## Core Philosophy & Token Optimization

Overall, we need to minimize token usage by sticking to a primary philosophy: **minimal LLM reliance and maximum reliance on mechanical processes.**

* **Reducing Output Tokens:** Since output tokens are much more expensive than input tokens, saving on output is key. While we can't reduce ingestion costs significantly, we can lower the LLM's output burden by using mechanical extraction tools to handle the initial 80/20 (or better) of requirements parsing.  
* **Document Decomposition:** We should explore what existing tools or mechanical options are available to decompose ingested documents and further cut down the LLM token load.

## State Management & Versioning

We need a clear status distinction between **"Approved / Locked"** and **"Draft / Editable"**. Making every small draft immutable would create far too much churn and turn this into a complex version control system rather than a clean, auditable set of evolving requirements. For history and audit logging, we could maintain detailed event logs while a requirement is in draft. Once it gets approved, we collapse those intermediate draft events to avoid history bloat, while still preserving visibility into why the document mutated during drafting (just an idea, not a strict requirement).

## Modular Governance & Profile Linking

For the next generation of the knowledge substrate (TKS), a key use case is ingesting governance materials—such as processes, procedures, policies, and standards.

* **Modular Encapsulation:** Governance documents should be encapsulated within their own units or projects, treating them as modular concerns. The substrate analyzes the document during ingestion to establish how internal requirements relate to one another.  
* **Governance Profiles:** When starting a project and ingesting its vision or architecture, you can define a governance profile to link specific policies and standards. The substrate handles the underlying connections automatically.  
  * **Example:** A documentation policy might mandate Markdown, specific linting rules, and themed Mermaid charts. When creating a new project (e.g., simulating leaves blowing on a tree), linking this documentation policy ensures the vision and architecture adhere to those rules.  
* **Quality Verification Loop:** With these connections in place, verification can tie commits directly to requirements and standards, checking whether project artifacts validate against required policies.  
* **Expanded Domain Policies:** This framework easily extends to security compliance (such as EU CRA equivalents) and repository management policies. All of these concepts stack on top of the core hybrid RAG and graph node connectivity.

## Background Processing & Local LLM Integration

While TKS avoids external LLM dependencies, it should support optional integration with a standard local LLM API for background processing and automated checks:

* **Requirements Analysis:** A free, local model can perform tasks like checking if two requirements overlap or contradict.  
* **Capability Configuration:** Configuration settings should detail model attributes (e.g., context window size, model strength). These traits can be logged alongside recorded decisions to maintain context around automated output.

## Substrate Authority & Cascading Changes

The knowledge substrate must remain authoritative, requiring strict consistency checks during ingestion:

* **Hierarchy & Ingestion Care:** Ingestion needs to evaluate where a document fits in the hierarchy and what it is subservient to. For example, architecture revisions must be checked for legitimate authority before updating implementation details.  
* **Change Cascades:** When top-level policies update (e.g., a new revision of the Cyber Resiliency Act), changes should cascade down automatically. The system identifies affected requirements and generates updated work packages to feed into design decisions.

## Self-Reimplementation Benchmark

A key performance gate for TKS is self-reimplementation. Using a previous version of TKS alongside a less capable, cheaper LLM should be sufficient to rebuild TKS. By providing richer context and higher precision from the substrate, the model requires fewer degrees of freedom to accomplish complex implementation tasks.

## Human-Readable Document Generation

TKS needs the capability to generate human-readable documentation. Graph nodes, links, and underlying rationales are optimized for machine processing rather than human comprehension. Therefore, clear workflows are required to assemble graph content into readable output:

* **Targeted & High-Level Views:** Users can request broad document types (such as high-level vision or architecture overviews) or targeted vertical queries (e.g., explaining how and why a specific UI workflow operates).  
* **Graph Aggregation:** Generating these views involves traversing and pulling diverse nodes and relationships across the graph into a single document.  
* **LLM Orchestration:** This can be implemented by pointing an LLM at TKS to query the database, pull the necessary context, and format the output according to defined project standards.

## Background Relationship Processing & Graph Maintenance

Beyond initial ingestion, a dedicated low-priority background process can continuously analyze and refine the relationship graph:

* **Continuous Analysis:** While direct ingestion handles surface-level validations and immediate conflicts, parallel or asynchronous ingestions may introduce indirect contradictions or concepts that collide later.  
* **Link Pruning & Adaptation:** A background task can perform deep evaluations of requirement linkages—strengthening, weakening, adapting, or pruning connections over time.

**Prioritized Review Backlog:** The process can maintain a ranked backlog of suspicious, interesting, or optional relationships to re-examine, incrementally grinding through and refining database structure.

## Document Validation & Knowledge Base Review

As TKS serves as the definitive knowledge base for a project, it can validate external documents or content presented to it. This capability allows team members to review artifacts—such as high-level executive summary presentations—against the established knowledge base. TKS can provide structured feedback by identifying incorrect statements or confirming that abstractions and high-level summaries accurately reflect the underlying details. While not an immediate priority, this represents a valuable roadmap feature.

## Ingestion Validation & Analysis Workflows

During the document ingestion phase (particularly for top-level vision documents), TKS should actively flag gaps, ambiguities, and contradictions. Performing this analysis requires an LLM, which can be integrated through two primary workflows:

* **MCP & Commercial LLM Workflow:** Documents are uploaded and staged, after which a commercial LLM processes the content, requests clarifications as needed, and routes items through a human review gate.  
* **Local LLM Integration:** TKS can interface directly with a local LLM to analyze statements and translate raw vision documents into precise, non-contradictory requirement language.  
* **Incremental Requirement Ingestion:** Requirements do not need to be fully finalized or validated immediately. TKS supports incremental ingestion across multiple documents, allowing subsequent inputs to elaborate on identified gaps over time.

## Tactical Decision Capture & Governance Policies

When an implementation or planning LLM makes tactical decisions (such as specifying missing command-line parameters and value ranges), those decisions should automatically push back into TKS. TKS records the complete traceability and audit trail, linking the decision to higher-level requirements, the originating task, and the specific agent responsible.  
To control this behavior, selectable governance policies define the scope of LLM decision-making authority:

* **Delegation Boundaries:** LLMs can be permitted to resolve inconsequential tactical choices to maintain progress without escalating minor decisions.  
* **Permission Scoping:** Policies restrict how far up the requirement hierarchy an LLM can propagate changes, preventing unauthorized modifications to core vision-level requirements unless explicitly allowed by project settings.

## Recursive Standards & Test Coverage Policies

TKS can apply requirements recursively to its own structure (dogfooding) by adhering to configurable standards (such as NASA or INCOSE requirement guidelines).  
Additionally, testing policies can enforce alignment across different architectural levels:

* **Level-Appropriate Verification:** Tactical decisions are validated via unit tests, whereas architectural decisions are verified via integration tests.  
* **Criticality-Based Rules:** TKS tracks test coverage and applies rules based on requirement criticality—ranging from basic structural checks for experimental code to rigorous test suites for critical components.

## Milestone Proof-of-Concept & Implementation Benchmarks

A key future milestone is demonstrating end-to-end project implementation directly from ingested documentation and context:

* **Open-Source POC:** Initial verification will focus on fully implementing a mature, solid open-source tool from its specification.  
* **Complex Systems & Standards:** Advanced milestones will target large-scale projects governed by detailed standards (similar to reproducing compiler specifications).

**Dual Execution Modes:** Demonstrations will evaluate both ingesting existing repositories to uncover architectural gaps/inconsistencies and building implementations purely from standard documentation definitions.
