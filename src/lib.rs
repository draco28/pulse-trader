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
