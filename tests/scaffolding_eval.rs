//! Brownfield intent scaffolding adapters & authority inheritance evaluation suite (WP-4.2, CAL-BROWN, TB-11).
//!
//! Validates:
//! 1. Zero-token mechanical extraction from OpenAPI 3.0/3.1 and AsyncAPI specs.
//! 2. Mechanical Root Requirement Synthesis satisfying Invariant INV-1.
//! 3. ADR (MADR & Nygard) extraction into root REQUIREMENT and child DECISION nodes.
//! 4. SDD (BDD Features, Spec Kit, AWS Kiro, AGENTS.md / CLAUDE.md) extraction.
//! 5. Authority inheritance auto-promotion for pre-governed sources (BRANCH_PROTECTED_CODEOWNERS)
//!    bypassing staging and directly inserting ACTIVE nodes with caller attribution in audit_ledger.

use std::time::Instant;
use uuid::Uuid;

use tks::db;
use tks::gateway::auth::AuthenticatedAgent;
use tks::gateway::routes::documents::IngestDocumentRequest;
use tks::ingest::scaffolding::{AdrAdapter, OpenApiAdapter, ScaffoldingAdapter, SddAdapter};
use tks::storage::StorageRepo;
use tks::storage::git::start_git_actor;
use tks::worker::decomp::process_one_job;

static TEST_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct TempRepoGuard {
    path: std::path::PathBuf,
}

impl TempRepoGuard {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("tks-test-scaffold-git-{}", Uuid::new_v4()));
        Self { path }
    }
}

impl Drop for TempRepoGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[test]
fn test_openapi_scaffolding_extraction() {
    let openapi_yaml = r#"
openapi: 3.0.3
info:
  title: Petstore Service
  version: 1.2.0
  description: High-throughput petstore management API.
paths:
  /pets:
    get:
      summary: List all pets
      operationId: listPets
    post:
      summary: Create a pet
      operationId: createPet
  /pets/{petId}:
    get:
      summary: Info for a specific pet
      operationId: showPetById
    delete:
      summary: Delete pet
      operationId: deletePet
components:
  schemas:
    Pet:
      type: object
      description: Domain representation of a pet.
    Error:
      type: object
      description: Error envelope.
"#;

    let adapter = OpenApiAdapter::new();
    assert!(adapter.can_handle("specs/petstore.yaml", openapi_yaml));
    assert!(adapter.can_handle("openapi.json", "{}"));

    let start = Instant::now();
    let result = adapter
        .parse("specs/petstore.yaml", openapi_yaml)
        .expect("OpenAPI parsing must succeed");
    let elapsed = start.elapsed();

    println!("OpenAPI parsed in {elapsed:?} at zero token cost");
    assert!(
        elapsed.as_millis() < 500,
        "Must parse mechanically in < 500ms"
    );

    // 1. Root requirement assertion
    let root = result
        .root_requirement()
        .expect("Must synthesize root REQUIREMENT node");
    assert_eq!(root.node_type.as_deref(), Some("REQUIREMENT"));
    assert_eq!(root.primary_node_key(), Some("API-PETSTORE-SERVICE"));
    assert!(root.parent_heading_chunk_id.is_none());
    assert_eq!(root.trust_tier, "EXTERNALLY_IMPORTED");

    // 2. Extracted specifications assertion
    let op_keys: Vec<&str> = result
        .chunks
        .iter()
        .filter(|c| c.node_type.as_deref() == Some("SPECIFICATION"))
        .filter_map(|c| c.primary_node_key())
        .collect();

    assert!(op_keys.contains(&"API-GET-PETS"), "Missing API-GET-PETS");
    assert!(op_keys.contains(&"API-POST-PETS"), "Missing API-POST-PETS");
    assert!(
        op_keys.contains(&"API-GET-PETS-PETID"),
        "Missing API-GET-PETS-PETID"
    );
    assert!(
        op_keys.contains(&"API-DELETE-PETS-PETID"),
        "Missing API-DELETE-PETS-PETID"
    );
    assert!(op_keys.contains(&"SCHEMA-PET"), "Missing SCHEMA-PET");
    assert!(op_keys.contains(&"SCHEMA-ERROR"), "Missing SCHEMA-ERROR");

    // 3. Invariant INV-1 Ancestry: Every child must have upward DERIVED_FROM edge to root
    for chunk in &result.chunks {
        if chunk.node_type.as_deref() != Some("REQUIREMENT") {
            assert_eq!(
                chunk.parent_heading_chunk_id.as_deref(),
                Some(root.ast_anchor.as_str()),
                "Child chunk {} must link upward to root anchor",
                chunk.ast_anchor
            );
        }
    }

    assert_eq!(result.edges.len(), result.chunks.len() - 1);
    for edge in &result.edges {
        assert_eq!(edge.to_anchor, root.ast_anchor);
        assert_eq!(edge.edge_type, "DERIVED_FROM");
    }
}

