-- VS-1.3.1 work-1.02 — 0004: the append-only `LlmCall` ledger (FR-24, README C6).
-- Mirrors the `0003_backtest_runs` conventions EXACTLY (the single source of truth
-- for these shapes):
--   * every money / Decimal column is TEXT, stored as the same `.normalize()`d
--     canonical text the domain uses — NEVER an f64 / binary-fraction column
--     (NFR-2). `cost` is the computed spend (1.04 fills it from usage × the price
--     table); this item persists whatever it is handed.
--   * `cost_currency` carries the price table's NATIVE billing currency (e.g.
--     `CNY` — GLM/Zhipu bills RMB), NOT an assumed-USD `_usd` column and NO silent
--     FX conversion (audit ch3). A v2 analytics view can convert with a dated rate.
--   * `prompt_messages` is the serde-JSON of `Vec<Message>` (verbatim-after-
--     redaction, NFR-6, applied upstream by the 1.04 decorator); `completion` is
--     the redacted response text (TEXT NULL for a tool-calls-only turn).
--   * tokens are INTEGER; `created_at` is an RFC3339 UTC TEXT injected via the
--     `Clock`; `created_by` is the `CreatedBy` provenance tag; `schema_version` is
--     a row-schema tag the adapter ASSERTS on read (read-reject, #68).
--
-- NO FK to `strategy_version` this slice — `llm_call` is standalone; VS-1.3.2's
-- composer wires attribution (`StrategyVersion.creating_llm_call_ids`). An index on
-- `created_at` (+ `id` tie-break) serves the recent-first ledger listing.

CREATE TABLE llm_call (
  id               TEXT PRIMARY KEY NOT NULL,
  backend          TEXT NOT NULL,                 -- backend serde tag, e.g. `glm`
  model            TEXT NOT NULL,                 -- model id, e.g. `glm-5.1`
  prompt_messages  TEXT NOT NULL,                 -- serde-JSON of Vec<Message>
  completion       TEXT,                          -- redacted response text, NULL when none
  input_tokens     INTEGER NOT NULL,              -- prompt tokens billed
  output_tokens    INTEGER NOT NULL,              -- completion tokens billed
  cost             TEXT NOT NULL,                 -- Decimal-as-TEXT (.normalize()'d), NFR-2 — never f64
  cost_currency    TEXT NOT NULL,                 -- native billing currency, e.g. `CNY` (audit ch3)
  created_at       TEXT NOT NULL,                 -- injected Clock (RFC3339 UTC)
  created_by       TEXT NOT NULL,                 -- CreatedBy provenance tag
  schema_version   TEXT NOT NULL                  -- row-schema tag, asserted on read (#68)
);

-- Immutability (FR-24, append-only ledger): SQLite has no combined trigger form, so
-- the table needs SEPARATE BEFORE UPDATE / BEFORE DELETE triggers, each
-- RAISE(ABORT, ...) — mirroring `backtest_run` at `0003_backtest_runs.up.sql:86-89`.
-- RAISE(ABORT) rolls the statement back and surfaces as a sqlx error → DataError::Db
-- on the caller side.
CREATE TRIGGER llm_call_no_update BEFORE UPDATE ON llm_call
  BEGIN SELECT RAISE(ABORT, 'llm_call is immutable'); END;
CREATE TRIGGER llm_call_no_delete BEFORE DELETE ON llm_call
  BEGIN SELECT RAISE(ABORT, 'llm_call is immutable'); END;

-- Recent-first listing (README C6): the composite `(created_at, id)` makes an
-- `ORDER BY created_at[, id]` ledger scan index-served + total.
CREATE INDEX idx_llm_call_created_at ON llm_call(created_at, id);
