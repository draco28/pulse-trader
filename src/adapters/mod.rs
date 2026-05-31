//! Adapters ring (outer): `SQLite` repos, `BinanceDataSource`, broker, clock.
//!
//! Stub for WI-01 to pin the hexagonal layout. WI-02/03/04 land the concrete
//! `MarketDataSource` implementations here.

pub(crate) mod binance;
