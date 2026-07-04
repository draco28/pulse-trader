//! Domain layer (innermost ring): pure value types, the `MarketDataSource`
//! port, and validation logic. Zero I/O — no `reqwest`/`sqlx`/`polars`/`tokio`
//! in non-test paths (the port's `Send` test uses tokio as a dev-dependency).
//!
//! Dependency policy is "zero I/O", not "zero deps": `serde`, `rust_decimal`,
//! `thiserror`, and `chrono` are permitted.

// VS-1.2.1: the pure-domain backtester foundation — money-math + trade entities
// (work-1.01) and the MTF-aligned, no-look-ahead candle feed (work-1.02).
// `pub(crate)` (matching the `dsl`/`strategy` nested-module precedent) so
// `lib.rs` can curate the public surface via the `domain::backtest::` path.
pub(crate) mod backtest;
mod candle;
mod clock;
mod dsl;
mod error;
// VS-1.2.3 work-3.01: the build-time `EngineFingerprint` domain newtype (FR-7 /
// NFR-2). Pure accessor over `build.rs`-baked env (`PULSE_ENGINE_FINGERPRINT` /
// `PULSE_TARGET_TRIPLE`) plus the FR-7 `compare()` warning mechanism (built but
// unwired this slice — VS-1.2.4 surfaces it).
mod fingerprint;
// VS-1.2.2 work-2.01: the dedicated exchange-port error taxonomy (audit C5).
mod exchange;
mod indicator;
// VS-1.3.1 work-1.01: the LLM domain ring (FR-23 / FR-24). `llm` holds the value
// types + dedicated error + pure cost model; `llm_call` the append-only ledger
// entity. Both pure + zero-I/O + free of any `PulseHive` dep (ADR-0012); the
// `LlmProvider` port lives in `port` beside the other ports.
mod llm;
mod llm_call;
mod pair;
mod port;
mod series;
// VS-1.2.2 work-2.01: the shared, exchange-aware position sizer (FR-5 / NFR-3,
// BACKLOG-5) — the `pulse-broker` money-math home as a module. `pub(crate)`
// (matching the `dsl`/`strategy`/`backtest` precedent) so `lib.rs` can curate the
// public surface; the types leave the crate only via the explicit re-exports.
pub(crate) mod sizing;
// `pub(crate)` (matching the `adapters`/`cli` nested-module precedent) so the
// curated `lib.rs` surface can re-export via the `domain::strategy::` path — the
// types still leave the crate only through the explicit `pub use` re-exports.
pub(crate) mod strategy;
mod timeframe;
mod version;

