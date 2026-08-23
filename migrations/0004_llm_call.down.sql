-- VS-1.3.1 work-1.02 — 0004 down: reverse-order teardown. Drops the triggers first,
-- then the index, then the `llm_call` table (there is no FK this slice, so the order
-- is a straight mirror of `0003_backtest_runs.down.sql`). Each DROP uses IF EXISTS
-- for idempotent re-runs (mirrors `0001_init.down.sql`). AC-11's migration-roundtrip
-- suite proves up/down reversibility with `0004` in the embedded set.

DROP TRIGGER IF EXISTS llm_call_no_delete;
DROP TRIGGER IF EXISTS llm_call_no_update;
DROP INDEX IF EXISTS idx_llm_call_created_at;
DROP TABLE IF EXISTS llm_call;
