-- V3__ingestion_jobs_attributes.sql: Add attributes JSONB column to ingestion_jobs (TB-7.5)
ALTER TABLE ingestion_jobs ADD COLUMN IF NOT EXISTS attributes JSONB NOT NULL DEFAULT '{}';
