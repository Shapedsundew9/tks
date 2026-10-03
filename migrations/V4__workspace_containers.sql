-- V4__workspace_containers.sql: Multi-agent workspace containers and ephemeral branch isolation (WP-3.2, PHASE3-003)

CREATE TABLE IF NOT EXISTS workspaces (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_name VARCHAR(128) NOT NULL,
    owner_agent VARCHAR(64) NOT NULL,
    base_event_seq BIGINT NOT NULL,
    status VARCHAR(20) NOT NULL DEFAULT 'ACTIVE'
        CHECK (status IN ('ACTIVE', 'MERGED', 'DISCARDED')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    attributes JSONB NOT NULL DEFAULT '{}'
);

CREATE INDEX IF NOT EXISTS idx_workspaces_owner ON workspaces(owner_agent);
CREATE INDEX IF NOT EXISTS idx_workspaces_status ON workspaces(status);

-- Partial index on graph_nodes for workspace candidate lookup
CREATE INDEX IF NOT EXISTS idx_graph_nodes_workspace_id
    ON graph_nodes ((attributes->>'workspace_id'))
    WHERE (attributes->>'workspace_id') IS NOT NULL;

-- Add attributes JSONB column to graph_edges and index workspace_id
ALTER TABLE graph_edges ADD COLUMN IF NOT EXISTS attributes JSONB NOT NULL DEFAULT '{}';

CREATE INDEX IF NOT EXISTS idx_graph_edges_workspace_id
    ON graph_edges ((attributes->>'workspace_id'))
    WHERE (attributes->>'workspace_id') IS NOT NULL;
