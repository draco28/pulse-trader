-- VS-1.1.4 work-1.04 — 0002: a reversible index on strategy(name).
--
-- Real, useful (name lookup / list_strategies ordering) AND the second versioned
-- step so a db at 0001 is detectably behind, exercising the backup-before-migrate
-- protocol with a genuine migration (not a synthetic one). `IF NOT EXISTS` keeps
-- the up direction idempotent; the matching down is `DROP INDEX IF EXISTS`.
CREATE INDEX IF NOT EXISTS idx_strategy_name ON strategy(name);