#[test]
fn test_asyncapi_scaffolding_extraction() {
    let asyncapi_yaml = r#"
asyncapi: '2.6.0'
info:
  title: Order Events Stream
  version: 2.0.0
  description: Event-driven broker for order updates.
channels:
  orders/created:
    description: Emitted when an order is submitted.
  orders/shipped:
    description: Emitted when an order is shipped.
"#;

    let adapter = OpenApiAdapter::new();
    assert!(adapter.can_handle("specs/orders.asyncapi.yaml", asyncapi_yaml));

    let result = adapter
        .parse("specs/orders.asyncapi.yaml", asyncapi_yaml)
        .expect("AsyncAPI parsing must succeed");

    let root = result
        .root_requirement()
        .expect("Must synthesize root REQUIREMENT");
    assert_eq!(root.primary_node_key(), Some("API-ORDER-EVENTS-STREAM"));

    let channel_keys: Vec<&str> = result
        .chunks
        .iter()
        .filter(|c| c.node_type.as_deref() == Some("SPECIFICATION"))
        .filter_map(|c| c.primary_node_key())
        .collect();

    assert!(channel_keys.contains(&"ASYNCAPI-CHANNEL-ORDERS-CREATED"));
    assert!(channel_keys.contains(&"ASYNCAPI-CHANNEL-ORDERS-SHIPPED"));

    for edge in &result.edges {
        assert_eq!(edge.to_anchor, root.ast_anchor);
        assert_eq!(edge.edge_type, "DERIVED_FROM");
    }
}

#[test]
fn test_adr_scaffolding_extraction() {
    let nygard_adr = r#"
# 12. Use PostgreSQL for Graph and Relational Storage

Date: 2026-10-04

## Status

Accepted

## Context

We need a unified storage engine supporting ACID transactions, recursive CTEs, and pgvector.

## Decision

We will standardize strictly on PostgreSQL 16+ with pgvector.

## Consequences

Single-engine simplicity, eliminating multi-database synchronization overhead.
"#;

    let madr_adr = r#"
# Use Axum for HTTP and MCP Gateway

* status: accepted
* date: 2026-10-04
* deciders: Core Architecture Team

## Context and Problem Statement

We need high-performance asynchronous HTTP and SSE routing.

## Decision Outcome

Chosen option: "Axum 0.8", because it integrates cleanly with Tokio and Tower.
"#;

    let adapter = AdrAdapter::new();
    assert!(adapter.can_handle("docs/adr/0012-use-postgresql.md", nygard_adr));
    assert!(adapter.can_handle("adr/0042-use-axum.md", madr_adr));

    // Test Nygard ADR
    let res_nygard = adapter
        .parse("docs/adr/0012-use-postgresql.md", nygard_adr)
        .expect("Nygard ADR parse must succeed");

    let root_nygard = res_nygard
        .root_requirement()
        .expect("Must synthesize root REQUIREMENT");
    assert_eq!(root_nygard.primary_node_key(), Some("REQ-ADR-COLLECTION"));

    let dec_node = res_nygard
        .chunks
        .iter()
        .find(|c| c.node_type.as_deref() == Some("DECISION"))
        .expect("Must extract DECISION node");
    assert_eq!(dec_node.primary_node_key(), Some("ADR-0012"));
    assert_eq!(
        dec_node.parent_heading_chunk_id.as_deref(),
        Some(root_nygard.ast_anchor.as_str())
    );

    // Test MADR
    let res_madr = adapter
        .parse("docs/adr/0042-use-axum.md", madr_adr)
        .expect("MADR parse must succeed");
    let dec_madr = res_madr
        .chunks
        .iter()
        .find(|c| c.node_type.as_deref() == Some("DECISION"))
        .expect("Must extract DECISION node");
    assert_eq!(dec_madr.primary_node_key(), Some("ADR-0042"));
}

