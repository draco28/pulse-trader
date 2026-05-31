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
    CANDLE_SCHEMA_VERSION, Candle, CandleSeries, DataError, DataVersion, Gap, MarketDataSource,
    Pair, Timeframe, ValidationError,
};

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
