//! Adapters ring (outer): `SQLite` repos, `BinanceDataSource`, broker, clock.
//!
//! Stub for WI-01 to pin the hexagonal layout. WI-02/03/04 land the concrete
//! `MarketDataSource` implementations here.

// WI-1.1.1.02: Binance HTTP client + bulk historical ingest.
pub(crate) mod binance;

// WI-1.1.1.03: Clock adapters (SystemClock + FakeClock) for the closed-candle cutoff.
pub(crate) mod clock;

// r1.s4.w4: IdSource adapters (UuidIdSource + SeqIdSource) — the injected row-id
// seam the coach accept mints its child/run identity through.
pub(crate) mod ids;

// WI-1.1.1.04: immutable, content-versioned Parquet persistence for `CandleSeries`.
pub(crate) mod store;

// VS-1.1.3 work-3.01: indicator adapters (ta-rs wrapped) + the `Decimal↔f64`
// conversion seam. The ONLY module tree where `f64` is permitted.
pub(crate) mod indicators;

// VS-1.2.1 work-1.03: deterministic backtest event loop. Lives in adapters
// because it owns the concrete IndicatorEngine.
pub(crate) mod backtest;

// VS-1.2.2 work-2.01: the `pulse-broker` exchange-metadata adapter home.
// `BinanceAdapter` implements the `ExchangeAdapter` port over pinned BTCUSDT
// USD-M futures consts. `pub(crate)` (matching the sibling adapter precedent) so
// `lib.rs` curates the public surface.
pub(crate) mod broker;

// VS-1.1.4 work-1.01: the SQLite persistence tier — the `Db` pool wrapper (WAL +
// foreign_keys + busy_timeout connect options), the embedded `MIGRATOR`, and the
// platform-default db-path resolver. The ONLY module tree where `sqlx` is allowed
// (the domain stays I/O-free).
pub(crate) mod db;

// VS-1.3.1 work-1.03: the GLM transport adapter home (README C8). `llm::glm` holds
// `GlmProvider` — the anti-corruption layer over the `PulseHive` OpenAI-compatible
// transport, and the ONLY module tree importing the `PulseHive` SDK crate (AC-6).
// `pub(crate)` matching the sibling adapter precedent so `lib.rs` curates the
// public `GlmProvider` re-export (dead-code gotcha).
pub(crate) mod llm;

// VS-1.3.1 work-1.03: the macOS Keychain READ accessor (`glm_api_key`) — FR-1 /
// NFR-5. The ONLY module tree touching `keyring` (the domain stays keyring-free,
// AC-7). `pub(crate)` matching the sibling adapter precedent; `lib.rs` curates the
// public re-export.
pub(crate) mod secrets;

// r1.s4.w4: in-memory adapters at product-owned seams — currently the
// `CoachAcceptanceRepository` test adapter that lets the accept rail's decision
// logic be driven without a database. Shipped (not `#[cfg(test)]`) because the
// integration binaries and later work items consume it across the crate boundary.
pub(crate) mod memory;
