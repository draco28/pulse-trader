//! Adapters ring (outer): `SQLite` repos, `BinanceDataSource`, broker, clock.
//!
//! Stub for WI-01 to pin the hexagonal layout. WI-02/03/04 land the concrete
//! `MarketDataSource` implementations here.

// WI-1.1.1.02: Binance HTTP client + bulk historical ingest.
pub(crate) mod binance;

// WI-1.1.1.03: Clock adapters (SystemClock + FakeClock) for the closed-candle cutoff.
pub(crate) mod clock;

// WI-1.1.1.04: immutable, content-versioned Parquet persistence for `CandleSeries`.
pub(crate) mod store;

// VS-1.1.3 work-3.01: indicator adapters (ta-rs wrapped) + the `Decimal↔f64`
// conversion seam. The ONLY module tree where `f64` is permitted.
pub(crate) mod indicators;