#[test]
fn test_sdd_scaffolding_extraction() {
    let adapter = SddAdapter::new();

    // 1. BDD Feature
    let bdd_content = r#"
Feature: User Authentication and Token Issuance
  As a registered developer
  I want to authenticate with an API token
  So that I can access protected endpoints.

  Scenario: Successful Token Authentication
    Given a registered developer with active credentials
    When submitting an HTTP request with Authorization Bearer token
    Then the gateway responds with HTTP 200 OK.

  Scenario: Revoked Token Rejection
    Given a revoked or expired token
    When submitting an HTTP request
    Then the gateway responds with HTTP 401 Unauthorized.
"#;

    assert!(adapter.can_handle("tests/features/auth.feature", bdd_content));
    let bdd_res = adapter
        .parse("tests/features/auth.feature", bdd_content)
        .expect("BDD parsing must succeed");

    let bdd_root = bdd_res
        .root_requirement()
        .expect("Must synthesize root REQUIREMENT from Feature");
    assert_eq!(
        bdd_root.primary_node_key(),
        Some("REQ-USER-AUTHENTICATION-AND-TOKEN-ISSUANCE")
    );

    let scenarios: Vec<&str> = bdd_res
        .chunks
        .iter()
        .filter(|c| c.node_type.as_deref() == Some("SPECIFICATION"))
        .filter_map(|c| c.primary_node_key())
        .collect();

    assert!(scenarios.contains(&"SPEC-SUCCESSFUL-TOKEN-AUTHENTICATION"));
    assert!(scenarios.contains(&"SPEC-REVOKED-TOKEN-REJECTION"));

    // 2. AGENTS.md steering file
    let agents_md = r#"
# Agent Steering Guidelines

## Invariant Adherence
Agents must maintain Invariants INV-1 through INV-9.

## Non-Destructive Mutations
Active normative requirements cannot be updated in-place.
"#;

    assert!(adapter.can_handle("AGENTS.md", agents_md));
    let agent_res = adapter
        .parse("AGENTS.md", agents_md)
        .expect("AGENTS.md parsing must succeed");

    let agent_root = agent_res
        .root_requirement()
        .expect("Must synthesize root REQUIREMENT from AGENTS.md");
    assert_eq!(agent_root.primary_node_key(), Some("REQ-AGENT-STEERING"));

    let agent_specs: Vec<&str> = agent_res
        .chunks
        .iter()
        .filter(|c| c.node_type.as_deref() == Some("SPECIFICATION"))
        .filter_map(|c| c.primary_node_key())
        .collect();
    assert!(agent_specs.contains(&"SPEC-STEERING-INVARIANT-ADHERENCE"));
    assert!(agent_specs.contains(&"SPEC-STEERING-NON-DESTRUCTIVE-MUTATIONS"));

    // 3. AWS Kiro steering file
    let kiro_md = r#"
# Core Design Principles

## Separation of Concerns
Components must maintain decoupled failure domains.
"#;

    assert!(adapter.can_handle(".kiro/steering/core.md", kiro_md));
    let kiro_res = adapter
        .parse(".kiro/steering/core.md", kiro_md)
        .expect("Kiro parsing must succeed");
    let kiro_root = kiro_res.root_requirement().expect("Kiro root REQUIREMENT");
    assert_eq!(
        kiro_root.primary_node_key(),
        Some("KIRO-CORE-DESIGN-PRINCIPLES")
    );
}

