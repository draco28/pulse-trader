//! Domain ports — the hexagonal seams the outer rings implement.
//!
//! - [`MarketDataSource`] — historical/incremental candle data (WI-02 onward).
//! - [`StrategyRepository`] — the strategy-tree persistence port (VS-1.1.4,
//!   FR-4 / FR-11); the `sqlx` adapter (1.03) implements it, the CLI (1.05) and
//!   agent layer consume it generically (`<R: StrategyRepository>`).
//!
//! Hexagonal inbound-of-outer ports (NFR-9): adapters (`BinanceDataSource` in
//! WI-02, a `Parquet`-replay source later) implement them; the engine consumes
//! them generically, never as `dyn`. The domain stays free of `tokio`/`reqwest`;
//! only the trait shapes live here.
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
use crate::domain::strategy::{NewVersion, Strategy, StrategyId, StrategyVersion, VersionId};
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

/// The strategy-tree persistence port (VS-1.1.4, FR-4 / FR-11).
///
/// The `sqlx` adapter (1.03) implements it over `pulse.db`; the CLI (1.05) and
/// agent layer consume it generically (`<R: StrategyRepository>`), never as
/// `dyn`. Same `Send`-future style as [`MarketDataSource`] (audit C3) so a
/// repository call can be `spawn`ed on tokio's multi-thread runtime.
///
/// **Versions are create + read only (FR-4 immutability).** There is
/// deliberately NO `update_version` / `delete_version` method — a version's
/// shape is structurally immutable in this API (the DB triggers in 1.01 are the
/// second guard). The FR-11 "compare" op is NOT a port method: it is the pure
/// [`diff_versions`](crate::domain::strategy::diff_versions) fn over two
/// `get_version` results.
///
/// `get_strategy` / `get_version` return `Option<_>` (an absent id is `Ok(None)`,
/// not an error). Ids/`version_hash`/timestamps are the adapter's to mint — this
/// trait only declares the contract every sibling reads through.
pub trait StrategyRepository {
    /// Create a new strategy and return the freshly-built record (with the
    /// adapter's generated id + timestamp).
    ///
    /// # Errors
    ///
    /// Returns [`DataError`] if the underlying store fails.
    fn create_strategy(
        &self,
        name: &str,
        owner: Option<&str>,
        tags: &[String],
    ) -> impl Future<Output = Result<Strategy, DataError>> + Send;

    /// Fetch a strategy by id (`Ok(None)` if no such row).
    ///
    /// # Errors
    ///
    /// Returns [`DataError`] if the underlying store fails.
    fn get_strategy(
        &self,
        id: &StrategyId,
    ) -> impl Future<Output = Result<Option<Strategy>, DataError>> + Send;

    /// List strategies (FR-11 browse); `include_archived` toggles archived rows.
    ///
    /// # Errors
    ///
    /// Returns [`DataError`] if the underlying store fails.
    fn list_strategies(
        &self,
        include_archived: bool,
    ) -> impl Future<Output = Result<Vec<Strategy>, DataError>> + Send;

    /// Rename a strategy, returning the updated record.
    ///
    /// # Errors
    ///
    /// Returns [`DataError`] if the underlying store fails or the id is absent.
    fn rename_strategy(
        &self,
        id: &StrategyId,
        new_name: &str,
    ) -> impl Future<Output = Result<Strategy, DataError>> + Send;

    /// Set a strategy's tags (FR-11 tag), returning the updated record.
    ///
    /// # Errors
    ///
    /// Returns [`DataError`] if the underlying store fails or the id is absent.
    fn set_tags(
        &self,
        id: &StrategyId,
        tags: &[String],
    ) -> impl Future<Output = Result<Strategy, DataError>> + Send;

    /// Set (or clear) a strategy's pinned version (FR-11 pin). 1.03 validates
    /// `version ∈ strategy`; the signature only declares it.
    ///
    /// # Errors
    ///
    /// Returns [`DataError`] if the underlying store fails, the id is absent, or
    /// the version does not belong to the strategy.
    fn set_pinned_version(
        &self,
        id: &StrategyId,
        version_id: Option<&VersionId>,
    ) -> impl Future<Output = Result<Strategy, DataError>> + Send;

    /// Archive or un-archive a strategy (FR-11 archive), returning the record.
    ///
    /// # Errors
    ///
    /// Returns [`DataError`] if the underlying store fails or the id is absent.
    fn archive_strategy(
        &self,
        id: &StrategyId,
        archived: bool,
    ) -> impl Future<Output = Result<Strategy, DataError>> + Send;

    /// Create an immutable version from a [`NewVersion`] request (FR-11 clone =
    /// parent set; FR-4 immutable). The adapter mints the id/`version_hash`/
    /// `created_at` and routes `dsl_json` through the `Migrator`.
    ///
    /// # Errors
    ///
    /// Returns [`DataError`] if the underlying store fails or the DSL is invalid.
    fn create_version(
        &self,
        request: NewVersion,
    ) -> impl Future<Output = Result<StrategyVersion, DataError>> + Send;