// VS-1.2.1 backtester domain surface: money-math + entities (work-1.01) and the
// no-look-ahead candle feed (work-1.02). Re-exported here so `lib.rs` can curate
// the crate surface; an un-re-exported public domain type is a `dead_code` BUILD
// error under `deny(warnings)`. `AlignedBar` borrows from the input series, so
// its lifetime is tied to the caller's `CandleSeries`.
pub use backtest::{
    AlignedBar, BacktestError, BacktestResult, ExitReason, Fill, IntraBarExit, Side, Trade,
    TradeSource, align, apply_slippage, funding_payment, realized_pnl, realized_r,
    resolve_intra_bar_exit, taker_fee,
};
// VS-1.2.2 work-2.03: the pure regime surface (EMA50/200 + ADX14 classifier).
// Re-exported here so `lib.rs` can curate the crate surface; an un-re-exported
// public domain type is a `dead_code` BUILD error under `deny(warnings)`.
pub use backtest::{ADX_TREND_THRESHOLD, Regime, RegimeBreakdown, RegimeCell, classify};
// VS-1.2.4 work-4.01: the derived read-only summary stats + equity curve surface
// (FR-6 / NFR-2). Re-exported so `lib.rs` can curate the crate surface; an
// un-re-exported public domain type is a `dead_code` BUILD error under
// `deny(warnings)`.
pub use backtest::{EquityCurve, EquityPoint, SummaryStats};
// VS-1.2.4 work-4.04: the persisted backtest-run projection types (FR-6 / FR-7 /
// NFR-2). `BacktestRunId`/`PersistedRun`/`RunSummary` are the typed read-back
// projections the `BacktestRunRepository` port returns; re-exported so `lib.rs`
// can curate the crate surface — an un-re-exported public domain type is a
// `dead_code` BUILD error under `deny(warnings)`.
pub use backtest::{BacktestRunId, PersistedRun, RunSummary};
pub use candle::Candle;
pub use clock::Clock;
// VS-1.1.3 work-3.01: the streaming `Indicator` port (FR-5) — the seam every
// concrete indicator adapter implements and the backtester reads through.
pub use dsl::{
    Comparator, Condition, Direction, ExitRule, IndicatorSpec, PriceField, RiskParams,
    SchemaVersion, SchemaVersionParseError, StrategyDsl, SweepableValue, ValueSource,
};
pub use indicator::Indicator;
// VS-1.1.2 work-2.03: the semantic-validation surface (FR-3 correctable rejection).
pub use dsl::{FieldError, ValidatedDsl, ValidationCode, ValidationErrors, validate};
// VS-1.1.2 work-2.05: the version-safe migration read-path (FR-4).
pub use dsl::{LoadError, Loaded, Migration, MigrationError, MigrationKind, Migrator};
// VS-1.1.2 work-2.04: the compiler → executable evaluator tree (FR-3). `compile`
// turns a `ValidatedDsl` into a `CompiledStrategy` the backtester walks; the
// `Compiled*` types + `EvalContext` seam + pure exit-geometry helpers are its
// surface.
pub use dsl::{
    CompileError, CompiledCondition, CompiledExit, CompiledRisk, CompiledStrategy, CompiledValue,
    EvalContext, compile, stop_price, take_profit_price,
};
pub use error::{DataError, ValidationError};
// VS-1.2.3 work-3.01: the build-time engine identity (FR-7 / NFR-2). Re-exported
// here so `lib.rs` can curate the crate surface; an un-re-exported public domain
// type is a `dead_code` BUILD error under `deny(warnings)`.
pub use fingerprint::EngineFingerprint;
pub use pair::Pair;
// VS-1.1.4 work-1.02: the `StrategyRepository` port (FR-4 / FR-11) alongside
// `MarketDataSource`. The strategy entity value types are surfaced to `lib.rs`
// via the `pub(crate) mod strategy` path directly (matching the
// `adapters::binance::` precedent), so they are NOT re-listed here.
pub use port::{
    BacktestRunRepository, ExchangeAdapter, LlmCallRepository, LlmProvider, MarketDataSource,
    StrategyRepository,
};
// VS-1.3.1 work-1.01: the LLM domain ring surface (FR-23 / FR-24, README C2–C5).
// The message/response/usage/config value types + the dedicated `LlmError` + the
// pure cost model (`ModelPrice`/`PriceTable`), and the `LlmCall` ledger entity +
// its `LlmCallId` newtype. Re-exported here so `lib.rs` can curate the crate
// surface — an un-re-exported public domain type is a `dead_code` BUILD error
// under `deny(warnings)`. 1.02–1.04 consume these through the `LlmProvider` port.
pub use llm::{
    LlmBackend, LlmConfig, LlmError, LlmResponse, Message, ModelPrice, PriceTable, TokenUsage,
    ToolCall,
};
pub use llm_call::{LlmCall, LlmCallId};
// VS-1.2.2 work-2.01: the shared sizer surface (FR-5 / NFR-3, BACKLOG-5).
// `compute_position_size` is the single exchange-constrained sizing entry; the
// `SymbolFilters` value type + its `unconstrained()` ctor, and the
// `SizingOutcome`/`SkipReason` skip-and-count substrate (2.04 wires, 2.05
// renders). The dedicated `ExchangeError` (audit C5) rides the `ExchangeAdapter`
// port. Re-exported so `lib.rs` can curate the crate surface — an un-re-exported
// public domain type is a `dead_code` BUILD error under `deny(warnings)`.
// NFR-3 hardening (slice-close): the pre-quantization core `risk_capped_qty` is
// NOT re-exported — it bypasses the exchange constraints, so it stays
// crate-internal (used only by `compute_position_size` + the in-module proptests).
pub use exchange::ExchangeError;
pub use series::{CandleSeries, Gap};
pub use sizing::{
    SizingOutcome, SkipReason, SkippedEntryCounts, SymbolFilters, compute_position_size,
};
pub use timeframe::Timeframe;
pub use version::DataVersion;

/// Version of the `CandleSeries` on-disk schema (audit C7).
///
/// WI-01 defined no schema version; WI-1.1.1.04 introduces it additively so it
/// can be folded into the content-hash `data_version` (see
/// `crate::adapters::store`). Bumping this constant on any future schema change
/// forces every snapshot to a new `data_version`, preventing a stale snapshot
/// from being mistaken for one written under the new schema.
pub const CANDLE_SCHEMA_VERSION: u32 = 1;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{DataError, ValidationError};

    #[test]
    fn data_error_skeleton_variants_exist_and_serialize() {
        // The audit-C5 documented skeleton: Validation{Unsorted,Duplicate}, Gap, Parse, Io.
        // VS-1.1.4 work-1.01 extends it additively with the SQLite tier's `Db`
        // (connection/query/trigger-ABORT) + `Migration` (apply/verify/backup)
        // variants — both `String`-payload so the domain stays free of
        // `sqlx::Error` (which is not `Serialize`), exactly as `Io` avoids
        // `std::io::Error`.
        let cases = vec![
            DataError::Validation(ValidationError::Unsorted {
                earlier: 1,
                later: 0,
            }),
            DataError::Validation(ValidationError::Duplicate(7)),
            DataError::Gap {
                expected: 900_000,
                found: 1_800_000,
            },
            DataError::Parse("bad decimal".to_string()),
            DataError::Io("disk full".to_string()),
            DataError::Db("near \"SELCT\": syntax error".to_string()),
            DataError::Migration("0001_init failed to apply".to_string()),
        ];

        for err in cases {
            // serde round-trip (errors must cross the Tauri boundary later).
            let json = serde_json::to_string(&err).expect("serialize DataError");
            let back: DataError = serde_json::from_str(&json).expect("deserialize DataError");
            assert_eq!(err, back);
            // thiserror Display is non-empty.
            assert!(!err.to_string().is_empty());
        }
    }
}