#[tokio::test]
async fn test_authority_inheritance_pipeline_e2e() {
    let _guard = TEST_MUTEX.lock().await;

    let database_url = db::ensure_test_database_ready()
        .await
        .expect("Failed to prepare test database");
    let (mut client, _handle) = db::connect(&database_url)
        .await
        .expect("Failed to connect to database");
    db::run_migrations(&mut client)
        .await
        .expect("Failed to run migrations");

    let repo = StorageRepo::from_database_url(&database_url).expect("Failed to create repo");
    let temp_repo = TempRepoGuard::new();
    let (write_handle, read_handle) =
        start_git_actor(&temp_repo.path, 64).expect("Failed to start git actor");

    // Clean up test paths
    client
        .execute(
            "DELETE FROM graph_nodes WHERE doc_path LIKE 'specs/test-scaffold-%';",
            &[],
        )
        .await
        .unwrap();
    client
        .execute(
            "DELETE FROM ingestion_jobs WHERE doc_path LIKE 'specs/test-scaffold-%';",
            &[],
        )
        .await
        .unwrap();

    let openapi_content = r#"
openapi: 3.1.0
info:
  title: User Authority API
  version: 1.0.0
  description: API imported with pre-governed authority.
paths:
  /accounts:
    get:
      summary: Retrieve accounts
"#;

    let doc_path = "specs/test-scaffold-authority.yaml";
    let commit_res = write_handle
        .commit_document(doc_path, openapi_content)
        .await
        .expect("Git commit must succeed");

    // 1. Submit ingestion job with pre-governed authority source
    let job_id = Uuid::new_v4();
    let caller_agent_id = "branch_webhook_admin";
    let token_fingerprint = "token_hash_abc123";
    let authority_source = "BRANCH_PROTECTED_CODEOWNERS";

    client
        .execute(
            "INSERT INTO ingestion_jobs (job_id, doc_path, document_hash, status, created_by, token_fingerprint, authority_source) \
             VALUES ($1, $2, $3, 'QUEUED', $4, $5, $6);",
            &[
                &job_id,
                &doc_path,
                &commit_res.blob_hash,
                &caller_agent_id,
                &token_fingerprint,
                &authority_source,
            ],
        )
        .await
        .expect("Failed to insert ingestion job");

    // 2. Execute decomposition worker tick
    let processed = process_one_job(repo.pool(), &read_handle)
        .await
        .expect("Worker tick failed");
    assert!(processed, "Worker must process the job");

    // 3. Verify job status transitioned directly to APPROVED (bypassing DRAFT staging)
    let job_row = client
        .query_one(
            "SELECT status, attributes FROM ingestion_jobs WHERE job_id = $1;",
            &[&job_id],
        )
        .await
        .expect("Query job failed");
    let job_status: String = job_row.get("status");
    assert_eq!(
        job_status, "APPROVED",
        "Job must auto-promote directly to APPROVED"
    );

    // 4. Verify nodes were directly inserted in ACTIVE state with caller attribution
    let node_rows = client
        .query(
            "SELECT id, node_key, node_type, lifecycle_state, created_by, trust_tier \
             FROM graph_nodes \
             WHERE job_id = $1 \
             ORDER BY byte_start ASC, id ASC;",
            &[&job_id],
        )
        .await
        .expect("Query nodes failed");

    assert!(
        node_rows.len() >= 2,
        "Expected at least root requirement and path specification"
    );

    let mut has_root_req = false;
    let mut has_spec = false;

    for row in &node_rows {
        let l_state: String = row.get("lifecycle_state");
        let creator: String = row.get("created_by");
        let node_type: String = row.get("node_type");
        let trust_tier: String = row.get("trust_tier");

        assert_eq!(
            l_state, "ACTIVE",
            "Authority inherited nodes must be directly ACTIVE"
        );
        assert_eq!(
            creator, caller_agent_id,
            "Node created_by must match caller identity attribution (INV-7)"
        );
        assert_eq!(
            trust_tier, "EXTERNALLY_IMPORTED",
            "Trust tier must be EXTERNALLY_IMPORTED"
        );

        if node_type == "REQUIREMENT" {
            has_root_req = true;
        } else if node_type == "SPECIFICATION" {
            has_spec = true;
        }
    }

    assert!(has_root_req, "Must contain active root REQUIREMENT node");
    assert!(has_spec, "Must contain active child SPECIFICATION node");

    // 5. Verify upward DERIVED_FROM edges are ACTIVE
    let edge_rows = client
        .query(
            "SELECT edge_type, lifecycle_state, created_by FROM graph_edges \
             WHERE from_node_id IN (SELECT id FROM graph_nodes WHERE job_id = $1);",
            &[&job_id],
        )
        .await
        .expect("Query edges failed");

    assert!(!edge_rows.is_empty(), "Must have upward hierarchy edges");
    for row in &edge_rows {
        let edge_type: String = row.get("edge_type");
        let l_state: String = row.get("lifecycle_state");
        let creator: String = row.get("created_by");

        assert_eq!(edge_type, "DERIVED_FROM");
        assert_eq!(l_state, "ACTIVE");
        assert_eq!(creator, caller_agent_id);
    }

    // 6. Verify audit_ledger contains SYSTEM_AUTHORITY_IMPORT event with exact credentials
    let audit_rows = client
        .query(
            "SELECT event_type, actor_id, token_fingerprint, entity_type \
             FROM audit_ledger \
             WHERE entity_id = $1;",
            &[&job_id],
        )
        .await
        .expect("Query audit failed");

    assert_eq!(audit_rows.len(), 1, "Must have exactly 1 audit event");
    let audit_row = &audit_rows[0];
    let event_type: String = audit_row.get("event_type");
    let actor_id: String = audit_row.get("actor_id");
    let token_fp: String = audit_row.get("token_fingerprint");
    let entity_type: String = audit_row.get("entity_type");

    assert_eq!(event_type, "SYSTEM_AUTHORITY_IMPORT");
    assert_eq!(actor_id, caller_agent_id);
    assert_eq!(token_fp, token_fingerprint);
    assert_eq!(entity_type, "INGESTION_JOB");
}

