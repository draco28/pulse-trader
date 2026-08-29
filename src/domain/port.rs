//! Domain ports — the hexagonal seams the outer rings implement.
//!
//! - [`MarketDataSource`] — historical/incremental candle data (WI-02 onward).
//! - [`StrategyRepository`] — the strategy-tree persistence port (VS-1.1.4,
//!   FR-4 / FR-11); the `sqlx` adapter (1.03) implements it, the CLI (1.05) and
//!   agent layer consume it generically (`<R: StrategyRepository>`).
//! - [`ExchangeAdapter`] — the exchange-metadata port (VS-1.2.2, FR-5 / NFR-3);
//!   `BinanceAdapter` (`adapters/broker`) implements it, the sizer consumes the
//!   returned [`SymbolFilters`](crate::domain::sizing::SymbolFilters). Minimal v1
//!   surface (NO order/account/balance methods); returns a dedicated
//!   [`ExchangeError`](crate::domain::exchange::ExchangeError), not
//!   `BacktestError` (audit C5).
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

use crate::domain::backtest::SummaryStats;
use crate::domain::backtest::{BacktestResult, BacktestRunId, PersistedRun, RunSummary, Trade};
use crate::domain::candle::Candle;
use crate::domain::coaching::{CoachingSession, CoachingSessionId, Disposition};
use crate::domain::error::DataError;
use crate::domain::exchange::ExchangeError;
use crate::domain::llm::{LlmConfig, LlmError, LlmResponse, Message, ToolDefinition};
use crate::domain::llm_call::{LlmCall, LlmCallId};
use crate::domain::pair::Pair;
use crate::domain::series::CandleSeries;
use crate::domain::sizing::SymbolFilters;
use crate::domain::strategy::{NewVersion, Strategy, StrategyId, StrategyVersion, VersionId};
use crate::domain::timeframe::Timeframe;
use rust_decimal::Decimal;

/// The exchange-metadata port (VS-1.2.2, FR-5 / NFR-3) — audit C3 / C5.
///
/// Supplies a symbol's [`SymbolFilters`] (lot step / min qty / min notional /
/// exchange max-leverage) so the shared
/// [`compute_position_size`](crate::domain::sizing::compute_position_size) can
/// apply exchange constraints. **Minimal v1 surface** — NO order / cancel /
/// account / balance methods (those land with live execution in v3). Returns the
/// **dedicated** [`ExchangeError`] (NOT [`DataError`] / `BacktestError`) because
/// the port serves live execution too (audit C5).
///
/// Synchronous: v1's only implementor (`BinanceAdapter`) returns **pinned
/// consts** with no I/O. A future networked filter-fetch adapter can wrap a cache
/// behind the same sync surface, or this method gains an async sibling additively.
pub trait ExchangeAdapter {
    /// Return the [`SymbolFilters`] for `pair`.
    ///
    /// # Errors
    ///
    /// Returns [`ExchangeError::UnknownSymbol`] when the adapter has no filters
    /// pinned for `pair` (v1's `BinanceAdapter` only knows `BTCUSDT`).
    fn symbol_filters(&self, pair: &Pair) -> Result<SymbolFilters, ExchangeError>;
}

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

