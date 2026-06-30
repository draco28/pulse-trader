-- VS-1.2.4 work-4.03 — 0003 down: FK-safe reverse-order teardown (D4). Drops in
-- REVERSE dependency order: triggers first, then indexes, then `trade` (the
-- referencing table whose FK points at `backtest_run`), then `backtest_run` (the
-- referenced table). Dropping `backtest_run` first while `trade`'s FK still
-- references it would be ill-formed. Each DROP uses IF EXISTS for idempotent
-- re-runs (mirrors `0001_init.down.sql`).

DROP TRIGGER IF EXISTS trade_no_delete;
DROP TRIGGER IF EXISTS trade_no_update;
DROP TRIGGER IF EXISTS backtest_run_no_delete;
DROP TRIGGER IF EXISTS backtest_run_no_update;
DROP INDEX IF EXISTS idx_trade_run;
DROP INDEX IF EXISTS idx_br_strategy_version;
DROP TABLE IF EXISTS trade;
DROP TABLE IF EXISTS backtest_run;
