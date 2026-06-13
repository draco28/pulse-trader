-- VS-1.1.4 work-1.01 — 0001_init teardown (FK-safe). Drops in REVERSE dependency
-- order: triggers first, then indexes, then strategy_version (the referencing
-- table whose FK points at strategy), then strategy. Dropping strategy first while
-- strategy_version's FK still references it would be ill-formed. Each DROP uses
-- IF EXISTS for idempotent re-runs. (1.04 owns the MIGRATOR.undo invocation; this
-- file is the reversible payload it executes.)

DROP TRIGGER IF EXISTS strategy_version_no_delete;
DROP TRIGGER IF EXISTS strategy_version_no_update;
DROP INDEX   IF EXISTS idx_sv_parent;
DROP INDEX   IF EXISTS idx_sv_strategy_id;
DROP TABLE   IF EXISTS strategy_version;
DROP TABLE   IF EXISTS strategy;
