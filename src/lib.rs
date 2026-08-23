//! `PulseTrader` core library.
//!
//! Hexagonal layout via module visibility (MASTER-SPEC §7.1): `mod domain`
//! holds pure types + ports + logic with zero I/O; `mod adapters`, `mod agent`,
//! and `mod tauri` are the outer rings. `pub(crate)` on `domain` enforces the
//! dependency-inward direction *inside* the library; the binary (a separate
//! crate, audit C1) reaches only `run()`.

pub(crate) mod domain;

mod adapters;
mod agent;
mod cli;
mod tauri;

// The domain layer is the library's stable public API surface (the port traits
// + value types — "Internal API surface = port traits in mod domain", tech
// context). `mod domain` stays `pub(crate)` so the dependency-inward direction
// is enforced *within* the crate; the curated re-exports below are what external
// consumers (and the integration boundary) actually see. The binary, a separate
// crate (audit C1), still reaches only `run()` — it never imports these.
pub use domain::{
    CANDLE_SCHEMA_VERSION, Candle, CandleSeries, Clock, DataError, DataVersion, Gap,
    MarketDataSource, Pair, Timeframe, ValidationError,
};

// VS-1.2.3 work-3.01: the build-time `EngineFingerprint` domain newtype (FR-7 /
// NFR-2). `current()` is the sha2-256 hex baked in by `build.rs`; `target()` is the
// compiled triple (the arch tag for the cross-arch story); `compare()` is the FR-7
// cross-fingerprint warning mechanism (built-but-unwired this slice — VS-1.2.4
// surfaces it). REQUIRED under `deny(warnings)` + `pub(crate) mod domain` — an
// un-re-exported public domain type is a `dead_code` build error, not a warning.
// 3.03 attaches it to `BacktestResult` and reaches it through this re-export.
pub use domain::EngineFingerprint;

// VS-1.1.3 work-3.01: the streaming `Indicator` port (FR-5). REQUIRED under
// `deny(warnings)` + `pub(crate) mod domain` — a new public domain type unused
// outside its module is a `dead_code` build error, not a warning (VS-1.1.2
// harvested gotcha). The EMA adapter + future indicators implement this seam.
pub use domain::Indicator;

// VS-1.2.1 work-1.02: the MTF-aligned, no-look-ahead candle feed (FR-5 /
// BACKLOG-4). `align(primary, htf)` is the backtest iteration substrate; each
// `AlignedBar` pairs a primary candle with the most-recent already-closed HTF
// bar (or `None`). REQUIRED under `deny(warnings)` + `pub(crate) mod domain` —
// an un-re-exported public domain type is a `dead_code` build error, not a
// warning. (C6: nothing consumes `AlignedBar::htf` this slice; it is the
// substrate validated by its own unit tests.)
pub use domain::{AlignedBar, align};

// VS-1.1.4 work-1.02: the strategy-tree entities + the `StrategyRepository` port
// (FR-4 / FR-11). `Strategy`/`StrategyVersion` are the persisted records (the
// immutable version's `dsl_original` is verbatim — FR-4); `StrategyId`/`VersionId`
// are the `#[serde(transparent)]` String id newtypes the adapter (1.03) fills
// with UUIDs (1.02 is uuid-free); `CreatedBy` is the `created_by` provenance enum;
// `NewVersion` is the create-request; `diff_versions`/`VersionDiff` are the pure
// FR-11 compare. `StrategyRepository` is the create+read-only persistence seam
// 1.03 implements and 1.05's CLI consumes. REQUIRED under `deny(warnings)` +
// `pub(crate) mod domain` — an un-re-exported public domain item is a `dead_code`
// build error, not a warning.
// The entity value types come from `domain::strategy::` directly (the module is
// `pub(crate)`, matching the `adapters::binance::`/`cli::fetch_data::` precedent);
// `StrategyRepository` lives in `domain::port`, re-exported via the `domain`
// curated surface like `MarketDataSource`.
pub use domain::StrategyRepository;
pub use domain::strategy::{
    CreatedBy, NewVersion, Strategy, StrategyId, StrategyVersion, VersionDiff, VersionId,
    diff_versions,
};

