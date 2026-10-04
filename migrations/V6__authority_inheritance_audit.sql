-- V6__authority_inheritance_audit.sql: Widen audit_ledger columns for authority inheritance (WP-4.2 / TB-11)
ALTER TABLE audit_ledger ALTER COLUMN actor_type TYPE VARCHAR(32);
ALTER TABLE audit_ledger ALTER COLUMN token_fingerprint TYPE VARCHAR(128);
