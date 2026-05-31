//! `pulse fetch-data` orchestration (WI-1.1.1.05).
//!
//! Composes the [`MarketDataSource`] port (a [`BinanceDataSource`](crate::adapters::binance::BinanceDataSource)
//! in production) + the [`CandleStore`] into the slice's user-facing seam. The
//! orchestration depends **only** on the port + the store (NFR-9 / AC-6) — it
//! never names the concrete adapter type.
//!
//! Per-(pair, tf) flow (grill + audit-locked, spec §3):
//! - **First run** (no `HEAD`): bulk over the `--years N` window
//!   ([`MarketDataSource::fetch_historical`]) **then** an immediate REST top-up
//!   to the clock cutoff ([`MarketDataSource::fetch_incremental`]) so the first
//!   snapshot is current. Write the snapshot, then set `HEAD`. Action `bulk`.
//! - **Subsequent run** (`HEAD` present): read the prior snapshot, top up only
//!   newly-closed candles. If any closed → write a new version + move `HEAD`
//!   (action `incremental`); if nothing newly closed → **`up-to-date` no-op**,
//!   not an error.
//! - **Ordering + crash-safety (audit C1):** the snapshot Parquet is written
//!   **first** (atomic, WI-04), `HEAD` **second** (atomic). A crash between
//!   leaves a valid orphaned snapshot and an unchanged `HEAD`.
//! - **`--years N` window (audit C5):** start = floor to the first day of the
//!   month `N` years before the current UTC month.
//! - **Multi-tf (audit C4):** each tf is fetched independently; a failing tf is
//!   reported in its summary and the process exits non-zero, while successful
//!   tfs remain.

use chrono::{Datelike, TimeZone, Utc};
use serde::Serialize;

use crate::adapters::store::CandleStore;
use crate::domain::{Clock, DataError, MarketDataSource, Pair, Timeframe};

/// The action taken for one `(pair, tf)` this run (the `--json` `action` field).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Action {
    /// First run: bulk window + immediate top-up to now.
    Bulk,
    /// Subsequent run: newly-closed candles topped up.
    Incremental,
    /// Subsequent run with nothing newly closed — a no-op, not an error.
    UpToDate,
}

impl Action {
    /// The stable string form used in human output + the `--json` schema.
    fn as_str(self) -> &'static str {
        match self {
            Action::Bulk => "bulk",
            Action::Incremental => "incremental",
            Action::UpToDate => "up-to-date",
        }
    }
}

/// The grill-locked per-timeframe `--json` summary object.
///
/// Schema (spec §3, grill-locked): `{pair, timeframe, data_version, action,
/// candle_count, first_open_ms, last_open_ms, path, gap_count}`. Stable field
/// names + order so downstream tooling can depend on it.
#[derive(Debug, Clone, Serialize)]
pub struct TfSummary {
    /// Trading pair symbol.
    pub pair: String,
    /// `Binance` interval string (`15m` / `4h`).
    pub timeframe: String,
    /// The snapshot's content-hash `data_version` (the new HEAD).
    pub data_version: String,
    /// One of `bulk` / `incremental` / `up-to-date`.
    pub action: String,
    /// Number of candles in the snapshot.
    pub candle_count: usize,
    /// `open_time` of the first candle, if any.
    pub first_open_ms: Option<i64>,
    /// `open_time` of the last candle, if any.
    pub last_open_ms: Option<i64>,
    /// Absolute snapshot path.
    pub path: String,
    /// Number of detected spacing gaps (reported, not rejected — audit C2).
    pub gap_count: usize,
}

/// The outcome of one tf's orchestration: a summary on success, or the error on
/// failure (AC-8 — a failing tf still produces a `--json` entry).
pub enum TfOutcome {
    /// The tf's snapshot was ensured (written or already current).
    Ok(TfSummary),
    /// The tf failed; `summary` carries the partial entry (action + error) for
    /// the `--json` report and the process exits non-zero.
    Failed {
        /// The timeframe that failed.
        timeframe: String,
        /// The error message surfaced in the report.
        error: String,
    },
}