// VS-1.1.2 work-2.01: the DSL grammar leaf + predicate layer. These are the
// strategy-as-data contract types (serde-tagged enums) the LLM builder tools
// (FR-3) target and later DSL items (2.02–2.05) compose. Re-exported on the
// same curated-surface pattern as the domain types above.
pub use domain::{Comparator, Condition, IndicatorSpec, PriceField, SweepableValue, ValueSource};

// VS-1.1.2 work-2.02: the whole-strategy document layer. `StrategyDsl` is the
// top-level document the LLM composes (FR-3) and the backtester executes;
// `ExitRule`/`RiskParams`/`Direction` are its exit/risk vocabulary; the
// hand-rolled `SchemaVersion` (+ its parse error) carries FR-4's semver field.
// Re-exported on the same curated-surface pattern — REQUIRED under
// `deny(warnings)` + `pub(crate) mod domain` (an un-re-exported public domain
// type is a `dead_code` build error, not a warning).
pub use domain::{
    Direction, ExitRule, RiskParams, SchemaVersion, SchemaVersionParseError, StrategyDsl,
};

// VS-1.1.2 work-2.03: the semantic-validation engine (FR-3 correctable rejection).
// `validate` is the entry point; `ValidatedDsl` is the newtype 2.04's compiler
// accepts (constructible ONLY via `validate`); `FieldError`/`ValidationCode`/
// `ValidationErrors` are the field-pathed, serde-serializable error collection
// that crosses the Tauri boundary later. REQUIRED under `deny(warnings)` +
// `pub(crate) mod domain` (an un-re-exported public domain type is a `dead_code`
// build error, not a warning).
pub use domain::{FieldError, ValidatedDsl, ValidationCode, ValidationErrors, validate};
// VS-1.1.2 work-2.05: the version-safe migration read-path (FR-4). `Migrator`
// detects a document's `schema_version`, migrates the JSON forward to `CURRENT`,
// preserves the verbatim `dsl_original`, and returns an UNvalidated
// current-version `StrategyDsl` (`Loaded`). `Migration`/`MigrationKind`/
// `MigrationError`/`LoadError` are its registry + error vocabulary. Re-exported
// on the same curated-surface pattern — REQUIRED under `deny(warnings)`.
pub use domain::{LoadError, Loaded, Migration, MigrationError, MigrationKind, Migrator};

// VS-1.1.2 work-2.04: the compiler → executable evaluator tree (FR-3 / BACKLOG-3).
// `compile(&ValidatedDsl) -> Result<CompiledStrategy, CompileError>` is the slice
// payoff (demo-2): it folds entry ∧ filters into one effective-entry predicate,
// resolves `Fixed` leaves, and yields the stateless evaluator tree the VS-1.2.x
// backtester walks per candle. `CompiledStrategy`/`CompiledCondition`/
// `CompiledValue`/`CompiledExit`/`CompiledRisk` are the tree types; `EvalContext`
// is the seam VS-1.1.3 indicators + the backtester implement; `stop_price`/
// `take_profit_price` are the pure direction-relative exit-geometry helpers.
// Re-exported on the same curated-surface pattern — REQUIRED under
// `deny(warnings)` + `pub(crate) mod domain`.
pub use domain::{
    CompileError, CompiledCondition, CompiledExit, CompiledRisk, CompiledStrategy, CompiledValue,
    EvalContext, compile, stop_price, take_profit_price,
};

// Binance bulk-ingest API surface (WI-1.1.1.02). The adapter module stays
// private (`mod adapters`); these curated re-exports are the entrypoints WI-05
// wires behind the CLI and the integration boundary consumes. Same pattern as
// the domain re-exports above: the implementation modules stay crate-internal,
// the public surface is explicit.
pub use adapters::binance::{
    BinanceDataSource, BulkMonthSource, FundingEvent, MonthData, MonthOutcome, MonthSource,
    decode_month, ingest_bulk, ingest_window, verify_archive_checksum,
};