/// The persisted backtest-run system-of-record port (VS-1.2.4 work-4.04, FR-6 /
/// FR-7 / NFR-2).
///
/// The `sqlx` adapter
/// ([`SqliteBacktestRunRepo`](crate::adapters::db::SqliteBacktestRunRepo))
/// implements it over `pulse.db`; the CLI (4.05) consumes it generically
/// (`<R: BacktestRunRepository>`), never as `dyn`. Same `Send`-future style as
/// [`StrategyRepository`] (audit C3) so a repository call can be `spawn`ed on
/// tokio's multi-thread runtime.
///
/// **Runs are create + read only.** A persisted [`BacktestRun`](PersistedRun) is
/// append-only live-capital provenance (FR-6) — there is deliberately NO
/// `update_run` / `delete_run` method; immutability is structural in this API and
/// enforced by the migration-`0003` `BEFORE UPDATE` / `BEFORE DELETE` triggers.
///
/// **#39 integrity guarantees:**
/// - `save_run` is **ownership-on-write** (the `strategy_version_id` must exist).
/// - `get_run` is **re-validate-on-read**: because `result_content_hash` is
///   trade-dependent, it fetches the run's `trade` rows internally and re-derives
///   the full hash input, rejecting a mismatch (tamper guard).
/// - **Corrupt-isolation is scoped to `list_runs_for_version` ONLY** (a catalog
///   is best-effort UX — a corrupt summary row is skipped-with-warning).
///   `get_run`, `latest_run_for_version`, and `get_trades` **fail-closed**: any
///   corrupt/un-parseable row is an `Err`, never a partial result (a trade log
///   feeds P&L/equity/hash reconstruction).
pub trait BacktestRunRepository {
    /// Persist a finished run + all its trades (FR-6, #39 ownership-on-write).
    ///
    /// Asserts the `strategy_version_id` row exists (FK + an explicit `SELECT 1`
    /// guard INSIDE the transaction); INSERTs the run + every trade + reads the
    /// run header back in ONE transaction; stores the `result_content_hash`. The
    /// `created_at` is sourced from the injected `Clock`.
    ///
    /// # Errors
    ///
    /// Returns [`DataError::Db`] if the `strategy_version_id` is absent /
    /// cross-strategy (no row persists), if a `sharpe`/`sortino` value is
    /// non-finite (fail-closed at the boundary), or the store fails.
    fn save_run(
        &self,
        strategy_version_id: &VersionId,
        result: &BacktestResult,
        summary: &SummaryStats,
        starting_equity: Decimal,
    ) -> impl Future<Output = Result<BacktestRunId, DataError>> + Send;

    /// Fetch one persisted run by id (`Ok(None)` if no such row), **#39
    /// re-validate-on-read**.
    ///
    /// Fetches the run's `trade` rows internally in the same read and reconstructs
    /// the full hash input (run totals + `regime_breakdown` + `skipped_entries` +
    /// the `seq`-ordered trades) to re-derive `result_content_hash`, rejecting a
    /// mismatch (tamper guard). Fail-closed on any corrupt/un-parseable row.
    ///
    /// # Errors
    ///
    /// Returns [`DataError::Db`] on a re-derived-hash mismatch, an unsupported
    /// stored `schema_version`, a malformed/corrupt column, or a store failure.
    fn get_run(
        &self,
        id: &BacktestRunId,
    ) -> impl Future<Output = Result<Option<PersistedRun>, DataError>> + Send;

    /// The most-recent run for a version (`ORDER BY created_at DESC, id DESC LIMIT
    /// 1`, #40 stable) — the FR-7 prior-run lookup. Reuses `get_run`'s
    /// fetch-trades-and-validate path (so it is fail-closed + tamper-checked).
    ///
    /// # Errors
    ///
    /// Returns [`DataError::Db`] on the same conditions as [`get_run`](Self::get_run).
    fn latest_run_for_version(
        &self,
        strategy_version_id: &VersionId,
    ) -> impl Future<Output = Result<Option<PersistedRun>, DataError>> + Send;

    /// List the run catalog for a version (`ORDER BY created_at, id`, #40 stable).
    ///
    /// **The ONLY method with #39 per-row corrupt-isolation** — a corrupt summary
    /// row is skipped-with-warning (`tracing::warn`), not a whole-list failure,
    /// because a run catalog is best-effort UX.
    ///
    /// # Errors
    ///
    /// Returns [`DataError::Db`] only on a store-level failure (the query itself
    /// failing), never for a single un-parseable summary row (that is skipped).
    fn list_runs_for_version(
        &self,
        strategy_version_id: &VersionId,
    ) -> impl Future<Output = Result<Vec<RunSummary>, DataError>> + Send;

    /// Fetch a run's full trade log (`ORDER BY seq`, #40 stable). **Fail-closed**
    /// (audit C9): a trade log feeds P&L/equity/hash reconstruction, so a
    /// corrupt/missing trade row is an `Err`, never silently skipped.
    ///
    /// # Errors
    ///
    /// Returns [`DataError::Db`] on any corrupt/un-parseable trade row, an
    /// unsupported stored `schema_version`, or a store failure.
    fn get_trades(
        &self,
        id: &BacktestRunId,
    ) -> impl Future<Output = Result<Vec<Trade>, DataError>> + Send;
}