/// Compute the bulk window start (epoch ms) for `--years N`: floor to the first
/// day of the month `n_years` before the current UTC month (audit C5).
///
/// `now_ms` is the [`Clock`]'s "now" so the window is deterministic in tests.
#[must_use]
pub fn years_window_start_ms(now_ms: i64, n_years: u32) -> i64 {
    let now = Utc
        .timestamp_millis_opt(now_ms)
        .single()
        .unwrap_or_else(Utc::now);
    let target_year = now.year() - i32::try_from(n_years).unwrap_or(i32::MAX);
    // Floor to the first millisecond of the first day of that month, UTC.
    Utc.with_ymd_and_hms(target_year, now.month(), 1, 0, 0, 0)
        .single()
        .map_or(now_ms, |dt| dt.timestamp_millis())
}

/// Ensure the snapshot for one `(pair, tf)`, returning a summary (or a failure
/// entry on error — never panics; the caller aggregates exit status, AC-8).
///
/// `now_ms` is read once from the clock so the window + cutoff are deterministic.
pub async fn ensure_one_tf<S, C>(
    source: &S,
    store: &CandleStore,
    clock: &C,
    pair: &Pair,
    tf: Timeframe,
    n_years: u32,
) -> TfOutcome
where
    S: MarketDataSource,
    C: Clock,
{
    let now_ms = clock.now_ms();
    match ensure_inner(source, store, pair, tf, n_years, now_ms).await {
        Ok(summary) => TfOutcome::Ok(summary),
        Err(e) => TfOutcome::Failed {
            timeframe: tf.binance_interval().to_string(),
            error: e.to_string(),
        },
    }
}

/// The fallible body of [`ensure_one_tf`] (kept ≤ 80 lines; helpers below).
async fn ensure_inner<S>(
    source: &S,
    store: &CandleStore,
    pair: &Pair,
    tf: Timeframe,
    n_years: u32,
    now_ms: i64,
) -> Result<TfSummary, DataError>
where
    S: MarketDataSource,
{
    match store.read_head(pair, tf)? {
        None => first_run(source, store, pair, tf, n_years, now_ms).await,
        Some(prior_version) => subsequent_run(source, store, pair, tf, &prior_version).await,
    }
}

/// First run: bulk over the years window, then top up to now; write snapshot,
/// then `HEAD` (audit C1).
async fn first_run<S>(
    source: &S,
    store: &CandleStore,
    pair: &Pair,
    tf: Timeframe,
    n_years: u32,
    now_ms: i64,
) -> Result<TfSummary, DataError>
where
    S: MarketDataSource,
{
    let start_ms = years_window_start_ms(now_ms, n_years);
    // Bulk covers COMPLETE months only — exclude the current (incomplete) month,
    // which data.binance.vision has not published a monthly archive for yet; the
    // REST top-up below fills it (audit C5). `years_window_start_ms(_, 0)` floors
    // `now` to the first day of the current UTC month. Passing `now_ms` here (the
    // original bug) made the bulk range include the current month → WI-02's
    // "expected month absent after listing" error on the live `--years 2` run.
    let bulk_end_ms = years_window_start_ms(now_ms, 0);
    let mut series = source
        .fetch_historical(pair, tf, start_ms, bulk_end_ms)
        .await?;
    // Immediate top-up to "now" (closed candles only) so the first snapshot is
    // current (grill). Empty bulk ⇒ since = -1 so a top-up still starts at 0.
    let since = series.candles.last().map_or(-1, |c| c.open_time);
    let new = source.fetch_incremental(pair, tf, since).await?;
    if !new.is_empty() {
        series = crate::adapters::binance::merge::merge_new(&series, new)?.0;
    }
    persist(store, pair, tf, series, Action::Bulk)
}

/// Subsequent run: read the prior snapshot, top up only newly-closed candles.
async fn subsequent_run<S>(
    source: &S,
    store: &CandleStore,
    pair: &Pair,
    tf: Timeframe,
    prior_version: &crate::domain::DataVersion,
) -> Result<TfSummary, DataError>
where
    S: MarketDataSource,
{
    let prior = store.read_snapshot(pair, tf, prior_version)?;
    let since = prior.candles.last().map_or(-1, |c| c.open_time);
    let new = source.fetch_incremental(pair, tf, since).await?;
    if new.is_empty() {
        // Nothing newly closed ⇒ up-to-date no-op (NOT an error). HEAD unchanged.
        return summarize(store, &prior, Action::UpToDate);
    }
    let (merged, _gaps) = crate::adapters::binance::merge::merge_new(&prior, new)?;
    persist(store, pair, tf, merged, Action::Incremental)
}