// WI-1.1.1.03: the REST incremental top-up surface. `top_up_with` is the
// offline-testable seam (`tests/binance_incremental.rs` drives it over a fixture
// `PageSource` + `FakeClock`); `top_up_incremental` is the production wrapper
// WI-05 calls with the injected `Clock`; `TopUpBoundary` carries the snapshot's
// last-open + last-applied-funding timestamps; `PageSource` is the transport seam.
pub use adapters::binance::{PageSource, TopUpBoundary, top_up_incremental, top_up_with};

// WI-1.1.1.03: the Clock adapters. `SystemClock` is the production clock WI-05
// injects into `BinanceDataSource`; `FakeClock` is the deterministic test double
// the integration suite (`tests/binance_incremental.rs`) drives the closed-candle
// cutoff with (audit C5 — cutoff tested exclusively via FakeClock).
pub use adapters::clock::{FakeClock, SystemClock};

// WI-1.1.1.04: the persistence surface (immutable, content-versioned Parquet).
// Re-exported so the integration boundary (and `tests/parquet_roundtrip.rs`)
// can drive a full `CandleSeries` round-trip without reaching `pub(crate)`
// internals.
pub use adapters::store::{CandleStore, SnapshotProvenance};

// WI-1.1.1.05: the `fetch-data` orchestration surface. The end-to-end OFFLINE
// integration test (`tests/integration_fetch_data.rs`, the auto-demo proxy per
// audit C2) drives `run_fetch_data` over fixture seams + a `FakeClock`, never
// the live network. The CLI depends only on the `MarketDataSource` port + the
// store (NFR-9 / AC-6); these re-exports expose the seam without leaking the
// concrete adapter.
pub use cli::fetch_data::{Action, TfOutcome, TfSummary, ensure_one_tf, years_window_start_ms};
pub use cli::{FetchArgs, run_fetch_data};

// VS-1.3.1 work-1.05: the composition-root demo surface. The OFFLINE e2e test
// (`tests/llm_roundtrip_cli.rs`, the auto-demo per audit C2) drives the injectable
// core `run_llm_check_with` over a FAKE provider + a tempfile-`Db` repo (never a
// live `GlmProvider`, never network/Keychain — MASTER-SPEC §9.4); `LlmCheckOutcome`
// is the persisted-`LlmCall` + response it returns. `LlmArgs`/`run_llm_check` are
// the live-arm surface, re-exported like the other `cli::` entrypoints.
pub use cli::llm::{LlmArgs, LlmCheckOutcome, run_llm_check, run_llm_check_with};

// VS-1.3.2 work-2.05: the `pulse compose` composition-root + demo surface (FR-3 /
// FR-4 / NFR-6, README C8). The OFFLINE e2e (`tests/compose_cli.rs`, demo criterion
// 1) drives the injectable core `run_compose_with` over a FAKE provider + the REAL
// composer + REAL builder tools + a tempfile SQLite repo (never a live LLM —
// MASTER-SPEC §9.4); `ComposeWiring` bundles the LLM-side deps, `ComposeCliOutcome`
// is the persisted `StrategyVersion` + provenance it returns. `ComposeArgs` /
// `run_compose` are the live-arm surface, re-exported like the other `cli::`
// entrypoints. REQUIRED under `deny(warnings)` — a `pub` item unused outside its
// private `cli::compose` module is a `dead_code` build error, not a warning.
pub use cli::compose::{
    ComposeArgs, ComposeCliOutcome, ComposeWiring, run_compose, run_compose_with,
};

// VS-1.1.3 work-3.01: the indicator-adapter surface. `Ema` is the walking-skeleton
// `Indicator` adapter; `decimal_to_f64`/`f64_to_decimal_rounded`/`INDICATOR_SCALE`
// are the `Decimal↔f64` conversion seam (the ONLY place floats are allowed).
// Re-exported on the same curated-surface pattern — REQUIRED under
// `deny(warnings)` + `pub(crate) mod adapters` (an un-re-exported public adapter
// item unused outside its module is a `dead_code` build error). Consumed by
// 3.02 (RSI/ADX/MACD), 3.03 (engine/EvalContext), and 3.04/3.05 downstream.
pub use adapters::indicators::convert::{INDICATOR_SCALE, decimal_to_f64, f64_to_decimal_rounded};
pub use adapters::indicators::ema::Ema;