/// `PulseTrader`'s own LLM chat port (VS-1.3.1 work-1.01, FR-23 / FR-24, README
/// C1).
///
/// The **established** port style — `impl Future<Output = ...> + Send`, consumed
/// generically (`<P: LlmProvider>`), **never `dyn`** — mirrors
/// [`MarketDataSource`] / [`StrategyRepository`] / [`BacktestRunRepository`]. This
/// is deliberately NOT `PulseHive`'s `#[async_trait]` + `dyn`-safe trait: the
/// shapes are `PulseTrader`-OWNED so `PulseHive`'s evolving 2.x API cannot ripple
/// inward (ADR-0012). The
/// [`LlmBackend`](crate::domain::llm::LlmBackend) on
/// [`LlmConfig`](crate::domain::llm::LlmConfig) IS FR-23's "config flag" — the
/// composition root (1.05) `match`es on it and constructs the concrete provider;
/// adding a backend is a new adapter + match arm, no domain refactor.
///
/// **Non-streaming ONLY (v1):** the cost-logged path needs `usage`, and only
/// `chat()` carries it (ADR-0012); streaming is a v1.5 GUI concern. The port takes
/// `tools` **additively** (VS-1.3.2 work-2.01, FR-3): a borrowed
/// `&[ToolDefinition]` slice the composer advertises so the model may return
/// `tool_calls`. An **empty slice** reproduces the VS-1.3.1 no-tools behavior
/// exactly (a response still carries `tool_calls` for forward-compat).
pub trait LlmProvider {
    /// Run one non-streaming chat completion, advertising `tools` to the model.
    ///
    /// `tools` is a borrowed slice of [`ToolDefinition`]s the model may call; pass
    /// `&[]` for the no-tools flow (identical to the VS-1.3.1 behavior).
    ///
    /// # Errors
    ///
    /// Returns [`LlmError`] if the provider / transport fails
    /// ([`LlmError::Provider`]) or the request / config is invalid
    /// ([`LlmError::Config`]).
    fn chat(
        &self,
        messages: Vec<Message>,
        tools: &[ToolDefinition],
        config: &LlmConfig,
    ) -> impl Future<Output = Result<LlmResponse, LlmError>> + Send;
}

/// `PulseTrader`'s append-only [`LlmCall`] ledger persistence port (VS-1.3.1
/// work-1.02, FR-24, README C6).
///
/// The **established** repository style — `impl Future<Output = ...> + Send`,
/// consumed generically (`<R: LlmCallRepository>`), **never `dyn`** — mirrors
/// [`BacktestRunRepository`] / [`StrategyRepository`] (audit C3), so a repository
/// call can be `spawn`ed on tokio's multi-thread runtime. The `SqliteLlmCallRepo`
/// (`adapters::db`) implements it over `pulse.db`; the 1.04 decorator writes
/// through it and the 1.05 demo reads a row back.
///
/// **Calls are create + read only.** A persisted [`LlmCall`] is an append-only
/// verbatim ledger record (FR-24) — there is deliberately NO `update_call` /
/// `delete_call` method; immutability is structural in this API and enforced by the
/// migration-`0004` `BEFORE UPDATE` / `BEFORE DELETE` triggers.
///
/// **Errors are [`DataError::Db`]**, the shared persistence error (like the other
/// repos) — NOT [`LlmError`], which is the *provider*-port error only.
pub trait LlmCallRepository {
    /// Persist one [`LlmCall`] ledger row (append-only, FR-24).
    ///
    /// The stored `created_at` is sourced from the adapter's injected `Clock`
    /// (deterministic under test); every other field is persisted verbatim.
    /// Returns the persisted [`LlmCallId`].
    ///
    /// # Errors
    ///
    /// Returns [`DataError::Db`] if the store fails.
    fn save_call(
        &self,
        call: &LlmCall,
    ) -> impl Future<Output = Result<LlmCallId, DataError>> + Send;

    /// Fetch one persisted call by id (`Ok(None)` if no such row).
    ///
    /// **Fail-closed** (mirror [`BacktestRunRepository::get_run`]): an unsupported
    /// stored `schema_version` or a corrupt/un-parseable column is an `Err`, never a
    /// silent partial.
    ///
    /// # Errors
    ///
    /// Returns [`DataError::Db`] on an unsupported stored `schema_version`, a
    /// malformed column, or a store failure.
    fn get_call(
        &self,
        id: &LlmCallId,
    ) -> impl Future<Output = Result<Option<LlmCall>, DataError>> + Send;
}

