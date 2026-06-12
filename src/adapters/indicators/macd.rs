//! MACD adapter — wraps ta-rs `MovingAverageConvergenceDivergence` behind the
//! domain [`Indicator`] port over `candle.close`.
//!
//! **Multi-output convention (resolves #18).** ta-rs emits a 3-field output
//! (MACD line, signal line, histogram). `IndicatorSpec::Macd` carries **no**
//! output selector, so the **v1 default** is the **MACD line**
//! (`macd = EMA(fast) − EMA(slow)`) — this adapter returns exactly that field.
//! Selecting the signal line or histogram is a *future* additive DSL schema bump
//! (`IndicatorSpec::Macd { …, output: MacdOutput }`); it is **not** implemented
//! here and is filed as a tracked issue at round close.
//!
//! **Warmup convention.** Like ta-rs's other indicators, the underlying EMAs are
//! *seeded* and emit from candle 1. This adapter emits the MACD line, not the
//! signal line, so the port suppresses output until the slow EMA is defined:
//! `slow - 1` candles return `None`, then candle `slow` returns the first
//! `Some`. We feed *every* candle to the underlying ta-rs MACD (warming its
//! recursive state) but gate emission on a candle counter. The warmup count is
//! pinned by an AC test.

use crate::adapters::indicators::convert::{decimal_to_f64, f64_to_decimal_rounded};
use crate::domain::{Candle, Indicator};
use rust_decimal::Decimal;
use ta::Next;
use ta::indicators::MovingAverageConvergenceDivergence;

/// MACD over closing prices, wrapping ta-rs behind the [`Indicator`] port.
///
/// Resolves to the **MACD line** (`EMA(fast) − EMA(slow)`), the v1 default for a
/// bare `Macd` spec (#18). Output is rounded to scale-8.
pub struct Macd {
    inner: MovingAverageConvergenceDivergence,
    /// Warmup bar-count: `slow - 1` candles are suppressed.
    warmup: u32,
    /// Number of candles fed so far.
    seen: u32,
}

impl Macd {
    /// Build a MACD from `fast`/`slow`/`signal` periods, each ≥ 1.
    ///
    /// Returns `None` if any period is 0 (ta-rs rejects a zero period).
    /// Constructed from concrete `u32`s (the `Fixed`-extraction factory is
    /// 3.03's concern). Panic-free: ta-rs's constructor error maps to `None`.
    #[must_use]
    pub fn new(fast: u32, slow: u32, signal: u32) -> Option<Self> {
        let inner =
            MovingAverageConvergenceDivergence::new(fast as usize, slow as usize, signal as usize)
                .ok()?;
        Some(Self {
            inner,
            warmup: slow,
            seen: 0,
        })
    }
}

impl Indicator for Macd {
    fn next(&mut self, candle: &Candle) -> Option<Decimal> {
        self.seen = self.seen.saturating_add(1);

        // Feed every candle so the recursive ta-rs EMA state warms, even during
        // warmup suppression. A non-representable price maps to `None`
        // defensively (never a panic).
        let input = decimal_to_f64(candle.close)?;
        let out = self.inner.next(input);

        // Warmup: suppress the first `slow - 1` candles; emit from candle
        // `slow` onward.
        if self.seen < self.warmup {
            return None;
        }
        // The v1 #18 default: the MACD line (`EMA(fast) − EMA(slow)`).
        f64_to_decimal_rounded(out.macd)
    }

    fn is_ready(&self) -> bool {
        // ready iff the NEXT call (the `seen + 1`-th candle) reaches candle
        // `warmup` or beyond.
        self.seen.saturating_add(1) >= self.warmup
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::Macd;
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

    /// Independent oracle (architect-critic C4): a seeded-EMA recurrence computed
    /// *here in the test*, NOT read back from ta-rs's own EMA objects. ta-rs EMA
    /// is seeded (out[0] = close[0]); thereafter `out[t] = k*c + (1-k)*out[t-1]`,
    /// `k = 2/(p+1)`. The MACD line is `EMA(fast) − EMA(slow)` over the same
    /// series. Returns the full per-candle MACD-line reference.
    fn macd_line_reference(closes: &[f64], fast: u32, slow: u32) -> Vec<f64> {
        let ema = |period: u32| -> Vec<f64> {
            let k = 2.0 / (f64::from(period) + 1.0);
            let mut out = Vec::with_capacity(closes.len());
            let mut prev = closes[0];
            out.push(prev);
            for &c in &closes[1..] {
                prev = k * c + (1.0 - k) * prev;
                out.push(prev);
            }
            out
        };
        let fast_ema = ema(fast);
        let slow_ema = ema(slow);
        fast_ema
            .iter()
            .zip(slow_ema.iter())
            .map(|(f, s)| f - s)
            .collect()
    }

    #[test]
    fn macd_returns_none_during_warmup_then_some() {
        let (fast, slow, signal) = (3u32, 6u32, 4u32);
        let warmup = slow; // MACD line first defined on the slow-period candle.
        let mut macd = Macd::new(fast, slow, signal).expect("periods >= 1");

        // Feed exactly `warmup - 1` candles → all None.
        for i in 1..warmup {
            let out = macd.next(&candle_close(&format!("{}", 100 + i)));
            assert_eq!(out, None, "candle {i} is warmup → None");
        }
        // Having fed exactly `slow - 1` candles, the NEXT call is candle
        // `warmup` → first defined MACD line.
        assert!(macd.is_ready(), "ready right before candle warmup");

        let out = macd.next(&candle_close("200"));
        assert!(out.is_some(), "candle warmup → Some");
        assert!(macd.is_ready(), "stays ready once warm");
    }

    #[test]
    fn macd_resolves_to_macd_line() {
        let (fast, slow, signal) = (3u32, 6u32, 4u32);
        let warmup = slow;
        // A series comfortably longer than warmup so we exercise warm output.
        let closes = [
            2.0_f64, 3.0, 4.2, 7.0, 6.7, 6.5, 8.1, 9.4, 8.8, 10.2, 11.5, 10.9, 12.3, 13.0, 12.1,
        ];
        let reference = macd_line_reference(&closes, fast, slow);

        let mut macd = Macd::new(fast, slow, signal).expect("periods >= 1");
        let mut idx: u32 = 0;
        for (i, &c) in closes.iter().enumerate() {
            idx += 1;
            let out = macd.next(&candle_close(&c.to_string()));
            if idx < warmup {
                assert_eq!(out, None, "warmup candle {idx} → None");
            } else {
                let got: f64 = out.expect("warm → Some").to_string().parse().unwrap();
                let expected = reference[i];
                assert!(
                    (got - expected).abs() < 1e-6,
                    "candle {idx}: macd line got {got}, expected {expected}"
                );
            }
        }
    }
}
