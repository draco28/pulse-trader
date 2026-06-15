-- VS-1.1.4 work-1.04 — 0002 down: reverse the strategy(name) index.
-- `IF EXISTS` keeps the down direction idempotent (mirrors the up's IF NOT EXISTS).
DROP INDEX IF EXISTS idx_strategy_name;