/// The coaching session persistence port (r1.s2.w2, ADR-0021 / audit C3).
///
/// The `sqlx` adapter
/// ([`SqliteCoachingRepo`](crate::adapters::db::SqliteCoachingRepo)) implements it
/// over `pulse.db`; `w3`'s coach turn and `r1.s4`'s rail consume it generically
/// (`<R: CoachingRepository>`), never as `dyn`. Same `Send`-future style as
/// [`StrategyRepository`] (audit C3) so a repository call can be `spawn`ed.
///
/// **The session row IS the audit trail.** Every turn persists — a proposal or a
/// typed failure — so "never silence" is a storage guarantee rather than a
/// convention, and `llm_call_id` is `None` precisely when no provider call was
/// made.
///
/// **This port persists; it does not decide.** The disposition state machine lives
/// in [`Proposal::transition`](crate::domain::Proposal::transition), and
/// [`record_disposition`](CoachingRepository::record_disposition) writes the
/// disposition it is given. The `0005` `CHECK` constraints are the second guard
/// (no accepted proposal without its child version, and no child version on
/// anything else); the legality of `rejected → accepted` is the caller's to
/// enforce through the domain, which is where `r1.s4`'s rail already goes.
///
/// **No `validated` anywhere** (audit C4): a stored mutation's applicability is
/// re-established by [`apply`](crate::domain::apply) at use time, so there is
/// deliberately no method here that reads or writes such a fact.
pub trait CoachingRepository {
    /// Persist one coach turn — the session row, plus its proposal row when the
    /// turn produced one.
    ///
    /// The stored `created_at` is sourced from the adapter's injected `Clock`
    /// (deterministic under test); every other field is persisted verbatim.
    /// Returns the persisted [`CoachingSessionId`].
    ///
    /// # Errors
    ///
    /// Returns [`DataError::Db`] if the store fails — including the re-save of a
    /// session id that already exists, which the primary key and the
    /// one-proposal-per-session `UNIQUE` refuse rather than silently double.
    fn save_session(
        &self,
        session: &CoachingSession,
    ) -> impl Future<Output = Result<CoachingSessionId, DataError>> + Send;

    /// Fetch one recorded turn by id (`Ok(None)` if no such row).
    ///
    /// **Fail-closed** (mirror [`LlmCallRepository::get_call`]): an unsupported
    /// stored `schema_version` or a corrupt/un-parseable column is an `Err`, never
    /// a silent partial — a coaching record that reads back wrong is worse than one
    /// that refuses to read.
    ///
    /// # Errors
    ///
    /// Returns [`DataError::Db`] on an unsupported stored `schema_version`, a
    /// malformed column, or a store failure.
    fn get_session(
        &self,
        id: &CoachingSessionId,
    ) -> impl Future<Output = Result<Option<CoachingSession>, DataError>> + Send;

    /// Every recorded turn for one backtest run, oldest first — successes and
    /// failures alike.
    ///
    /// # Errors
    ///
    /// Returns [`DataError::Db`] on a malformed row or a store failure.
    fn list_sessions_for_run(
        &self,
        run_id: &BacktestRunId,
    ) -> impl Future<Output = Result<Vec<CoachingSession>, DataError>> + Send;

    /// Record a proposal's disposition, keyed by its session id — the accept
    /// idempotency key (`r1.s4`'s consistency model keys one child version per
    /// proposal by session id).
    ///
    /// Dormant in `r1.s2`: `w2` writes only the `Proposed` state, and `r1.s4`'s
    /// rail is what drives the rest.
    ///
    /// # Errors
    ///
    /// Returns [`DataError::Db`] when the session has no proposal to disposition
    /// (an absent session, or a turn that failed), or when the store rejects the
    /// write — including the `0005` `CHECK` that an accepted proposal must name its
    /// child version and nothing else may.
    fn record_disposition(
        &self,
        id: &CoachingSessionId,
        disposition: &Disposition,
    ) -> impl Future<Output = Result<(), DataError>> + Send;
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
    use std::future::Future;

    /// A trivial generic fake implementing the port with no I/O.
    struct FakeSource;

