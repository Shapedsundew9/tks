-- V2__dev_seed.sql: Development environment seed provisioning well-known token tks_dev_token
INSERT INTO agent_identities (agent_id, token_hash, actor_type, is_active)
VALUES ('tks_dev_token', 'tks_dev_token', 'HUMAN', TRUE)
ON CONFLICT (agent_id) DO NOTHING;