    /// Fetch a version by id (`Ok(None)` if no such row).
    ///
    /// # Errors
    ///
    /// Returns [`DataError`] if the underlying store fails.
    fn get_version(
        &self,
        id: &VersionId,
    ) -> impl Future<Output = Result<Option<StrategyVersion>, DataError>> + Send;

    /// List all versions of a strategy.
    ///
    /// # Errors
    ///
    /// Returns [`DataError`] if the underlying store fails.
    fn list_versions(
        &self,
        strategy_id: &StrategyId,
    ) -> impl Future<Output = Result<Vec<StrategyVersion>, DataError>> + Send;

    /// Browse the strategy's version subtree, parent-ordered (FR-11 browse).
    /// 1.03 owns the topological ordering.
    ///
    /// # Errors
    ///
    /// Returns [`DataError`] if the underlying store fails.
    fn version_tree(
        &self,
        strategy_id: &StrategyId,
    ) -> impl Future<Output = Result<Vec<StrategyVersion>, DataError>> + Send;
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod repository_tests {
    use super::StrategyRepository;
    use crate::domain::error::DataError;
    use crate::domain::strategy::{
        CreatedBy, NewVersion, Strategy, StrategyId, StrategyVersion, VersionId,
    };
    use std::collections::HashMap;
    use std::sync::Mutex;

    use crate::domain::dsl::{
        Comparator, Condition, Direction, ExitRule, IndicatorSpec, RiskParams, SchemaVersion,
        StrategyDsl, SweepableValue, ValueSource,
    };
    use chrono::{TimeZone, Utc};
    use rust_decimal::Decimal;

    /// A canonical `1.0.0` typed DSL the fake embeds in created versions.
    fn canonical_dsl() -> StrategyDsl {
        StrategyDsl {
            schema_version: SchemaVersion::CURRENT,
            name: "RSI Oversold".to_owned(),
            direction: Direction::Long,
            entry: Condition::Compare {
                lhs: ValueSource::Indicator {
                    spec: IndicatorSpec::Rsi {
                        period: SweepableValue::Fixed(14),
                    },
                },
                op: Comparator::Lt,
                rhs: ValueSource::Constant {
                    value: Decimal::new(30, 0),
                },
            },
            filters: vec![],
            exits: vec![ExitRule::TakeProfit {
                target_r: SweepableValue::Fixed(Decimal::new(2, 0)),
            }],
            risk: RiskParams {
                risk_per_trade_pct: SweepableValue::Fixed(Decimal::new(1, 2)),
                max_leverage: SweepableValue::Fixed(Decimal::new(3, 0)),
            },
        }
    }

    /// An in-memory, zero-I/O fake implementing `StrategyRepository`. It locks
    /// the trait shape (create + read only for versions — no update/delete
    /// method to implement) and proves the futures are `Send`-spawnable. The
    /// `Mutex`-guarded maps keep the fake `Send + Sync` so its futures are `Send`
    /// even while borrowing `&self`.
    #[derive(Default)]
    struct FakeRepo {
        strategies: Mutex<HashMap<String, Strategy>>,
        versions: Mutex<HashMap<String, StrategyVersion>>,
        seq: Mutex<u64>,
    }

    impl FakeRepo {
        fn next_id(&self, prefix: &str) -> String {
            let mut seq = self.seq.lock().expect("seq lock");
            *seq += 1;
            format!("{prefix}-{seq}")
        }
    }

    impl StrategyRepository for FakeRepo {
        async fn create_strategy(
            &self,
            name: &str,
            owner: Option<&str>,
            tags: &[String],
        ) -> Result<Strategy, DataError> {
            let strat = Strategy {
                id: StrategyId::new(self.next_id("strat")),
                name: name.to_owned(),
                tags: tags.to_vec(),
                owner: owner.map(ToOwned::to_owned),
                pinned_version_id: None,
                archived: false,
                created_at: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
            };
            self.strategies
                .lock()
                .expect("strategies lock")
                .insert(strat.id.as_str().to_owned(), strat.clone());
            Ok(strat)
        }

        async fn get_strategy(&self, id: &StrategyId) -> Result<Option<Strategy>, DataError> {
            Ok(self
                .strategies
                .lock()
                .expect("strategies lock")
                .get(id.as_str())
                .cloned())
        }

        async fn list_strategies(
            &self,
            include_archived: bool,
        ) -> Result<Vec<Strategy>, DataError> {
            Ok(self
                .strategies
                .lock()
                .expect("strategies lock")
                .values()
                .filter(|s| include_archived || !s.archived)
                .cloned()
                .collect())
        }

        async fn rename_strategy(
            &self,
            id: &StrategyId,
            new_name: &str,
        ) -> Result<Strategy, DataError> {
            let mut map = self.strategies.lock().expect("strategies lock");
            let strat = map
                .get_mut(id.as_str())
                .ok_or_else(|| DataError::Io("no such strategy".to_owned()))?;
            strat.name = new_name.to_owned();
            Ok(strat.clone())
        }

        async fn set_tags(&self, id: &StrategyId, tags: &[String]) -> Result<Strategy, DataError> {
            let mut map = self.strategies.lock().expect("strategies lock");
            let strat = map
                .get_mut(id.as_str())
                .ok_or_else(|| DataError::Io("no such strategy".to_owned()))?;
            strat.tags = tags.to_vec();
            Ok(strat.clone())
        }

        async fn set_pinned_version(
            &self,
            id: &StrategyId,
            version_id: Option<&VersionId>,
        ) -> Result<Strategy, DataError> {
            let mut map = self.strategies.lock().expect("strategies lock");
            let strat = map
                .get_mut(id.as_str())
                .ok_or_else(|| DataError::Io("no such strategy".to_owned()))?;
            strat.pinned_version_id = version_id.cloned();
            Ok(strat.clone())
        }

        async fn archive_strategy(
            &self,
            id: &StrategyId,
            archived: bool,
        ) -> Result<Strategy, DataError> {
            let mut map = self.strategies.lock().expect("strategies lock");
            let strat = map
                .get_mut(id.as_str())
                .ok_or_else(|| DataError::Io("no such strategy".to_owned()))?;
            strat.archived = archived;
            Ok(strat.clone())
        }

        async fn create_version(&self, request: NewVersion) -> Result<StrategyVersion, DataError> {
            let version = StrategyVersion {
                id: VersionId::new(self.next_id("ver")),
                strategy_id: request.strategy_id,
                parent_version_id: request.parent_version_id,
                dsl_schema_version: SchemaVersion::CURRENT,
                dsl: canonical_dsl(),
                dsl_original: request.dsl_json,
                version_hash: "deadbeef".to_owned(),
                created_by: request.created_by,
                creating_llm_call_ids: request.creating_llm_call_ids,
                created_at: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
            };
            self.versions
                .lock()
                .expect("versions lock")
                .insert(version.id.as_str().to_owned(), version.clone());
            Ok(version)
        }

        async fn get_version(&self, id: &VersionId) -> Result<Option<StrategyVersion>, DataError> {
            Ok(self
                .versions
                .lock()
                .expect("versions lock")
                .get(id.as_str())
                .cloned())
        }

        async fn list_versions(
            &self,
            strategy_id: &StrategyId,
        ) -> Result<Vec<StrategyVersion>, DataError> {
            Ok(self
                .versions
                .lock()
                .expect("versions lock")
                .values()
                .filter(|v| &v.strategy_id == strategy_id)
                .cloned()
                .collect())
        }

        async fn version_tree(
            &self,
            strategy_id: &StrategyId,
        ) -> Result<Vec<StrategyVersion>, DataError> {
            self.list_versions(strategy_id).await
        }
    }

    /// Generic consumption (`<R: StrategyRepository>`) proves the port is used by
    /// bound, not as `dyn`, and that the create→read futures are `Send` (required
    /// by `spawn`). Round-trips a strategy then a version.
    async fn roundtrip_via<R: StrategyRepository>(
        repo: R,
    ) -> Result<(Strategy, StrategyVersion), DataError> {
        let created = repo
            .create_strategy("Demo", Some("alice"), &["btc".to_owned()])
            .await?;
        let fetched = repo
            .get_strategy(&created.id)
            .await?
            .expect("created strategy is fetchable");

        let version = repo
            .create_version(NewVersion {
                strategy_id: fetched.id.clone(),
                parent_version_id: None,
                dsl_json: r#"{"schema_version":"1.0.0"}"#.to_owned(),
                created_by: CreatedBy::Human,
                creating_llm_call_ids: vec![],
            })
            .await?;
        let fetched_version = repo
            .get_version(&version.id)
            .await?
            .expect("created version is fetchable");

        Ok((fetched, fetched_version))
    }

    /// AC-17 (FR-11): the port double is `Send`-spawnable and create→read
    /// round-trips for both a strategy and a version. The ABSENCE of any
    /// `update_version`/`delete_version` method is demonstrated by `FakeRepo`
    /// having none to implement (FR-4 immutability is structural in the API).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn repository_port_double_create_read_only() {
        // If the port's futures were not `Send`, this `spawn` would not compile.
        let handle = tokio::spawn(async { roundtrip_via(FakeRepo::default()).await });
        let (strat, version) = handle
            .await
            .expect("spawned task joins")
            .expect("round-trip succeeds");

        assert_eq!(strat.name, "Demo");
        assert_eq!(strat.owner.as_deref(), Some("alice"));
        assert_eq!(version.strategy_id, strat.id);
        assert_eq!(version.created_by, CreatedBy::Human);
        // The verbatim source bytes survived the create→read round-trip.
        assert_eq!(version.dsl_original, r#"{"schema_version":"1.0.0"}"#);
    }
}
