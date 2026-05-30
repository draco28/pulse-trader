//! `MarketDataSource` — the domain port for historical/incremental candle data.
//!
//! Hexagonal inbound-of-outer port (NFR-9): adapters (`BinanceDataSource` in
//! WI-02, a `Parquet`-replay source later) implement it; the engine consumes it
//! generically (`<S: MarketDataSource>`), never as `dyn`. The domain stays free
//! of `tokio`/`reqwest`; only the trait shape lives here.
//!
//! **`Send` futures (audit C3):** the methods return `impl Future<..> + Send`
//! rather than bare `async fn`, so the returned futures are guaranteed `Send`
//! and adapter calls can be `spawn`ed on tokio's multi-thread runtime. Stating
//! the bound explicitly also sidesteps the `async_fn_in_trait` lint — no
//! `#[allow(..)]` is needed.

use std::future::Future;

use crate::domain::candle::Candle;
use crate::domain::error::DataError;
use crate::domain::pair::Pair;
use crate::domain::series::CandleSeries;
use crate::domain::timeframe::Timeframe;

/// A source of historical and incremental candle data.
///
/// All methods return `Send` futures so callers may `spawn` them across threads.
pub trait MarketDataSource {
    /// Fetch a closed historical range `[start_ms, end_ms)` for `(pair, tf)`.
    ///
    /// # Errors
    ///
    /// Returns [`DataError`] if the underlying source fails (I/O, parse) or the
    /// returned series is structurally invalid.
    fn fetch_historical(
        &self,
        pair: &Pair,
        tf: Timeframe,
        start_ms: i64,
        end_ms: i64,
    ) -> impl Future<Output = Result<CandleSeries, DataError>> + Send;

    /// Fetch candles newer than `since_ms` for `(pair, tf)`.
    ///
    /// # Errors
    ///
    /// Returns [`DataError`] if the underlying source fails (I/O, parse).
    fn fetch_incremental(
        &self,
        pair: &Pair,
        tf: Timeframe,
        since_ms: i64,
    ) -> impl Future<Output = Result<Vec<Candle>, DataError>> + Send;
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::MarketDataSource;
    use crate::domain::candle::Candle;
    use crate::domain::error::DataError;
    use crate::domain::pair::Pair;
    use crate::domain::series::CandleSeries;
    use crate::domain::timeframe::Timeframe;
    use crate::domain::version::DataVersion;

    /// A trivial generic fake implementing the port with no I/O.
    struct FakeSource;

    impl MarketDataSource for FakeSource {
        async fn fetch_historical(
            &self,
            pair: &Pair,
            tf: Timeframe,
            _start_ms: i64,
            _end_ms: i64,
        ) -> Result<CandleSeries, DataError> {
            Ok(CandleSeries {
                pair: pair.clone(),
                timeframe: tf,
                version: DataVersion::new("fake"),
                candles: Vec::new(),
            })
        }

        async fn fetch_incremental(
            &self,
            _pair: &Pair,
            _tf: Timeframe,
            _since_ms: i64,
        ) -> Result<Vec<Candle>, DataError> {
            Ok(Vec::new())
        }
    }

    // Generic consumption (`<S: MarketDataSource>`) proves the port is used by
    // bound, not as `dyn`, and that the future is `Send` (required by `spawn`).
    async fn fetch_via<S: MarketDataSource>(source: S) -> Result<CandleSeries, DataError> {
        let pair = Pair::new("BTCUSDT");
        source
            .fetch_historical(&pair, Timeframe::H4, 0, 14_400_000)
            .await
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn fetch_future_is_send_spawnable_on_multi_thread_runtime() {
        // If the port's future were not `Send`, this `spawn` would not compile.
        let handle = tokio::spawn(async { fetch_via(FakeSource).await });
        let series = handle
            .await
            .expect("spawned task joins")
            .expect("fetch succeeds");
        assert_eq!(series.timeframe, Timeframe::H4);
        assert!(series.candles.is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn fetch_incremental_future_is_send_spawnable() {
        let handle = tokio::spawn(async {
            let pair = Pair::new("BTCUSDT");
            FakeSource.fetch_incremental(&pair, Timeframe::M15, 0).await
        });
        let candles = handle
            .await
            .expect("spawned task joins")
            .expect("fetch succeeds");
        assert!(candles.is_empty());
    }
}
