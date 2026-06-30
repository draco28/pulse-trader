-- VS-1.2.4 work-4.03 — 0003: the durable backtest system-of-record floor.
-- Adds `backtest_run` + `trade` EXACTLY per the slice interface contract README C4
-- (the single source of truth for these shapes). 4.04's BacktestRunRepository
-- projects typed columns INTO these tables; 4.05's CLI reads rows back OUT.
--
-- NFR-2 (Decimal-as-TEXT, never f64/floating-point): every money / Decimal column is TEXT,
-- stored as the same `.normalize()`d canonical text the content hash uses, so a
-- reloaded run re-derives a byte-identical `result_content_hash` (the 4.04 integrity
-- guard). The two f64-derived stats (`sharpe`/`sortino`) and `profit_factor` are
-- TEXT NULL (audit C10: f64 `to_string()` round-trip, finite-or-NULL, never NaN/Inf;
-- NULL for the `trade_count < 2` / zero-denominator `None` cases). Counts are INTEGER.
-- There is NO `equity_curve_point` table — the equity curve is reconstructed on read
-- from the `seq`-ordered `trade` rows (C2 / audit C4).
--
-- `trade.backtest_run_id REFERENCES backtest_run(id)` is resolved lazily by SQLite at
-- row-insert/check time, so declaring `backtest_run` before `trade` is valid (mirrors
-- the `0001_init.up.sql:5-9` forward-reference note).

CREATE TABLE backtest_run (
  id                      TEXT PRIMARY KEY NOT NULL,
  strategy_version_id     TEXT NOT NULL REFERENCES strategy_version(id),
  schema_version          TEXT NOT NULL,                    -- run-row schema tag (#68)
  created_at              TEXT NOT NULL,                    -- injected Clock (RFC3339 UTC)
  engine_fingerprint      TEXT NOT NULL,
  engine_target           TEXT NOT NULL,                    -- #49 cohort key
  result_content_hash     TEXT NOT NULL,                    -- integrity / tamper guard
  starting_equity         TEXT NOT NULL,                    -- Decimal-as-TEXT (NFR-2)
  net_pnl                 TEXT NOT NULL,                    -- Decimal-as-TEXT (NFR-2)
  fees_total              TEXT NOT NULL,                    -- Decimal-as-TEXT (NFR-2)
  funding_total           TEXT NOT NULL,                    -- Decimal-as-TEXT, signed (NFR-2)
  slippage_total          TEXT NOT NULL,                    -- Decimal-as-TEXT (NFR-2)
  -- derived SummaryStats columns (read-only; computed by 4.01/4.02, written by 4.04)
  expectancy              TEXT,                             -- Decimal-as-TEXT (NFR-2)
  win_rate                TEXT,                             -- Decimal-as-TEXT (NFR-2)
  profit_factor           TEXT,                             -- Decimal-as-TEXT NULL (None when gross_loss==0)
  gross_profit            TEXT,                             -- Decimal-as-TEXT (NFR-2)
  gross_loss              TEXT,                             -- Decimal-as-TEXT (NFR-2)
  avg_win                 TEXT,                             -- Decimal-as-TEXT (NFR-2)
  avg_loss                TEXT,                             -- Decimal-as-TEXT (NFR-2)
  max_drawdown            TEXT,                             -- Decimal-as-TEXT (NFR-2)
  trade_count             INTEGER,
  wins                    INTEGER,
  losses                  INTEGER,
  breakeven               INTEGER,
  max_win_streak          INTEGER,
  max_loss_streak         INTEGER,
  sharpe                  TEXT,                             -- f64-derived TEXT NULL (audit C10; NFR-2, TEXT not floating-point)
  sortino                 TEXT,                             -- f64-derived TEXT NULL (audit C10; NFR-2, TEXT not floating-point)
  regime_breakdown        TEXT,                             -- inline JSON of the aggregate
  skipped_sub_lot         INTEGER,
  skipped_sub_notional    INTEGER,
  skipped_leverage_capped INTEGER
);

CREATE TABLE trade (
  id                TEXT PRIMARY KEY NOT NULL,
  backtest_run_id   TEXT NOT NULL REFERENCES backtest_run(id),
  seq               INTEGER NOT NULL,                       -- 0-based chronological
  direction         TEXT,
  qty               TEXT,                                   -- Decimal-as-TEXT (NFR-2)
  entry_price       TEXT,                                   -- Decimal-as-TEXT (NFR-2)
  exit_price        TEXT,                                   -- Decimal-as-TEXT (NFR-2)
  entry_signal_time INTEGER,                                -- epoch ms
  entry_fill_time   INTEGER,                                -- epoch ms
  exit_signal_time  INTEGER,                                -- epoch ms
  exit_fill_time    INTEGER,                                -- epoch ms
  fees_total        TEXT,                                   -- Decimal-as-TEXT (NFR-2)
  funding_total     TEXT,                                   -- Decimal-as-TEXT (NFR-2)
  slippage_total    TEXT,                                   -- Decimal-as-TEXT (NFR-2)
realized_pnl        TEXT,                                   -- Decimal-as-TEXT (NFR-2)
realized_r          TEXT,                                   -- Decimal-as-TEXT (NFR-2)
  mfe_r             TEXT,                                   -- Decimal-as-TEXT (NFR-2)
  mae_r             TEXT,                                   -- Decimal-as-TEXT (NFR-2)
  exit_reason       TEXT,
  source            TEXT,
  -- #49: f64-derived, cohort-keyed; deterministic-on-pinned-toolchain, NOT byte-portable.
  regime            TEXT,
  fills             TEXT                                    -- inline JSON of Vec<Fill>
);

-- Immutability (FR-4, live-capital provenance): append-only rows. SQLite has no
-- combined trigger form, so each table needs SEPARATE BEFORE UPDATE / BEFORE DELETE
-- triggers, each RAISE(ABORT, ...) — mirroring `strategy_version` at
-- `0001_init.up.sql:41-44`. RAISE(ABORT) rolls the statement back and surfaces as a
-- sqlx error → DataError::Db on the caller side.
CREATE TRIGGER backtest_run_no_update BEFORE UPDATE ON backtest_run
  BEGIN SELECT RAISE(ABORT, 'backtest_run is immutable'); END;
CREATE TRIGGER backtest_run_no_delete BEFORE DELETE ON backtest_run
  BEGIN SELECT RAISE(ABORT, 'backtest_run is immutable'); END;
CREATE TRIGGER trade_no_update BEFORE UPDATE ON trade
  BEGIN SELECT RAISE(ABORT, 'trade is immutable'); END;
CREATE TRIGGER trade_no_delete BEFORE DELETE ON trade
  BEGIN SELECT RAISE(ABORT, 'trade is immutable'); END;

-- Indexes (C4 / C6): the composite makes `latest_run_for_version` /
-- `list_runs_for_version` `ORDER BY created_at[, id]` index-served + total; the
-- per-run index makes the `seq`-ordered `get_trades` fetch index-served.
CREATE INDEX idx_br_strategy_version ON backtest_run(strategy_version_id, created_at, id);
CREATE INDEX idx_trade_run ON trade(backtest_run_id, seq);