// VS-1.1.3 work-3.02: the RSI + MACD adapters — thin ta-rs wraps behind the same
// `Indicator` port. `Rsi` is Cutler's RSI (EMA-smoothed; 3.04 pins pandas-ta to
// `mamode="ema"`); `Macd` resolves a bare `Macd` spec to the MACD line
// (`EMA(fast) − EMA(slow)`), the v1 #18 default. REQUIRED under `deny(warnings)`
// + `pub(crate) mod adapters` — an un-re-exported public adapter struct unused
// outside its module is a `dead_code` build error (the 3.03 factory consumes
// them next round).
pub use adapters::indicators::macd::Macd;
pub use adapters::indicators::rsi::Rsi;
// VS-1.1.3 work-3.02b: the `Adx` adapter (Wilder `+DI`/`−DI`/ATR), built
// in-adapter because ta-rs v0.5.0 ships no ADX. REQUIRED under `deny(warnings)`
// + `pub(crate) mod adapters` (a new public adapter struct unused outside its
// module is a `dead_code` build error, not a warning); the 3.03 factory that
// consumes it is next round.
pub use adapters::indicators::adx::Adx;
// VS-1.1.3 work-3.03: the multi-indicator engine that implements the frozen
// `EvalContext` seam over real candles and streaming adapter values. Its
// readiness gate is load-bearing for warmup safety under the current boolean DSL
// evaluator.
pub use adapters::indicators::engine::{EngineError, IndicatorEngine};

// VS-1.2.1 work-1.03: deterministic, sequential backtest engine. The adapter
// owns the concrete indicator engine while composing the pure domain backtest
// types and money-math primitives.
pub use adapters::backtest::{BacktestConfig, run_backtest};

// VS-1.1.4 work-1.01: the SQLite persistence foundation. `Db` is the WAL pool
// wrapper (`with_path`/`open_default`/`pool`); `MIGRATOR` is the embedded
// `0001_init` migration set. REQUIRED under `deny(warnings)` + `pub(crate) mod
// adapters` — a new public adapter item unused outside its module is a `dead_code`
// build error, not a warning (VS-1.1.2/1.1.3 harvested gotcha). Their first
// in-crate consumers (1.03's repo + 1.04's backup wrapper) land next round and
// reach them through these re-exports; the new `DataError::{Db,Migration}`
// variants ride the existing `pub use domain::{... DataError ...}` re-export.
pub use adapters::db::{Db, MIGRATOR};

// VS-1.1.4 work-1.03: the SQLite `StrategyRepository` adapter. `SqliteStrategyRepo`
// implements the FR-11 strategy surface + the FR-4 immutable version write/read
// path over `query!`/`query_as!` (the committed `.sqlx/` cache). REQUIRED under
// `deny(warnings)` + `pub(crate) mod adapters` — a new public adapter type unused
// outside its module is a `dead_code` build error, not a warning (§4a-2: the
// `db/mod.rs` re-export alone is necessary but NOT sufficient). 1.05's CLI consumes
// it through the `StrategyRepository` port.
pub use adapters::db::SqliteStrategyRepo;
// VS-1.2.4 work-4.04: the SQLite `BacktestRunRepository` adapter. `SqliteBacktestRunRepo`
// implements the FR-6 persisted-run surface over `query!`/`query_as!` (the
// committed `.sqlx/` cache). REQUIRED under `deny(warnings)` + `pub(crate) mod
// adapters` — a new public adapter type unused outside its module is a `dead_code`
// build error, not a warning (the `db/mod.rs` re-export alone is necessary but NOT
// sufficient). 4.05's CLI consumes it through the `BacktestRunRepository` port.
// Append-only (keep-both with 1.03's re-export at merge).
pub use adapters::db::SqliteBacktestRunRepo;
// VS-1.3.1 work-1.02: the SQLite `LlmCallRepository` adapter. `SqliteLlmCallRepo`
// implements the FR-24 append-only ledger surface over `query!` (the committed
// `.sqlx/` cache), `created_at` from an injected `Clock`, Decimal-as-TEXT cost, and
// a `schema_version` read-reject. REQUIRED under `deny(warnings)` + `pub(crate) mod
// adapters` — a new public adapter type unused outside its module is a `dead_code`
// build error (the `db/mod.rs` re-export alone is necessary but NOT sufficient).
// 1.04's decorator constructs it; 1.05's demo reads a row back. Append-only
// (keep-both with 1.03's re-export at merge).
pub use adapters::db::SqliteLlmCallRepo;
// VS-1.1.4 work-1.04: the backup-before-migrate protocol surface. `open_migrated`
// is 1.05's single startup entry (migrate-then-open); `run_migrations_with_backup`
// + `undo_to` + `MigrationOutcome` are the protocol vocabulary tests + the
// integration boundary drive. REQUIRED under `deny(warnings)` + `pub(crate) mod
// adapters` — a new public adapter item unused outside its module is a `dead_code`
// build error, not a warning (VS-1.1.2 harvested gotcha); re-export ALL of them,
// not just the first. Append-only (keep-both with 1.03's re-exports at merge).
pub use adapters::db::{MigrationOutcome, open_migrated, run_migrations_with_backup, undo_to};

