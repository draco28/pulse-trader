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

// Binance bulk-ingest API surface (WI-1.1.1.02). The adapter module stays
// private (`mod adapters`); these curated re-exports are the entrypoints WI-05
// wires behind the CLI and the integration boundary consumes. Same pattern as
// the domain re-exports above: the implementation modules stay crate-internal,
// the public surface is explicit.
pub use adapters::binance::{
    BulkMonthSource, FundingEvent, MonthData, MonthOutcome, MonthSource, decode_month, ingest_bulk,
    ingest_window, verify_archive_checksum,
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

/// Library entry point invoked by the thin binary shim (`src/main.rs`).
///
/// Placeholder for WI-01; WI-05 replaces the body with CLI dispatch. Returns
/// `Ok(())` so the bootstrap binary exits cleanly.
///
/// # Errors
///
/// Currently never errors. The `Result` signature is the stable contract the
/// binary maps to a process exit code; later work items return real failures.
pub fn run() -> anyhow::Result<()> {
    Ok(())
}
