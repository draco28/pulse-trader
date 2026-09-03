-- r1.s3.w2 — 0006: durable backtest INPUT provenance (#110).
--
-- `0003` persists what a run PRODUCED (engine_fingerprint, engine_target,
-- result_content_hash, the money totals, the summary stats). It persists nothing
-- about what the run CONSUMED. Meanwhile `src/cli/backtest.rs` loads the HEAD
-- snapshot for (pair, timeframe) and `fetch-data` advances HEAD, so once HEAD
-- moves the immutable old Parquet files may still exist while NOTHING in the run
-- record says which of them produced the row. `engine_fingerprint` pins the
-- engine and `result_content_hash` detects tampering with the result; neither
-- pins the data. These eight columns close that gap.
--
-- RESERVED NUMBER, OUT OF ORDER. Release planning allocated `0005` to `r1.s2` and
-- `0006` to `r1.s3` while `r1.s1` shipped `0007`, so this migration arrives at
-- databases whose maximum applied version is already 7. `src/adapters/db/migrate.rs`
-- compares applied and embedded version SETS rather than maxima for exactly this
-- reason; a max comparison would report `AlreadyCurrent` and silently skip it.
--
-- NULLABLE ON PURPOSE (ADR-0018, planning audit C3). Rows written before this
-- migration cannot be backfilled truthfully — the snapshot identity they used is
-- not recoverable from anything stored — and ADR-0018 forbids rewriting immutable
-- records with invented facts. So legacy rows keep all eight NULL and read back as
-- `inputs: None`, an explicit "provenance unavailable", never a guess. SQLite also
-- cannot ADD COLUMN ... NOT NULL without a default, and a default here would BE
-- the guess.
--
-- NFR-2 (Decimal-as-TEXT): `taker_fee_bps` / `slippage_bps` store the same
-- `.normalize()`d canonical text every other Decimal column uses. `funding_config`
-- stores the domain enum's bare snake_case token (`snapshot_rates`), matching the
-- `direction` / `exit_reason` / `regime` precedent on `trade`.

ALTER TABLE backtest_run ADD COLUMN pair                 TEXT;  -- canonical Pair string
ALTER TABLE backtest_run ADD COLUMN primary_timeframe    TEXT;  -- Binance interval text (15m/4h)
ALTER TABLE backtest_run ADD COLUMN primary_data_version TEXT;  -- opaque DataVersion (ADR-0009)
ALTER TABLE backtest_run ADD COLUMN htf_timeframe        TEXT;  -- nullable; paired with the next
ALTER TABLE backtest_run ADD COLUMN htf_data_version     TEXT;  -- nullable; paired with the previous
ALTER TABLE backtest_run ADD COLUMN taker_fee_bps        TEXT;  -- Decimal-as-TEXT (NFR-2)
ALTER TABLE backtest_run ADD COLUMN slippage_bps         TEXT;  -- Decimal-as-TEXT (NFR-2)
ALTER TABLE backtest_run ADD COLUMN funding_config       TEXT;  -- snake_case enum token

-- INSERT-ONLY completeness guard, installed AFTER the columns exist.
--
-- A table-level CHECK would be the obvious tool and is the wrong one: the legacy
-- rows are all-NULL and must stay readable, so any constraint that evaluates over
-- existing rows would either reject the migration or force the guess this
-- migration exists to avoid. A BEFORE INSERT trigger sees only NEW rows, so old
-- rows are untouched and every FRESH write must be complete.
--
-- It guards two properties and deliberately nothing else:
--   1. the six required base/cost/funding columns are all present;
--   2. the HTF pair is all-or-nothing — a run either used a higher timeframe or it
--      did not, and "half an HTF selection" is not a state the domain can express.
-- It does NOT key on `schema_version` (the corrupt-schema read-rejection tests
-- hand-write rows with junk tags and must stay insertable), and it does NOT
-- constrain `funding_config`'s value: the product can emit exactly one variant,
-- fail-closed read decoding already rejects an unknown discriminant, and a second
-- database constraint for a choice nothing can make is a maintenance cost with no
-- reachable failure. RAISE(ABORT) rolls the statement back and surfaces as a sqlx
-- error → DataError::Db, matching the 0003 immutability triggers.
CREATE TRIGGER backtest_run_inputs_complete BEFORE INSERT ON backtest_run
WHEN NEW.pair IS NULL
  OR NEW.primary_timeframe IS NULL
  OR NEW.primary_data_version IS NULL
  OR NEW.taker_fee_bps IS NULL
  OR NEW.slippage_bps IS NULL
  OR NEW.funding_config IS NULL
  OR ((NEW.htf_timeframe IS NULL) <> (NEW.htf_data_version IS NULL))
BEGIN
  SELECT RAISE(
    ABORT,
    'backtest_run insert is missing input provenance: pair, primary_timeframe, primary_data_version, taker_fee_bps, slippage_bps and funding_config are all required, and htf_timeframe/htf_data_version must be both present or both absent'
  );
END;