// VS-1.2.1 work-1.01: the pure backtester domain foundation (FR-5 / FR-6,
// BACKLOG-4). The trade-record entities (`Trade`/`Fill`/`ExitReason`/
// `TradeSource`), the run aggregate (`BacktestResult`), the error taxonomy
// (`BacktestError`, incl. `NoStopLoss` (G5/#20) + `UnsupportedExit` (C4)), and
// the pure `Decimal`-only money-math (`taker_fee`/`apply_slippage`+`Side`/
// `funding_payment`/`realized_pnl`/`realized_r`/`position_size`) +
// intra-bar collision (`resolve_intra_bar_exit`/`IntraBarExit`). The event loop
// (1.03) composes these; 1.04's CLI renders the result. REQUIRED under
// `deny(warnings)` + `pub(crate) mod domain` — an un-re-exported public domain
// type is a `dead_code` build error, not a warning. Kept additive (work-1.02's
// MTF feed extends the same `backtest` tree at the R1→R2 merge).
pub use domain::{
    BacktestError, BacktestResult, ExitReason, Fill, IntraBarExit, Side, Trade, TradeSource,
    apply_slippage, funding_payment, realized_pnl, realized_r, resolve_intra_bar_exit, taker_fee,
};

// VS-1.2.2 work-2.01: the shared, exchange-aware position sizer (FR-5 / NFR-3,
// BACKLOG-5) — the `pulse-broker` money-math home. `compute_position_size` is the
// **single** sizer sim + (future v3) live execution share (NFR-3 by
// construction). `SymbolFilters` (+ `unconstrained()`) is the exchange-filter
// value type; `SizingOutcome`/`SkipReason` are the skip-and-count substrate (2.04
// wires, 2.05 renders). `ExchangeAdapter` is the exchange-metadata port;
// `ExchangeError` its dedicated error (audit C5). REQUIRED under `deny(warnings)`
// + `pub(crate) mod domain` — an un-re-exported public domain type is a
// `dead_code` build error, not a warning.
//
// NFR-3 single-sizing-path hardening (VS-1.2.2 slice-close, close-audit finding):
// the pre-quantization core `risk_capped_qty` is **intentionally NOT re-exported**
// — it bypasses lot_step / min_qty / min_notional / exchange max-leverage, so
// exposing it publicly would make the single-path invariant a convention rather
// than construction. It stays crate-internal (the `pub(crate) mod domain` gate),
// reachable only by `compute_position_size` (its sole production caller) + the
// in-module proptests; the ONLY public sizing entry is `compute_position_size`.
pub use domain::{
    ExchangeAdapter, ExchangeError, SizingOutcome, SkipReason, SkippedEntryCounts, SymbolFilters,
    compute_position_size,
};