/// Write the snapshot (FIRST) then move `HEAD` (SECOND) atomically (audit C1),
/// re-deriving the content-hash `data_version` for the merged candle set.
fn persist(
    store: &CandleStore,
    pair: &Pair,
    tf: Timeframe,
    mut series: crate::domain::CandleSeries,
    action: Action,
) -> Result<TfSummary, DataError> {
    series.version = CandleStore::content_version(pair, tf, &series.candles);
    // Snapshot FIRST (atomic temp→rename, WI-04).
    store.write_snapshot(&series)?;
    // HEAD SECOND (atomic temp→rename). A crash between leaves a valid orphan.
    store.write_head(pair, tf, &series.version)?;
    summarize(store, &series, action)
}

/// Build the `--json`/human summary for a (persisted or already-current) series.
fn summarize(
    store: &CandleStore,
    series: &crate::domain::CandleSeries,
    action: Action,
) -> Result<TfSummary, DataError> {
    let gaps = series.validate()?;
    let path = store.snapshot_path(&series.pair, series.timeframe, &series.version);
    Ok(TfSummary {
        pair: series.pair.to_string(),
        timeframe: series.timeframe.binance_interval().to_string(),
        data_version: series.version.to_string(),
        action: action.as_str().to_string(),
        candle_count: series.candles.len(),
        first_open_ms: series.candles.first().map(|c| c.open_time),
        last_open_ms: series.candles.last().map(|c| c.open_time),
        path: path.display().to_string(),
        gap_count: gaps.len(),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{Action, TfSummary, years_window_start_ms};
    use chrono::{Datelike, TimeZone, Timelike, Utc};

    // ---- C5: --years N floors to the start of the month N years back, UTC --

    #[test]
    fn years_window_floors_to_start_of_month_n_years_back() {
        // now = 2026-05-31T13:00:00Z, N = 2 → 2024-05-01T00:00:00Z.
        let now = Utc.with_ymd_and_hms(2026, 5, 31, 13, 0, 0).unwrap();
        let start_ms = years_window_start_ms(now.timestamp_millis(), 2);
        let start = Utc.timestamp_millis_opt(start_ms).single().unwrap();
        assert_eq!(start.year(), 2024);
        assert_eq!(start.month(), 5);
        assert_eq!(start.day(), 1);
        assert_eq!((start.hour(), start.minute(), start.second()), (0, 0, 0));
    }

    #[test]
    fn years_window_one_year_back() {
        let now = Utc.with_ymd_and_hms(2026, 1, 15, 9, 30, 0).unwrap();
        let start_ms = years_window_start_ms(now.timestamp_millis(), 1);
        let start = Utc.timestamp_millis_opt(start_ms).single().unwrap();
        assert_eq!((start.year(), start.month(), start.day()), (2025, 1, 1));
    }

    // ---- AC-4: the --json summary serializes with the locked schema --------

    #[test]
    fn tf_summary_serializes_with_the_locked_schema() {
        let summary = TfSummary {
            pair: "BTCUSDT".to_string(),
            timeframe: "15m".to_string(),
            data_version: "deadbeefcafef00d".to_string(),
            action: "bulk".to_string(),
            candle_count: 3,
            first_open_ms: Some(0),
            last_open_ms: Some(1_800_000),
            path: "/tmp/candles/BTCUSDT/15m/deadbeefcafef00d.parquet".to_string(),
            gap_count: 0,
        };
        let json = serde_json::to_value(&summary).expect("serialize");
        // Every grill-locked field is present under its exact name.
        for key in [
            "pair",
            "timeframe",
            "data_version",
            "action",
            "candle_count",
            "first_open_ms",
            "last_open_ms",
            "path",
            "gap_count",
        ] {
            assert!(json.get(key).is_some(), "missing field {key}: {json}");
        }
        assert_eq!(json["candle_count"], 3);
        assert_eq!(json["action"], "bulk");
    }

    #[test]
    fn action_strings_are_stable_kebab_case() {
        assert_eq!(Action::Bulk.as_str(), "bulk");
        assert_eq!(Action::Incremental.as_str(), "incremental");
        assert_eq!(Action::UpToDate.as_str(), "up-to-date");
    }
}
