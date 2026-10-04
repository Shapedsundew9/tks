-- V5__phase4_foundational_traceability.sql: Phase 4 foundational traceability & trust-tier ingestion (WP-4.1)

-- 1. Add trust_tier to graph_nodes with check constraint chk_trust_tier
ALTER TABLE graph_nodes ADD COLUMN IF NOT EXISTS trust_tier VARCHAR(32) NOT NULL DEFAULT 'HUMAN_APPROVED';

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'chk_trust_tier'
    ) THEN
        ALTER TABLE graph_nodes ADD CONSTRAINT chk_trust_tier CHECK (
            trust_tier IN ('HUMAN_APPROVED', 'AGENT_DRAFTED', 'EXTERNALLY_IMPORTED')
        );
    END IF;
END $$;

-- 2. Expand chk_node_type to allow 'CODE_COMMIT'
ALTER TABLE graph_nodes DROP CONSTRAINT IF EXISTS chk_node_type;
ALTER TABLE graph_nodes ADD CONSTRAINT chk_node_type CHECK (
    (lifecycle_state = 'DRAFT' AND node_type IN ('REQUIREMENT', 'SPECIFICATION', 'TASK', 'VERIFICATION', 'DECISION', 'UNCLASSIFIED', 'CODE_COMMIT'))
    OR (lifecycle_state != 'DRAFT' AND node_type IN ('REQUIREMENT', 'SPECIFICATION', 'TASK', 'VERIFICATION', 'DECISION', 'CODE_COMMIT'))
);

-- 3. Ingestion jobs provenance and authority columns (WP-4.1 / WP-4.2 preconditions)
ALTER TABLE ingestion_jobs ADD COLUMN IF NOT EXISTS created_by VARCHAR(64) NOT NULL DEFAULT 'system';
ALTER TABLE ingestion_jobs ADD COLUMN IF NOT EXISTS token_fingerprint VARCHAR(128);
ALTER TABLE ingestion_jobs ADD COLUMN IF NOT EXISTS authority_source VARCHAR(64);