// VS-1.2.2 work-2.01: the `BinanceAdapter` exchange-metadata implementor
// (`adapters/broker`), returning pinned BTCUSDT USD-M futures filters. REQUIRED
// under `deny(warnings)` + `pub(crate) mod adapters` — a new public adapter type
// unused outside its module is a `dead_code` build error. 2.04 wires it into
// `run_backtest`'s sizing call through the `ExchangeAdapter` port.
pub use adapters::broker::BinanceAdapter;

// VS-1.2.2 work-2.03: the regime classifier surface (FR-5 / FR-6, BACKLOG-5).
// `Regime`/`RegimeBreakdown`/`RegimeCell`/`classify`/`ADX_TREND_THRESHOLD` are
// the pure domain half; `RegimeDetector` is the stateful adapter composing the
// VS-1.1.3 `Ema`/`Adx` adapters. REQUIRED under `deny(warnings)` + `pub(crate)
// mod domain`/`mod adapters` — an un-re-exported public item is a `dead_code`
// build error, not a warning. 2.04 steps the detector over the run + aggregates
// the breakdown onto `BacktestResult`; 2.05 renders it.
pub use adapters::backtest::RegimeDetector;
pub use domain::{ADX_TREND_THRESHOLD, Regime, RegimeBreakdown, RegimeCell, classify};

// VS-1.2.4 work-4.01: the derived read-only `SummaryStats` + equity curve surface
// (FR-6 / NFR-2, BACKLOG-4). `SummaryStats`/`EquityCurve`/`EquityPoint` are the
// pure-`Decimal`/`usize` headline read of a finished backtest, computed in
// `LoopState::into_result` and attached to `BacktestResult`; oracle-excluded
// (README C3) so the frozen baseline stays frozen by construction. REQUIRED under
// `deny(warnings)` + `pub(crate) mod domain` — an un-re-exported public domain
// type is a `dead_code` build error, not a warning. 4.02 adds Sharpe/Sortino onto
// the same struct; 4.03/4.04 persist these columns; 4.05 renders + rebuilds the
// curve on read via the SAME `EquityCurve::from_trades` constructor.
pub use domain::{EquityCurve, EquityPoint, SummaryStats};

// VS-1.2.4 work-4.04: the persisted backtest-run system-of-record (FR-6 / FR-7 /
// NFR-2). `BacktestRunRepository` is the create+read-only persistence port (the
// #39 ownership-on-write / re-validate-on-read / scoped corrupt-isolation seam);
// `BacktestRunId`/`PersistedRun`/`RunSummary` are the typed read-back projections
// (explicit columns per README C4, NOT a `BacktestResult` blob — #68 / D1).
// REQUIRED under `deny(warnings)` + `pub(crate) mod domain` — an un-re-exported
// public domain type is a `dead_code` build error, not a warning. 4.05's CLI
// consumes `save_run`/`get_run`/`latest_run_for_version` through the port.
pub use domain::{BacktestRunId, BacktestRunRepository, PersistedRun, RunSummary};

// VS-1.3.1 work-1.01: the LLM domain ring (FR-23 / FR-24, README C1–C5). The
// PulseTrader-OWNED `LlmProvider` port + the message/response/usage/config value
// types + the dedicated `LlmError` + the pure cost model
// (`ModelPrice`/`PriceTable`) + the `LlmCall` ledger entity + its `LlmCallId`
// newtype — all free of any `PulseHive` dep (ADR-0012 insulation; AC-8). REQUIRED under
// `deny(warnings)` + `pub(crate) mod domain` — an un-re-exported public domain
// type is a `dead_code` BUILD error, not a warning (the `domain/mod.rs` re-export
// alone is necessary but NOT sufficient — the harvested dead-code gotcha). 1.02
// (persistence) + 1.03 (GLM adapter) + 1.04 (redacting-logging decorator) consume
// these through the `LlmProvider` port + the value/cost types.
pub use domain::{
    LlmBackend, LlmCall, LlmCallId, LlmConfig, LlmError, LlmProvider, LlmResponse, Message,
    ModelPrice, PriceTable, TokenUsage, ToolCall,
};
// VS-1.3.2 work-2.01: the additive tool-calling transport type (FR-23 / FR-3).
// Mirror of the `domain/mod.rs` re-export (REQUIRED — the dead-code gotcha: the
// module-level re-export alone is necessary but NOT sufficient under
// `deny(warnings)`). Appended as its own line so the parallel 2.03 additions merge
// cleanly.
pub use domain::ToolDefinition;
// VS-1.3.1 work-1.02: the append-only `LlmCall` persistence port (FR-24, README C6)
// — create + read only, `DataError::Db`-erroring, immutability structural in the API
// (enforced by the migration-`0004` triggers). REQUIRED under `deny(warnings)` +
// `pub(crate) mod domain` — an un-re-exported public domain type is a `dead_code`
// BUILD error, not a warning (the `domain/mod.rs` re-export alone is necessary but
// NOT sufficient). 1.04's decorator writes through it; 1.05's demo reads a row back.
pub use domain::LlmCallRepository;

