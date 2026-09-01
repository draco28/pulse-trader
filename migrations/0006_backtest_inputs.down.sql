-- r1.s3.w2 — 0006 down: reverse-order teardown (mirrors `0003_backtest_runs.down.sql`).
--
-- The TRIGGER comes first and that ordering is load-bearing, not cosmetic: its WHEN
-- clause references all eight columns, and SQLite refuses to DROP a column that a
-- trigger body or condition still names. Dropping the columns first would fail the
-- migration halfway and leave the table in the shape neither version expects.
--
-- Columns then drop in reverse declaration order, purely so a reader can diff this
-- file against the up migration line for line.

DROP TRIGGER IF EXISTS backtest_run_inputs_complete;

ALTER TABLE backtest_run DROP COLUMN funding_config;
ALTER TABLE backtest_run DROP COLUMN slippage_bps;
ALTER TABLE backtest_run DROP COLUMN taker_fee_bps;
ALTER TABLE backtest_run DROP COLUMN htf_data_version;
ALTER TABLE backtest_run DROP COLUMN htf_timeframe;
ALTER TABLE backtest_run DROP COLUMN primary_data_version;
ALTER TABLE backtest_run DROP COLUMN primary_timeframe;
ALTER TABLE backtest_run DROP COLUMN pair;
