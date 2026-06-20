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
// VS-1.2.2 work-2.01: the dedicated exchange-port error taxonomy (audit C5).
mod exchange;
mod indicator;
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
    TradeSource, align, apply_slippage, funding_payment, position_size, realized_pnl, realized_r,
    resolve_intra_bar_exit, taker_fee,
};
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
pub use pair::Pair;
// VS-1.1.4 work-1.02: the `StrategyRepository` port (FR-4 / FR-11) alongside
// `MarketDataSource`. The strategy entity value types are surfaced to `lib.rs`
// via the `pub(crate) mod strategy` path directly (matching the
// `adapters::binance::` precedent), so they are NOT re-listed here.
pub use port::{ExchangeAdapter, MarketDataSource, StrategyRepository};
// VS-1.2.2 work-2.01: the shared sizer surface (FR-5 / NFR-3, BACKLOG-5). The
// pure money-math (`risk_capped_qty` core + `compute_position_size`), the
// `SymbolFilters` value type + its `unconstrained()` ctor, and the
// `SizingOutcome`/`SkipReason` skip-and-count substrate (2.04 wires, 2.05
// renders). The dedicated `ExchangeError` (audit C5) rides the `ExchangeAdapter`
// port. Re-exported so `lib.rs` can curate the crate surface — an un-re-exported
// public domain type is a `dead_code` BUILD error under `deny(warnings)`.
pub use exchange::ExchangeError;
pub use series::{CandleSeries, Gap};
pub use sizing::{
    SizingOutcome, SkipReason, SymbolFilters, compute_position_size, risk_capped_qty,
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
