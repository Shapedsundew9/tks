# Technical Implementation Backlog

This document captures vital component-level designs, library evaluation tasks, sprint-level tactical choices, and downstream implementation details deferred from architectural review.

---

## TB-1

* **ID:** TB-1
* **Title:** Bare Git Repository Direct ODB Ingestion and Volume Backup Synchronization
* **Origin:** LD-8, iteration 1
* **Description/Tactical Details:**
  1. **Low-Level ODB Direct Writes:** In the Git document adapter module (`src/storage/git/`), configure the `git2` crate to interact exclusively with a bare Git repository (`git init --bare`). Implement direct object-database (ODB) writes via `git_blob_create_from_buffer` for raw document streams and construct tree/commit objects directly within the ODB backend. This bypasses the Git working tree and index (`.git/index`), completely eliminating index lock collisions (`.git/index.lock`) during concurrent document ingestion calls.
  2. **Dedicated Storage Volume Binding:** In `docker-compose.yml` and container deployment manifests, configure a dedicated, persistent filesystem volume mount for the bare Git repository co-located with the PostgreSQL persistent data volume, ensuring Git blob storage persists across container recreation and redeployment cycles.
  3. **Point-in-Time Backup & Restore Synchronization:** Author operational shell scripts in `scripts/` to orchestrate coordinated point-in-time backups: trigger a PostgreSQL WAL checkpoint / `pg_dump` and execute a simultaneous snapshot of the bare Git repository object database (`objects/` and `refs/`), preventing referential drift between PostgreSQL blob hash columns and underlying Git blobs.
