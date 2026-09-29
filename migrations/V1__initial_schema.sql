-- V1__initial_schema.sql: Initial TKS relational and vector schema
CREATE EXTENSION IF NOT EXISTS vector;

CREATE TABLE ingestion_jobs (
    job_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    doc_path VARCHAR(255) NOT NULL,
    document_hash VARCHAR(64) NOT NULL,
    status VARCHAR(20) NOT NULL DEFAULT 'QUEUED',
    error_message TEXT,
    retry_count INT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE graph_nodes (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    node_key VARCHAR(64),
    node_type VARCHAR(32) NOT NULL,
    title TEXT,
    content TEXT,
    search_tsv tsvector GENERATED ALWAYS AS (
        to_tsvector('english', coalesce(node_key, '') || ' ' || coalesce(title, '') || ' ' || coalesce(content, ''))
    ) STORED,
    lifecycle_state VARCHAR(20) NOT NULL DEFAULT 'DRAFT',
    governance_policy VARCHAR(32) NOT NULL DEFAULT 'AUTONOMOUS_ELABORATION',
    created_by VARCHAR(64) NOT NULL DEFAULT 'system',
    job_id UUID REFERENCES ingestion_jobs(job_id) ON DELETE SET NULL,
    doc_path VARCHAR(255),
    doc_hash VARCHAR(64),
    byte_start INT,
    byte_end INT,
    attributes JSONB NOT NULL DEFAULT '{}',
    CONSTRAINT chk_node_type CHECK (
        (lifecycle_state = 'DRAFT' AND node_type IN ('REQUIREMENT', 'SPECIFICATION', 'TASK', 'VERIFICATION', 'DECISION', 'UNCLASSIFIED'))
        OR (lifecycle_state != 'DRAFT' AND node_type IN ('REQUIREMENT', 'SPECIFICATION', 'TASK', 'VERIFICATION', 'DECISION'))
    ),
    CONSTRAINT chk_lifecycle_state CHECK (
        lifecycle_state IN ('DRAFT', 'ACTIVE', 'SUPERSEDED', 'ARCHIVED', 'NEEDS_REVERIFICATION')
    ),
    CONSTRAINT chk_governance_policy CHECK (
        governance_policy IN ('AUTONOMOUS_ELABORATION', 'HUMAN_REVIEW_REQUIRED', 'LOCKED')
    )
);

CREATE UNIQUE INDEX idx_graph_nodes_node_key_active
    ON graph_nodes (node_key)
    WHERE lifecycle_state = 'ACTIVE' AND node_key IS NOT NULL;

CREATE INDEX idx_graph_nodes_draft_author
    ON graph_nodes (created_by)
    WHERE lifecycle_state = 'DRAFT';

CREATE INDEX idx_graph_nodes_doc_path
    ON graph_nodes (doc_path)
    WHERE doc_path IS NOT NULL;

CREATE INDEX idx_graph_nodes_search_tsv
    ON graph_nodes USING gin(search_tsv)
    WHERE lifecycle_state = 'ACTIVE';

CREATE TABLE graph_edges (
    edge_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    from_node_id UUID NOT NULL REFERENCES graph_nodes(id) ON DELETE CASCADE,
    to_node_id UUID NOT NULL REFERENCES graph_nodes(id) ON DELETE CASCADE,
    edge_type VARCHAR(32) NOT NULL,
    created_by VARCHAR(64) NOT NULL DEFAULT 'system',
    lifecycle_state VARCHAR(20) NOT NULL DEFAULT 'ACTIVE'
        CHECK (lifecycle_state IN ('DRAFT', 'ACTIVE', 'SUPERSEDED', 'REVERTED'))
);

CREATE UNIQUE INDEX idx_graph_edges_active_unique
    ON graph_edges (from_node_id, to_node_id, edge_type)
    WHERE lifecycle_state = 'ACTIVE';

CREATE INDEX idx_graph_edges_to_node_active
    ON graph_edges (to_node_id, from_node_id, edge_type)
    WHERE lifecycle_state = 'ACTIVE';

CREATE INDEX idx_graph_edges_draft_author
    ON graph_edges (from_node_id, created_by)
    WHERE lifecycle_state = 'DRAFT';

CREATE TABLE node_embeddings (
    node_id UUID PRIMARY KEY REFERENCES graph_nodes(id) ON DELETE CASCADE,
    content_hash VARCHAR(64) NOT NULL,
    embedding vector(384),
    status VARCHAR(20) NOT NULL DEFAULT 'PENDING'
        CHECK (status IN ('PENDING', 'PROCESSING', 'COMPLETED', 'FAILED')),
    retry_count INT NOT NULL DEFAULT 0,
    scheduled_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    error_message TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

CREATE INDEX idx_node_embeddings_vector
    ON node_embeddings USING hnsw (embedding vector_cosine_ops)
    WHERE status = 'COMPLETED';

CREATE INDEX idx_node_embeddings_pending
    ON node_embeddings (scheduled_at, retry_count)
    WHERE status = 'PENDING';

CREATE TABLE audit_ledger (
    event_seq BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    batch_id UUID NOT NULL DEFAULT gen_random_uuid(),
    event_id UUID NOT NULL DEFAULT gen_random_uuid(),
    event_type VARCHAR(32) NOT NULL,
    entity_id UUID NOT NULL,
    entity_type VARCHAR(32) NOT NULL,
    actor_id VARCHAR(64) NOT NULL,
    actor_type VARCHAR(16) NOT NULL,
    token_fingerprint VARCHAR(64) NOT NULL,
    delta JSONB NOT NULL,
    snapshot JSONB NOT NULL,
    draft_evolution_summary JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_audit_ledger_entity ON audit_ledger(entity_id, event_seq);
CREATE INDEX idx_audit_ledger_batch ON audit_ledger(batch_id, event_seq);
CREATE INDEX idx_audit_ledger_created_at ON audit_ledger(created_at);

CREATE TABLE agent_identities (
    agent_id VARCHAR(64) PRIMARY KEY,
    token_hash VARCHAR(128) NOT NULL,
    actor_type VARCHAR(16) NOT NULL,
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