    impl MarketDataSource for FakeSource {
        fn fetch_historical(
            &self,
            pair: &Pair,
            tf: Timeframe,
            _start_ms: i64,
            _end_ms: i64,
        ) -> impl Future<Output = Result<CandleSeries, DataError>> {
            std::future::ready(Ok(CandleSeries {
                pair: pair.clone(),
                timeframe: tf,
                version: DataVersion::new("fake"),
                candles: Vec::new(),
            }))
        }

        fn fetch_incremental(
            &self,
            _pair: &Pair,
            _tf: Timeframe,
            _since_ms: i64,
        ) -> impl Future<Output = Result<Vec<Candle>, DataError>> {
            std::future::ready(Ok(Vec::new()))
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
    use std::future::Future;

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
        fn create_strategy(
            &self,
            name: &str,
            owner: Option<&str>,
            tags: &[String],
        ) -> impl Future<Output = Result<Strategy, DataError>> {
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
            std::future::ready(Ok(strat))
        }

        fn get_strategy(
            &self,
            id: &StrategyId,
        ) -> impl Future<Output = Result<Option<Strategy>, DataError>> {
            std::future::ready(Ok(self
                .strategies
                .lock()
                .expect("strategies lock")
                .get(id.as_str())
                .cloned()))
        }

        fn list_strategies(
            &self,
            include_archived: bool,
        ) -> impl Future<Output = Result<Vec<Strategy>, DataError>> {
            std::future::ready(Ok(self
                .strategies
                .lock()
                .expect("strategies lock")
                .values()
                .filter(|s| include_archived || !s.archived)
                .cloned()
                .collect()))
        }

        fn rename_strategy(
            &self,
            id: &StrategyId,
            new_name: &str,
        ) -> impl Future<Output = Result<Strategy, DataError>> {
            std::future::ready((|| {
                let mut map = self.strategies.lock().expect("strategies lock");
                let strat = map
                    .get_mut(id.as_str())
                    .ok_or_else(|| DataError::Io("no such strategy".to_owned()))?;
                strat.name = new_name.to_owned();
                Ok(strat.clone())
            })())
        }

        fn set_tags(
            &self,
            id: &StrategyId,
            tags: &[String],
        ) -> impl Future<Output = Result<Strategy, DataError>> {
            std::future::ready((|| {
                let mut map = self.strategies.lock().expect("strategies lock");
                let strat = map
                    .get_mut(id.as_str())
                    .ok_or_else(|| DataError::Io("no such strategy".to_owned()))?;
                strat.tags = tags.to_vec();
                Ok(strat.clone())
            })())
        }

        fn set_pinned_version(
            &self,
            id: &StrategyId,
            version_id: Option<&VersionId>,
        ) -> impl Future<Output = Result<Strategy, DataError>> {
            std::future::ready((|| {
                let mut map = self.strategies.lock().expect("strategies lock");
                let strat = map
                    .get_mut(id.as_str())
                    .ok_or_else(|| DataError::Io("no such strategy".to_owned()))?;
                strat.pinned_version_id = version_id.cloned();
                Ok(strat.clone())
            })())
        }

        fn archive_strategy(
            &self,
            id: &StrategyId,
            archived: bool,
        ) -> impl Future<Output = Result<Strategy, DataError>> {
            std::future::ready((|| {
                let mut map = self.strategies.lock().expect("strategies lock");
                let strat = map
                    .get_mut(id.as_str())
                    .ok_or_else(|| DataError::Io("no such strategy".to_owned()))?;
                strat.archived = archived;
                Ok(strat.clone())
            })())
        }

        fn create_version(
            &self,
            request: NewVersion,
        ) -> impl Future<Output = Result<StrategyVersion, DataError>> {
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
            std::future::ready(Ok(version))
        }

        fn get_version(
            &self,
            id: &VersionId,
        ) -> impl Future<Output = Result<Option<StrategyVersion>, DataError>> {
            std::future::ready(Ok(self
                .versions
                .lock()
                .expect("versions lock")
                .get(id.as_str())
                .cloned()))
        }

        fn list_versions(
            &self,
            strategy_id: &StrategyId,
        ) -> impl Future<Output = Result<Vec<StrategyVersion>, DataError>> {
            std::future::ready(Ok(self
                .versions
                .lock()
                .expect("versions lock")
                .values()
                .filter(|v| &v.strategy_id == strategy_id)
                .cloned()
                .collect()))
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod backtest_run_repository_tests {
    use super::BacktestRunRepository;
    use crate::domain::backtest::{
        BacktestResult, BacktestRunId, PersistedRun, RunSummary, SummaryStats, Trade,
    };
    use crate::domain::error::DataError;
    use crate::domain::strategy::VersionId;
    use rust_decimal::Decimal;
    use std::collections::HashMap;
    use std::future::Future;
    use std::sync::Mutex;

    /// An in-memory, zero-I/O fake implementing `BacktestRunRepository`. It locks
    /// the trait shape (create + read only — no `update_run`/`delete_run` to
    /// implement) and proves the futures are `Send`-spawnable. The `Mutex`-guarded
    /// map keeps the fake `Send + Sync` so its futures are `Send` even while
    /// borrowing `&self` (mirror `FakeRepo`).
    #[derive(Default)]
    struct FakeRunRepo {
        runs: Mutex<HashMap<String, (PersistedRun, Vec<Trade>)>>,
        seq: Mutex<u64>,
    }

    impl FakeRunRepo {
        fn next_id(&self) -> String {
            let mut seq = self.seq.lock().expect("seq lock");
            *seq += 1;
            format!("run-{seq}")
        }
    }

    impl BacktestRunRepository for FakeRunRepo {
        fn save_run(
            &self,
            strategy_version_id: &VersionId,
            result: &BacktestResult,
            summary: &SummaryStats,
            starting_equity: Decimal,
        ) -> impl Future<Output = Result<BacktestRunId, DataError>> {
            let id = BacktestRunId::new(self.next_id());
            let persisted = PersistedRun {
                id: id.clone(),
                strategy_version_id: strategy_version_id.clone(),
                schema_version: 1,
                created_at: "2026-06-30T00:00:00.000Z".to_owned(),
                engine_fingerprint: result.engine_fingerprint.as_str().to_owned(),
                engine_target: "test-target".to_owned(),
                result_content_hash: result.result_content_hash(),
                starting_equity,
                net_pnl: result.net_pnl,
                fees_total: result.fees_total,
                funding_total: result.funding_total,
                slippage_total: result.slippage_total,
                summary: summary.clone(),
            };
            self.runs
                .lock()
                .expect("runs lock")
                .insert(id.as_str().to_owned(), (persisted, result.trades.clone()));
            std::future::ready(Ok(id))
        }

        fn get_run(
            &self,
            id: &BacktestRunId,
        ) -> impl Future<Output = Result<Option<PersistedRun>, DataError>> {
            std::future::ready(Ok(self
                .runs
                .lock()
                .expect("runs lock")
                .get(id.as_str())
                .map(|(run, _)| run.clone())))
        }

        fn latest_run_for_version(
            &self,
            strategy_version_id: &VersionId,
        ) -> impl Future<Output = Result<Option<PersistedRun>, DataError>> {
            std::future::ready(Ok(self
                .runs
                .lock()
                .expect("runs lock")
                .values()
                .filter(|(run, _)| &run.strategy_version_id == strategy_version_id)
                .map(|(run, _)| run.clone())
                .last()))
        }

        fn list_runs_for_version(
            &self,
            strategy_version_id: &VersionId,
        ) -> impl Future<Output = Result<Vec<RunSummary>, DataError>> {
            std::future::ready(Ok(self
                .runs
                .lock()
                .expect("runs lock")
                .values()
                .filter(|(run, _)| &run.strategy_version_id == strategy_version_id)
                .map(|(run, _)| RunSummary {
                    id: run.id.clone(),
                    strategy_version_id: run.strategy_version_id.clone(),
                    schema_version: run.schema_version,
                    created_at: run.created_at.clone(),
                    engine_fingerprint: run.engine_fingerprint.clone(),
                    engine_target: run.engine_target.clone(),
                    result_content_hash: run.result_content_hash.clone(),
                    net_pnl: run.net_pnl,
                    expectancy: run.summary.expectancy,
                    trade_count: run.summary.trade_count,
                })
                .collect()))
        }

        fn get_trades(
            &self,
            id: &BacktestRunId,
        ) -> impl Future<Output = Result<Vec<Trade>, DataError>> {
            std::future::ready(Ok(self
                .runs
                .lock()
                .expect("runs lock")
                .get(id.as_str())
                .map(|(_, trades)| trades.clone())
                .unwrap_or_default()))
        }
    }

    /// Generic consumption (`<R: BacktestRunRepository>`) proves the port is used by
    /// bound, not as `dyn`, and that the save→read futures are `Send` (required by
    /// `spawn`). Round-trips a saved run.
    async fn roundtrip_via<R: BacktestRunRepository>(
        repo: R,
    ) -> Result<(PersistedRun, Vec<Trade>), DataError> {
        let version_id = VersionId::new("ver-1");
        let result = BacktestResult {
            trades: Vec::new(),
            net_pnl: Decimal::ZERO,
            fees_total: Decimal::ZERO,
            funding_total: Decimal::ZERO,
            slippage_total: Decimal::ZERO,
            regime_breakdown: crate::domain::backtest::RegimeBreakdown::default(),
            skipped_entries: crate::domain::sizing::SkippedEntryCounts::default(),
            engine_fingerprint: crate::domain::EngineFingerprint::current(),
            summary: SummaryStats::default(),
            equity_curve: crate::domain::backtest::EquityCurve::default(),
        };
        let id = repo
            .save_run(
                &version_id,
                &result,
                &SummaryStats::default(),
                Decimal::ZERO,
            )
            .await?;
        let fetched = repo.get_run(&id).await?.expect("saved run is fetchable");
        let trades = repo.get_trades(&id).await?;
        Ok((fetched, trades))
    }

    /// AC-7 (D8): the run-repo port double is `Send`-spawnable and consumed
    /// generically (`<R: BacktestRunRepository>`, never `dyn`). The ABSENCE of any
    /// `update_run`/`delete_run` method is demonstrated by `FakeRunRepo` having none
    /// to implement (FR-6 immutability is structural in the API).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn backtest_run_repo_port_double_is_send_spawnable() {
        // If the port's futures were not `Send`, this `spawn` would not compile.
        let handle = tokio::spawn(async { roundtrip_via(FakeRunRepo::default()).await });
        let (run, trades) = handle
            .await
            .expect("spawned task joins")
            .expect("round-trip succeeds");

        assert_eq!(run.strategy_version_id, VersionId::new("ver-1"));
        assert_eq!(run.schema_version, 1);
        assert!(trades.is_empty());
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod llm_provider_tests {
    use super::LlmProvider;
    use crate::domain::llm::{
        LlmBackend, LlmConfig, LlmError, LlmResponse, Message, TokenUsage, ToolDefinition,
    };
    use std::future::Future;

    /// An in-memory, zero-I/O fake implementing [`LlmProvider`]. It echoes the
    /// last user message back as the completion and reports fixed token usage,
    /// proving the port's future is `Send` (spawnable) and consumed generically
    /// (`<P: LlmProvider>`, never `dyn`) — mirrors `FakeSource` / `FakeRepo` /
    /// `FakeRunRepo`.
    struct FakeProvider;

    impl LlmProvider for FakeProvider {
        fn chat(
            &self,
            messages: Vec<Message>,
            _tools: &[ToolDefinition],
            _config: &LlmConfig,
        ) -> impl Future<Output = Result<LlmResponse, LlmError>> {
            let content = messages.iter().rev().find_map(|m| match m {
                Message::User { content } => Some(content.clone()),
                _ => None,
            });
            std::future::ready(Ok(LlmResponse {
                content,
                tool_calls: Vec::new(),
                usage: TokenUsage {
                    input_tokens: 7,
                    output_tokens: 2,
                },
            }))
        }
    }

    /// Generic consumption (`<P: LlmProvider>`) proves the port is used by bound,
    /// not as `dyn`, and that the returned future is `Send` (required by `spawn`).
    async fn chat_via<P: LlmProvider>(provider: P) -> Result<LlmResponse, LlmError> {
        let config = LlmConfig {
            backend: LlmBackend::Ollama,
            model: "gpt-oss:120b".to_owned(),
            temperature: 0.5,
            max_tokens: 128,
        };
        provider
            .chat(vec![Message::user("ping")], &[], &config)
            .await
    }

    /// AC-9: the [`LlmProvider`] future is `Send`-spawnable on the multi-thread
    /// runtime (mirror `fetch_future_is_send_spawnable_on_multi_thread_runtime`).
    /// If the port's future were not `Send`, this `spawn` would not compile.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn llm_provider_future_is_send() {
        let handle = tokio::spawn(async { chat_via(FakeProvider).await });
        let response = handle
            .await
            .expect("spawned task joins")
            .expect("chat succeeds");
        assert_eq!(response.content.as_deref(), Some("ping"));
        assert_eq!(response.usage.input_tokens, 7);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod llm_call_repository_tests {
    use super::LlmCallRepository;
    use crate::domain::error::DataError;
    use crate::domain::llm::{LlmBackend, Message};
    use crate::domain::llm_call::{LlmCall, LlmCallId};
    use crate::domain::strategy::CreatedBy;
    use chrono::{TimeZone, Utc};
    use rust_decimal::Decimal;
    use std::collections::HashMap;
    use std::future::Future;
    use std::sync::Mutex;

    /// An in-memory, zero-I/O fake implementing [`LlmCallRepository`]. It locks the
    /// trait shape (create + read only — no `update_call`/`delete_call` to
    /// implement, so FR-24 immutability is structural in the API) and proves the
    /// futures are `Send`-spawnable. The `Mutex`-guarded map keeps the fake
    /// `Send + Sync` so its futures are `Send` while borrowing `&self` (mirror
    /// `FakeRunRepo`).
    #[derive(Default)]
    struct FakeLlmCallRepo {
        calls: Mutex<HashMap<String, LlmCall>>,
    }

    impl LlmCallRepository for FakeLlmCallRepo {
        fn save_call(&self, call: &LlmCall) -> impl Future<Output = Result<LlmCallId, DataError>> {
            self.calls
                .lock()
                .expect("calls lock")
                .insert(call.id.as_str().to_owned(), call.clone());
            std::future::ready(Ok(call.id.clone()))
        }

        fn get_call(
            &self,
            id: &LlmCallId,
        ) -> impl Future<Output = Result<Option<LlmCall>, DataError>> {
            std::future::ready(Ok(self
                .calls
                .lock()
                .expect("calls lock")
                .get(id.as_str())
                .cloned()))
        }
    }

    fn sample_call() -> LlmCall {
        LlmCall {
            id: LlmCallId::new("call-1"),
            backend: LlmBackend::Ollama,
            model: "gpt-oss:120b".to_owned(),
            prompt_messages: vec![Message::system("be terse"), Message::user("hi")],
            completion: Some("hello".to_owned()),
            input_tokens: 7,
            output_tokens: 2,
            cost: Decimal::new(1234, 4),
            cost_currency: "CNY".to_owned(),
            created_at: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
            created_by: CreatedBy::ComposerLlm,
            key_source: None,
            prompt_version: None,
        }
    }

    /// Generic consumption (`<R: LlmCallRepository>`) proves the port is used by
    /// bound, not as `dyn`, and that the save→read futures are `Send` (required by
    /// `spawn`). Round-trips a saved call.
    async fn roundtrip_via<R: LlmCallRepository>(repo: R) -> Result<Option<LlmCall>, DataError> {
        let call = sample_call();
        let id = repo.save_call(&call).await?;
        repo.get_call(&id).await
    }

    /// The call-repo port double is `Send`-spawnable and consumed generically
    /// (`<R: LlmCallRepository>`, never `dyn`). The ABSENCE of any
    /// `update_call`/`delete_call` method is demonstrated by `FakeLlmCallRepo`
    /// having none to implement (FR-24 immutability is structural in the API).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn llm_call_repo_port_double_is_send_spawnable() {
        // If the port's futures were not `Send`, this `spawn` would not compile.
        let handle = tokio::spawn(async { roundtrip_via(FakeLlmCallRepo::default()).await });
        let fetched = handle
            .await
            .expect("spawned task joins")
            .expect("round-trip succeeds")
            .expect("saved call is fetchable");

        assert_eq!(fetched.id, LlmCallId::new("call-1"));
        assert_eq!(fetched.backend, LlmBackend::Ollama);
        assert_eq!(fetched.cost_currency, "CNY");
        assert_eq!(fetched.cost, Decimal::new(1234, 4));
    }
}
