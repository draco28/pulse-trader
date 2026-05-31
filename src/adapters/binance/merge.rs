//! Idempotent merge of incrementally-fetched candles onto a prior snapshot
//! (WI-1.1.1.03, AC-2/AC-3).
//!
//! The incremental fetch ([`super::incremental`]) returns only the **new**
//! candles (audit C1); this module appends them to the prior series, dedups on
//! `open_time` (keeping the freshest copy), re-runs WI-02's deterministic
//! [`normalize`](super::normalize) (stable sort + dedup + structural
//! re-validation), and hands back the **full merged [`CandleSeries`]** plus its
//! reported gaps.
//!
//! Immutability (audit C1): the returned series is a *new* value; persisting it
//! mints a new `data_version` (a fresh content hash via WI-04). Nothing here
//! mutates the prior snapshot or appends in place to an existing Parquet — the
//! old (shorter) snapshot stays on disk.
//!
//! Idempotency (AC-3): merging with zero new candles, or with candles that
//! already exist at the same `open_time`s, leaves the candle vector unchanged.
//! Combined with the content-hash version, a second top-up that discovers no
//! newly-closed data produces a byte-identical series → the same `data_version`.

use crate::domain::{Candle, CandleSeries, DataError, Gap};

use super::normalize::normalize;

/// Merge `new_candles` onto `prior`, returning the full merged series + gaps.
///
/// Reuses WI-02's [`normalize`]: concatenate prior + new, stable-sort ascending
/// by `open_time`, dedup on `open_time` keeping the *last* occurrence (a re-fetched
/// candle overwrites the stored copy), then structurally re-validate. The
/// `(pair, timeframe)` identity is inherited from `prior`; the version tag is
/// carried forward unchanged here — the caller (WI-04/05) re-derives the
/// content-hash `data_version` at persist time (audit C1), so this fn does not
/// invent a version.
///
/// # Errors
///
/// Propagates [`DataError::Validation`] from [`CandleSeries::validate`] (via
/// `normalize`). Post-normalize the merged series is sorted and dup-free, so a
/// structural error is not expected; the call enforces the contract rather than
/// assuming it.
pub(crate) fn merge_new(
    prior: &CandleSeries,
    new_candles: Vec<Candle>,
) -> Result<(CandleSeries, Vec<Gap>), DataError> {
    let mut all: Vec<Candle> = Vec::with_capacity(prior.candles.len() + new_candles.len());
    all.extend_from_slice(&prior.candles);
    all.extend(new_candles);

    let (mut series, gaps) = normalize(&prior.pair, prior.timeframe, &prior.version, all)?;
    // Identity carried from the prior snapshot; the persist layer re-derives the
    // content-hash version (audit C1) — never an in-place append.
    series.pair = prior.pair.clone();
    series.timeframe = prior.timeframe;
    Ok((series, gaps))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::merge_new;
    use crate::domain::{Candle, CandleSeries, DataVersion, Pair, Timeframe};
    use rust_decimal::Decimal;

    const M15: i64 = 900_000;

    fn candle(open_time: i64, close: &str) -> Candle {
        Candle {
            open_time,
            close_time: open_time + M15 - 1,
            open: Decimal::ONE,
            high: Decimal::ONE,
            low: Decimal::ONE,
            close: close.parse().unwrap(),
            volume: Decimal::ONE,
            funding_rate: None,
        }
    }

    fn series(open_times: &[i64]) -> CandleSeries {
        CandleSeries {
            pair: Pair::new("BTCUSDT"),
            timeframe: Timeframe::M15,
            version: DataVersion::new("v1"),
            candles: open_times.iter().map(|&t| candle(t, "1")).collect(),
        }
    }

    // ---- AC-2: append + dedup + re-validate -------------------------------

    #[test]
    fn appends_new_candles_and_revalidates_ok() {
        let prior = series(&[0, M15]);
        let new = vec![candle(2 * M15, "2"), candle(3 * M15, "3")];
        let (merged, gaps) = merge_new(&prior, new).expect("merge Ok");
        let times: Vec<i64> = merged.candles.iter().map(|c| c.open_time).collect();
        assert_eq!(times, vec![0, M15, 2 * M15, 3 * M15]);
        assert!(gaps.is_empty(), "contiguous merge: {gaps:?}");
    }

    #[test]
    fn merge_reports_gap_without_filling() {
        let prior = series(&[0, M15]);
        // Jump straight to 3*M15: the 2*M15 bar is missing.
        let new = vec![candle(3 * M15, "3")];
        let (merged, gaps) = merge_new(&prior, new).expect("merge Ok");
        assert_eq!(merged.candles.len(), 3, "no fill: hole stays a hole");
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].expected, 2 * M15);
        assert_eq!(gaps[0].found, 3 * M15);
    }

    #[test]
    fn overlapping_open_time_dedups_keeping_new() {
        let prior = series(&[0, M15]);
        // A re-fetched candle at M15 with a different close must overwrite.
        let new = vec![candle(M15, "9"), candle(2 * M15, "2")];
        let (merged, _) = merge_new(&prior, new).expect("merge Ok");
        assert_eq!(merged.candles.len(), 3);
        assert_eq!(
            merged.candles[1].close,
            "9".parse::<Decimal>().unwrap(),
            "the freshly-fetched candle wins the dedup"
        );
    }

    // ---- AC-3: idempotency -------------------------------------------------

    #[test]
    fn empty_top_up_is_byte_identical() {
        let prior = series(&[0, M15, 2 * M15]);
        let (merged, gaps) = merge_new(&prior, Vec::new()).expect("merge Ok");
        assert_eq!(merged, prior, "zero new candles ⇒ identical series");
        assert!(gaps.is_empty());
    }

    #[test]
    fn re_merging_existing_candles_is_idempotent() {
        let prior = series(&[0, M15, 2 * M15]);
        // Re-supply candles that already exist (a second run before any new
        // candle closed): the merged series must equal the prior exactly.
        let dup = vec![candle(M15, "1"), candle(2 * M15, "1")];
        let (merged, _) = merge_new(&prior, dup).expect("merge Ok");
        assert_eq!(merged, prior, "re-merging existing data is a no-op");
    }

    #[test]
    fn merge_preserves_pair_and_timeframe_identity() {
        let prior = series(&[0]);
        let (merged, _) = merge_new(&prior, vec![candle(M15, "2")]).expect("merge Ok");
        assert_eq!(merged.pair, prior.pair);
        assert_eq!(merged.timeframe, prior.timeframe);
    }
}
