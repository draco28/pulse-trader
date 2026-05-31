//! Deterministic normalization (NFR-2): stable-sort ascending by `open_time`,
//! dedup on `open_time` (keep last), then assemble a [`CandleSeries`] and run
//! its structural validation.
//!
//! Gap policy (spec §3, audit C2): [`CandleSeries::validate`] returns
//! `Ok(Vec<Gap>)` for a structurally-sound but gapped series — gaps are
//! *reported* through `Ok`, never filled and never raised. Only structural
//! corruption (a duplicate that survives dedup is impossible; an out-of-order
//! pair after sorting is impossible) would surface as `Err`, so the normalized
//! series validates `Ok` and the gap list is handed back to the caller.

use crate::domain::{Candle, CandleSeries, DataError, DataVersion, Gap, Pair, Timeframe};

/// Normalize raw candles into a validated [`CandleSeries`] plus its reported
/// gaps (AC-3).
///
/// Steps, in order (NFR-2 determinism):
/// 1. **Stable sort** ascending by `open_time` (ties keep input order so the
///    subsequent dedup's "keep last" is well-defined).
/// 2. **Dedup** on `open_time`, keeping the *last* occurrence (a re-downloaded
///    overlapping month overwrites the earlier copy).
/// 3. Assemble the `(pair, tf, version)` series and call
///    [`CandleSeries::validate`]: a structurally-sound series returns
///    `Ok(gaps)` — the gaps are returned alongside the series for the caller to
///    log; ingest does **not** fill them.
///
/// # Errors
///
/// Propagates [`DataError::Validation`] from [`CandleSeries::validate`]. After
/// sort+dedup this should not occur, but the call is made (not assumed) so the
/// determinism contract is enforced rather than trusted.
pub(crate) fn normalize(
    pair: &Pair,
    tf: Timeframe,
    version: &DataVersion,
    mut candles: Vec<Candle>,
) -> Result<(CandleSeries, Vec<Gap>), DataError> {
    // (1) Stable sort by open_time.
    candles.sort_by_key(|c| c.open_time);

    // (2) Dedup on open_time, keeping the last occurrence. `dedup_by_key`
    // retains the *first* of a run, so reverse → dedup → reverse to keep last.
    candles.reverse();
    candles.dedup_by_key(|c| c.open_time);
    candles.reverse();

    let series = CandleSeries {
        pair: pair.clone(),
        timeframe: tf,
        version: version.clone(),
        candles,
    };

    // (3) Report gaps via Ok; structural corruption (impossible post-normalize)
    // would surface as Err and is propagated rather than swallowed.
    let gaps = series.validate()?;
    Ok((series, gaps))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::normalize;
    use crate::domain::{Candle, CandleSeries, DataVersion, Gap, Pair, Timeframe};
    use rust_decimal::Decimal;

    fn candle_at(open_time: i64, close: &str) -> Candle {
        Candle {
            open_time,
            close_time: open_time + 899_999,
            open: Decimal::ONE,
            high: Decimal::ONE,
            low: Decimal::ONE,
            close: close.parse().unwrap(),
            volume: Decimal::ONE,
            funding_rate: None,
        }
    }

    fn run(candles: Vec<Candle>) -> (CandleSeries, Vec<Gap>) {
        normalize(
            &Pair::new("BTCUSDT"),
            Timeframe::M15,
            &DataVersion::new("v1"),
            candles,
        )
        .expect("normalize Ok")
    }

    #[test]
    fn unsorted_input_sorts_ascending() {
        let (series, gaps) = run(vec![
            candle_at(1_800_000, "3"),
            candle_at(0, "1"),
            candle_at(900_000, "2"),
        ]);
        let times: Vec<i64> = series.candles.iter().map(|c| c.open_time).collect();
        assert_eq!(times, vec![0, 900_000, 1_800_000]);
        assert!(gaps.is_empty(), "contiguous after sort: {gaps:?}");
    }

    #[test]
    fn duplicate_open_time_dedups_keeping_last() {
        // Two candles at open_time 0; the *last* (close "9") must win.
        let (series, _) = run(vec![
            candle_at(0, "1"),
            candle_at(0, "9"),
            candle_at(900_000, "2"),
        ]);
        assert_eq!(series.candles.len(), 2);
        assert_eq!(series.candles[0].open_time, 0);
        assert_eq!(series.candles[0].close, "9".parse::<Decimal>().unwrap());
    }

    #[test]
    fn gapped_series_reports_gap_through_ok_and_does_not_fill() {
        // Missing the 900_000 bar.
        let (series, gaps) = run(vec![
            candle_at(0, "1"),
            candle_at(1_800_000, "3"),
            candle_at(2_700_000, "4"),
        ]);
        // No fill: the hole stays a hole, only reported.
        assert_eq!(series.candles.len(), 3);
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].expected, 900_000);
        assert_eq!(gaps[0].found, 1_800_000);
    }

    #[test]
    fn clean_series_reports_no_gaps() {
        let (_, gaps) = run(vec![
            candle_at(0, "1"),
            candle_at(900_000, "2"),
            candle_at(1_800_000, "3"),
        ]);
        assert!(gaps.is_empty());
    }
}
