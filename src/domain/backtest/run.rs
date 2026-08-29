//! Persisted backtest-run domain types (VS-1.2.4 work-4.04, FR-6 / FR-7 / NFR-2).
//!
//! The read-back projections of a persisted backtest run: [`BacktestRunId`] (the
//! opaque run id), [`PersistedRun`] (the run header + its [`SummaryStats`]
//! projection + the integrity / cohort-key fields), and [`RunSummary`] (the typed
//! list projection for the run catalog — explicit fields, NOT a serialized
//! [`BacktestResult`](super::BacktestResult) blob).
//!
//! **Typed projection, never a blob (D1, #68 — README C5).** Both projections
//! carry EXPLICIT typed fields mirrored one-to-one onto the `backtest_run` columns
//! (README C4). Nothing here round-trips a serde-serialized `BacktestResult`, so
//! read-back is independent of serde field-presence and #17 stays a track-forward
//! DSL-grammar concern (no silent-field-drop surface reintroduced).
//!
//! **`engine_target` cohort key (#49, D6).** [`PersistedRun::engine_target`] +
//! [`RunSummary::engine_target`] persist the engine's compiled target triple as
//! the regime cohort key. The per-trade `regime` (persisted on the `trade` row) is
//! **deterministic on the v1 pinned toolchain, NOT byte-portable** (inherits the
//! deferred #29 caveat): it is threshold-on-`f64`-EMA/ADX derived, so two
//! architectures may classify a borderline bar differently. The richer regime
//! provenance (threshold flag + raw indicator snapshot) is DEFERRED to #49.
//!
//! These types carry **`PartialEq` but not `Eq`** (R2 carry-forward): the embedded
//! [`SummaryStats`] holds `sharpe`/`sortino` `f64` fields, so `Eq` is not
//! derivable transitively. Use `PartialEq` only.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::regime::RegimeBreakdown;
use super::stats::SummaryStats;
use crate::domain::sizing::SkippedEntryCounts;
use crate::domain::strategy::VersionId;

/// Identifier of a persisted [`PersistedRun`] — a `#[serde(transparent)]`
/// `String` newtype (mirror [`StrategyId`](crate::domain::strategy::StrategyId) /
/// [`VersionId`]).
///
/// Holds a UUID-hyphenated value the adapter
/// ([`SqliteBacktestRunRepo`](crate::adapters::db::SqliteBacktestRunRepo)) mints;
/// serializes as a bare JSON string matching the `backtest_run.id` `TEXT` primary
/// key.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BacktestRunId(String);

impl BacktestRunId {
    /// Wrap a raw (adapter-generated) id string.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Borrow the underlying id string (for SQL binding / map keys).
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The read-back header of one persisted backtest run (README C4 / C6).
///
/// The typed projection a `get_run` / `latest_run_for_version` read reconstructs:
/// the run identity + provenance (`strategy_version_id`, `created_at`,
/// `engine_fingerprint`, `engine_target`, `schema_version`), the integrity field
/// (`result_content_hash`, re-derived-and-checked on read per D4), the run-level
/// money totals, the equity-curve base (`starting_equity`), and the derived
/// [`SummaryStats`] projection. **Not** a `BacktestResult` blob (D1).
///
/// `PartialEq` (not `Eq`): the embedded [`SummaryStats`] carries `f64` fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistedRun {
    /// The run's opaque id.
    pub id: BacktestRunId,
    /// The `strategy_version` this run was produced against (FK).
    pub strategy_version_id: VersionId,
    /// The run-row schema tag (#68 / D1b) — `RUN_SCHEMA_VERSION` for a v1 row.
    pub schema_version: i64,
    /// The injected-Clock run timestamp (RFC3339 UTC ms text on the column; D7).
    pub created_at: String,
    /// The recording engine's build-time fingerprint (FR-7 prior-run key).
    pub engine_fingerprint: String,
    /// The recording engine's compiled target triple (#49 cohort key; D6).
    pub engine_target: String,
    /// The integrity hash re-derived from the read-back run totals + regime
    /// breakdown + skipped counts + the `seq`-ordered trades, and checked against
    /// the stored column on read (D4 tamper guard).
    pub result_content_hash: String,
    /// The equity-curve base the read-side `EquityCurve::from_trades` reuses (C2).
    pub starting_equity: Decimal,
    /// Net P&L across the run (the engine total — README C4).
    pub net_pnl: Decimal,
    /// Total taker fees across the run.
    pub fees_total: Decimal,
    /// Total signed funding P&L across the run.
    pub funding_total: Decimal,
    /// Total adverse slippage cost across the run.
    pub slippage_total: Decimal,
    /// The derived read-only summary projection (expectancy / win rate / profit
    /// factor / Sharpe / drawdown / streaks / totals — all C4 stat columns).
    pub summary: SummaryStats,
    /// The per-regime trade-count / net-P&L breakdown, as persisted.
    ///
    /// r1.s2.w3: a **pass-through of a stored value**, not a recomputation.
    /// `get_run` already decoded this column to re-derive `result_content_hash`;
    /// it now also surfaces it, because the coach's bounded `CoachContext`
    /// (ADR-0021 decision 8) reads it and the money-math control says the coach
    /// must read persisted results rather than recompute them. Nothing about the
    /// hash input, its feed order, or the query changed.
    pub regime_breakdown: RegimeBreakdown,
    /// The counts of entries the sizer skipped, as persisted. Same pass-through
    /// reasoning as [`regime_breakdown`](Self::regime_breakdown) — and unlike the
    /// regime split, these are **not** derivable from the trade log at all: they
    /// count entries that never became trades.
    pub skipped_entries: SkippedEntryCounts,
}

/// The typed list projection of one run for the catalog
/// ([`list_runs_for_version`](crate::domain::port::BacktestRunRepository::list_runs_for_version)).
///
/// Explicit typed fields per README C4 — NOT a [`BacktestResult`](super::BacktestResult)
/// blob (D1). A best-effort catalog row: the headline identity + provenance + the
/// two headline stats most useful in a run list (`net_pnl`, `expectancy`). The
/// full [`SummaryStats`] + trade log are fetched per-run via `get_run` /
/// `get_trades`.
///
/// `PartialEq` (not `Eq`): `expectancy` rides a `Decimal` (Eq-able), but the type
/// is kept consistent with [`PersistedRun`]'s non-`Eq` discipline; only `PartialEq`
/// is derived.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunSummary {
    /// The run's opaque id.
    pub id: BacktestRunId,
    /// The `strategy_version` this run was produced against (FK).
    pub strategy_version_id: VersionId,
    /// The run-row schema tag (#68 / D1b).
    pub schema_version: i64,
    /// The injected-Clock run timestamp (RFC3339 UTC ms text; D7).
    pub created_at: String,
    /// The recording engine's build-time fingerprint (FR-7).
    pub engine_fingerprint: String,
    /// The recording engine's compiled target triple (#49 cohort key; D6).
    pub engine_target: String,
    /// The integrity hash stored on the run row (catalog displays it; the
    /// trade-dependent re-derive is a `get_run` concern, D4).
    pub result_content_hash: String,
    /// Net P&L across the run (headline catalog stat).
    pub net_pnl: Decimal,
    /// Mean P&L per trade (headline catalog stat).
    pub expectancy: Decimal,
    /// Number of completed trades in the run.
    pub trade_count: usize,
}