// VS-1.3.1 work-1.03 / VS-1.3.2 work-2.01: the OpenAI-compatible transport adapter +
// the macOS Keychain READ accessor (README C8/C2, FR-23 / FR-3 / FR-1 / NFR-5).
// `OpenAiCompatProvider` (generalized from 1.03's `GlmProvider`, pointed at Ollama
// Cloud) is the anti-corruption layer implementing the `LlmProvider` port over the
// `PulseHive` OpenAI-compatible transport (the ONLY `PulseHive`-importing module —
// AC-9); `glm_api_key` sources the API key from the login Keychain (READ path only).
// REQUIRED under `deny(warnings)` + `pub(crate) mod adapters` — a new public adapter
// item unused outside its module is a `dead_code` BUILD error, not a warning. 1.04's
// decorator composes over the provider; 1.05's composition root injects the key.
pub use adapters::llm::openai_compat::OpenAiCompatProvider;
pub use adapters::secrets::glm_api_key;

// VS-1.3.1 work-1.04: the redacting + cost-logging `LlmProvider` decorator
// (README C7, FR-24 / NFR-6). `RedactingLoggingProvider` wraps any inner provider
// (1.05 wraps `GlmProvider`), redacts the PERSISTED copy of the prompt/completion
// (grill OQ-A — the model still gets the real bytes), computes the `Decimal` cost
// from `usage` times the `PriceTable`, and writes an `LlmCall` through the
// `LlmCallRepository`; `Redactor` is the scoped, config-driven secret scrubber.
// REQUIRED under `deny(warnings)` + `pub(crate) mod adapters` — a new public
// adapter type unused outside its module is a `dead_code` BUILD error, not a
// warning (the `llm/mod.rs` re-export alone is necessary but NOT sufficient).
pub use adapters::llm::redacting_logging::{RedactingLoggingProvider, Redactor};

// VS-1.3.2 work-2.04: the composer agent loop surface (FR-3 / FR-4, README C7). The
// `Composer<P>` orchestrator + `compose()` + the `ComposeOutcome` value it RETURNS +
// the streamed `ComposerEvent` + the dedicated `ComposerError` + the `LlmCallCapture`
// provenance handle. Mirror of the `agent/mod.rs` re-export (REQUIRED — the dead-code
// gotcha: the module-level `pub use` alone is necessary but NOT sufficient under
// `deny(warnings)`; a `pub` item unused outside its private module is a `dead_code`
// BUILD error). 2.05's `pulse compose` verb + composition root consume these.
pub use agent::{ComposeOutcome, Composer, ComposerError, ComposerEvent, LlmCallCapture};

/// Library entry point invoked by the thin binary shim (`src/main.rs`).
///
/// A thin **sync** entry (audit C1/C3): it delegates to [`cli::run`], which
/// parses args via `clap`, builds a multi-thread `tokio` runtime, and
/// `block_on`s the async `fetch-data` orchestration. There is no `#[tokio::main]`
/// and `main` stays the trivial `Result` → `ExitCode` shim.
///
/// # Errors
///
/// Returns an [`anyhow::Error`] on arg-parse failure, runtime-build failure, or
/// when any requested timeframe failed to fetch (non-zero exit, audit C4).
pub fn run() -> anyhow::Result<()> {
    cli::run()
}
