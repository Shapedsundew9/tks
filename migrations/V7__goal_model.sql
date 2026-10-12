-- V7__goal_model.sql: Goal Model & Source Excerpt schema expansion (EB-3)

-- 1. Expand chk_node_type to allow 'GOAL' and 'SOURCE_EXCERPT'
ALTER TABLE graph_nodes DROP CONSTRAINT IF EXISTS chk_node_type;
ALTER TABLE graph_nodes ADD CONSTRAINT chk_node_type CHECK (
    (lifecycle_state = 'DRAFT' AND node_type IN ('REQUIREMENT', 'SPECIFICATION', 'TASK', 'VERIFICATION', 'DECISION', 'UNCLASSIFIED', 'CODE_COMMIT', 'GOAL', 'SOURCE_EXCERPT'))
    OR (lifecycle_state != 'DRAFT' AND node_type IN ('REQUIREMENT', 'SPECIFICATION', 'TASK', 'VERIFICATION', 'DECISION', 'CODE_COMMIT', 'GOAL', 'SOURCE_EXCERPT'))
);

-- 2. Add check constraint ensuring GOAL nodes specify valid goal_kind in attributes
ALTER TABLE graph_nodes DROP CONSTRAINT IF EXISTS chk_goal_kind;
ALTER TABLE graph_nodes ADD CONSTRAINT chk_goal_kind CHECK (
    node_type != 'GOAL' OR (
        attributes ? 'goal_kind' AND
        attributes->>'goal_kind' IN ('ACHIEVE', 'MAINTAIN', 'AVOID', 'SOFTGOAL')
    )
);

-- 3. Partial index on edge_type for active structural and goal model edges
CREATE INDEX IF NOT EXISTS idx_graph_edges_type_active
    ON graph_edges (edge_type)
    WHERE lifecycle_state = 'ACTIVE';
