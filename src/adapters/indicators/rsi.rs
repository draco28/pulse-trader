//! RSI adapter — wraps ta-rs `RelativeStrengthIndex` behind the domain
//! [`Indicator`] port over `candle.close`.
//!
//! **Smoothing convention (load-bearing for 3.04 cross-validation).** ta-rs RSI
//! smooths the up/down moves with a **standard EMA (`α = 2/(period + 1)`)** — it
//! is *Cutler's* RSI, **not** Wilder's RMA-smoothed RSI. So any external
//! reference (3.04's pandas-ta) MUST be pinned to `mamode="ema"`; the pandas-ta
//! default (`rma`/Wilder) would mismatch this adapter.
//!
//! **Warmup convention.** ta-rs RSI is *seeded*: it emits a value from its very
//! first input (the first delta is seeded, so candle 1 yields ~50). Our port
//! convention, mirroring 3.01's EMA, returns `None` for the first `period`
//! candles — RSI(p) needs `p` price deltas, so the first genuinely-defined value
//! lands on candle `period + 1`, aligning with pandas-ta's first non-NaN RSI
//! row (the alignment 3.04 depends on). We therefore feed *every* candle to the
//! underlying ta-rs RSI (warming its recursive EMA state) but gate emission on a
//! candle counter. The warmup count is pinned by an AC test.

use crate::adapters::indicators::convert::{decimal_to_f64, f64_to_decimal_rounded};
use crate::domain::{Candle, Indicator};
use rust_decimal::Decimal;
use ta::Next;
use ta::indicators::RelativeStrengthIndex;

/// RSI(period) over closing prices, wrapping ta-rs behind the [`Indicator`]
/// port. Output is the RSI in `[0, 100]`, rounded to scale-8.
pub struct Rsi {
    inner: RelativeStrengthIndex,
    period: u32,
    /// Number of candles fed so far.
    seen: u32,
}

impl Rsi {
    /// Build an RSI over `period` candles. `period` must be ≥ 1.
    ///
    /// Returns `None` if `period` is 0 (ta-rs rejects a zero period). Constructed
    /// from a concrete `u32` (the `Fixed`-extraction factory that resolves
    /// `SweepableValue` periods is 3.03's concern). Panic-free: ta-rs's
    /// constructor error maps to `None`.
    #[must_use]
    pub fn new(period: u32) -> Option<Self> {
        let inner = RelativeStrengthIndex::new(period as usize).ok()?;
        Some(Self {
            inner,
            period,
            seen: 0,
        })
    }
}

impl Indicator for Rsi {
    fn next(&mut self, candle: &Candle) -> Option<Decimal> {
        // Convert BEFORE advancing the warmup counter: a non-representable price
        // (`None`) must not desync `seen`/readiness from the inner ta-rs EMA
        // state (a phase shift in the determinism layer). Feed every valid candle
        // so the recursive state warms even while output is suppressed.
        let input = decimal_to_f64(candle.close)?;
        self.seen = self.seen.saturating_add(1);
        let out = self.inner.next(input);

        // Warmup: suppress the first `period` candles; emit from candle
        // `period + 1` onward (RSI(p) needs `p` deltas → pandas-ta alignment).
        if self.seen <= self.period {
            return None;
        }
        f64_to_decimal_rounded(out)
    }

    fn is_ready(&self) -> bool {
        // ready iff the NEXT call (the `seen + 1`-th candle) is candle
        // `period + 1` or beyond.
        self.seen.saturating_add(1) > self.period
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::Rsi;
    use crate::domain::{Candle, Indicator};
    use rust_decimal::Decimal;
    use std::str::FromStr;

    fn candle_close(close: &str) -> Candle {
        let c = Decimal::from_str(close).unwrap();
        Candle {
            open_time: 0,
            close_time: 0,
            open: c,
            high: c,
            low: c,
            close: c,
            volume: Decimal::ONE,
            funding_rate: None,
        }
    }

    /// Hand-computed Cutler RSI reference (matching ta-rs): up/down moves smoothed
    /// with a **seeded standard EMA** (`α = 2/(p+1)`), seeded with the up=down=0.1
    /// guard on candle 1, exactly as ta-rs does internally. Returns the full
    /// emitted-or-None sequence under our port's warmup gating (first `period`
    /// candles → `None`).
    fn rsi_reference(closes: &[f64], period: u32) -> Vec<Option<f64>> {
        let k = 2.0 / (f64::from(period) + 1.0);
        // Seeded EMA: first output is its first input.
        let mut up_ema: Option<f64> = None;
        let mut down_ema: Option<f64> = None;
        let mut prev = 0.0_f64;
        let mut is_new = true;
        let mut out = Vec::with_capacity(closes.len());
        let mut candle_no: u32 = 0;

        for &c in closes {
            candle_no += 1;
            let (up, down) = if is_new {
                is_new = false;
                (0.1, 0.1) // ta-rs's division-by-zero seed
            } else if c > prev {
                (c - prev, 0.0)
            } else {
                (0.0, prev - c)
            };
            prev = c;

            let ue = match up_ema {
                None => up,
                Some(p) => k * up + (1.0 - k) * p,
            };
            let de = match down_ema {
                None => down,
                Some(p) => k * down + (1.0 - k) * p,
            };
            up_ema = Some(ue);
            down_ema = Some(de);
            let rsi = 100.0 * ue / (ue + de);

            // Port warmup gating: first `period` candles suppressed.
            if candle_no <= period {
                out.push(None);
            } else {
                out.push(Some(rsi));
            }
        }
        out
    }

    #[test]
    fn rsi_returns_none_during_warmup_then_some() {
        let period = 5u32;
        let mut rsi = Rsi::new(period).expect("period >= 1");

        // First `period` candles: warmup → None. Use a varied series so the
        // suppression isn't masked by a degenerate constant input.
        let warmup = ["100", "101", "100.5", "102", "101.25"];
        assert_eq!(u32::try_from(warmup.len()).unwrap(), period);
        let mut candle_no: u32 = 0;
        for c in warmup {
            candle_no += 1;
            let out = rsi.next(&candle_close(c));
            assert_eq!(out, None, "candle {candle_no} is warmup → None");
            assert!(
                !rsi.is_ready() || candle_no == period,
                "not ready until the warmup count is reached"
            );
        }
        // Having fed exactly `period` candles, the NEXT call is candle
        // `period + 1` → first defined RSI.
        assert!(rsi.is_ready(), "ready right before candle period + 1");

        let out = rsi.next(&candle_close("103"));
        assert!(out.is_some(), "candle period + 1 → Some");
        assert!(rsi.is_ready(), "stays ready once warm");
    }

    #[test]
    fn rsi_matches_reference_within_epsilon() {
        let period = 3u32;
        let closes = [10.0_f64, 10.5, 10.0, 9.5, 11.0, 10.75, 12.0];
        let reference = rsi_reference(&closes, period);

        let mut rsi = Rsi::new(period).expect("period >= 1");
        for (i, &c) in closes.iter().enumerate() {
            let out = rsi.next(&candle_close(&c.to_string()));
            match reference[i] {
                None => assert_eq!(out, None, "warmup candle {} → None", i + 1),
                Some(expected) => {
                    let got: f64 = out.expect("warm → Some").to_string().parse().unwrap();
                    assert!(
                        (got - expected).abs() < 1e-6,
                        "candle {}: got {got}, expected {expected}",
                        i + 1
                    );
                }
            }
        }
    }
}