#[tokio::test]
async fn test_gateway_authority_source_authorization_guard() {
    let _guard = TEST_MUTEX.lock().await;

    let database_url = db::ensure_test_database_ready()
        .await
        .expect("Failed to prepare test database");
    let (mut client, _handle) = db::connect(&database_url)
        .await
        .expect("Failed to connect to database");
    db::run_migrations(&mut client)
        .await
        .expect("Failed to run migrations");

    let repo = StorageRepo::from_database_url(&database_url).expect("Failed to create repo");
    let temp_repo = TempRepoGuard::new();
    let (write_handle, read_handle) =
        start_git_actor(&temp_repo.path, 64).expect("Failed to start git actor");

    let app_state =
        tks::gateway::AppState::new(repo.pool().clone(), repo, write_handle, read_handle);

    // 1. Untrusted AGENT caller attempting to claim authority_source
    let untrusted_agent = AuthenticatedAgent {
        agent_id: "untrusted_agent_99".to_string(),
        actor_type: "AGENT".to_string(),
    };

    let req_untrusted = IngestDocumentRequest {
        doc_path: "specs/test-scaffold-untrusted.yaml".to_string(),
        content: "openapi: 3.0.0\ninfo:\n  title: Untrusted\n  version: 1.0\npaths: {}".to_string(),
        authority_source: Some("BRANCH_PROTECTED_CODEOWNERS".to_string()),
    };

    let resp = tks::gateway::routes::documents::ingest_document(
        axum::extract::State(app_state.clone()),
        untrusted_agent,
        axum::Json(req_untrusted),
    )
    .await
    .expect("Ingest call should succeed");

    assert_eq!(resp.status(), axum::http::StatusCode::ACCEPTED);

    // Verify authority_source was stripped (is NULL) in ingestion_jobs
    let row_untrusted = client
        .query_one(
            "SELECT authority_source, created_by FROM ingestion_jobs WHERE doc_path = 'specs/test-scaffold-untrusted.yaml' ORDER BY created_at DESC LIMIT 1;",
            &[],
        )
        .await
        .unwrap();

    let auth_src: Option<String> = row_untrusted.get("authority_source");
    let created_by: String = row_untrusted.get("created_by");
    assert!(
        auth_src.is_none(),
        "Untrusted AGENT authority_source must be stripped by gateway guard"
    );
    assert_eq!(created_by, "untrusted_agent_99");

    // 2. Verified HUMAN caller with authority_source
    let human_caller = AuthenticatedAgent {
        agent_id: "human_supervisor".to_string(),
        actor_type: "HUMAN".to_string(),
    };

    let req_human = IngestDocumentRequest {
        doc_path: "specs/test-scaffold-human.yaml".to_string(),
        content: "openapi: 3.0.0\ninfo:\n  title: Human Verified\n  version: 1.0\npaths: {}"
            .to_string(),
        authority_source: Some("COMMITTED_SPEC_MAIN".to_string()),
    };

    let resp_human = tks::gateway::routes::documents::ingest_document(
        axum::extract::State(app_state),
        human_caller,
        axum::Json(req_human),
    )
    .await
    .expect("Ingest call should succeed");

    assert_eq!(resp_human.status(), axum::http::StatusCode::ACCEPTED);

    // Verify authority_source is preserved for HUMAN caller
    let row_human = client
        .query_one(
            "SELECT authority_source, created_by FROM ingestion_jobs WHERE doc_path = 'specs/test-scaffold-human.yaml' ORDER BY created_at DESC LIMIT 1;",
            &[],
        )
        .await
        .unwrap();

    let auth_src_human: Option<String> = row_human.get("authority_source");
    let created_by_human: String = row_human.get("created_by");
    assert_eq!(
        auth_src_human.as_deref(),
        Some("COMMITTED_SPEC_MAIN"),
        "HUMAN caller authority_source must be preserved"
    );
    assert_eq!(created_by_human, "human_supervisor");
}
