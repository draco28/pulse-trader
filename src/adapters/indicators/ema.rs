//! EMA adapter (walking skeleton) — wraps ta-rs `ExponentialMovingAverage`
//! behind the domain [`Indicator`] port over `candle.close`.
//!
//! ta-rs's EMA is *seeded*: it emits a value from its very first input (it has
//! no internal warmup). Our port convention, by contrast, returns `None` for the
//! first `period − 1` candles and `Some` from candle `period` onward — so the
//! first emitted value aligns with pandas-ta's first non-NaN EMA row (the
//! alignment 3.04 cross-validation depends on). We therefore feed *every* candle
//! to the underlying ta-rs EMA (warming its recursive state) but gate emission
//! on a candle counter. The warmup count is pinned by an AC test.

use crate::adapters::indicators::convert::{decimal_to_f64, f64_to_decimal_rounded};
use crate::domain::{Candle, Indicator};
use rust_decimal::Decimal;
use ta::Next;
use ta::indicators::ExponentialMovingAverage;

/// EMA(period) over closing prices, wrapping ta-rs behind the [`Indicator`] port.
pub struct Ema {
    inner: ExponentialMovingAverage,
    period: u32,
    /// Number of candles fed so far.
    seen: u32,
}

impl Ema {
    /// Build an EMA over `period` candles. `period` must be ≥ 1.
    ///
    /// Returns `None` if `period` is 0 (ta-rs rejects a zero period). Constructed
    /// from a concrete `u32` (the `Fixed`-extraction factory that resolves
    /// `SweepableValue` periods is 3.03's concern).
    #[must_use]
    pub fn new(period: u32) -> Option<Self> {
        // ta-rs takes `usize`; `period` is small (≤ a few hundred). Reject 0 by
        // letting ta-rs's constructor error map to `None` (panic-free).
        let inner = ExponentialMovingAverage::new(period as usize).ok()?;
        Some(Self {
            inner,
            period,
            seen: 0,
        })
    }
}

impl Indicator for Ema {
    fn next(&mut self, candle: &Candle) -> Option<Decimal> {
        // Convert BEFORE advancing the warmup counter: a non-representable price
        // (`None`) must not desync `seen`/readiness from the inner ta-rs state (a
        // phase shift in the determinism layer). Feed every valid candle so the
        // recursive ta-rs state warms even while output is suppressed.
        let input = decimal_to_f64(candle.close)?;
        self.seen = self.seen.saturating_add(1);
        let out = self.inner.next(input);

        // Warmup: suppress the first `period − 1` emissions; emit from candle
        // `period` onward (pandas-ta alignment).
        if self.seen < self.period {
            return None;
        }
        f64_to_decimal_rounded(out)
    }

    fn is_ready(&self) -> bool {
        // ready iff the NEXT call (the `seen + 1`-th candle) reaches candle
        // `period` or beyond.
        self.seen.saturating_add(1) >= self.period
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::Ema;
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

    #[test]
    fn ema_returns_none_during_warmup_then_some() {
        let period = 5u32;
        let mut ema = Ema::new(period).expect("period >= 1");

        // First `period - 1` candles: warmup → None, not ready until the last.
        for i in 1..period {
            let before_ready = ema.is_ready();
            let out = ema.next(&candle_close("100"));
            assert_eq!(out, None, "candle {i} is warmup → None");
            // After feeding the (period-1)-th candle, the next call is candle
            // `period`, so is_ready() flips true exactly there.
            let _ = before_ready;
        }
        // Having fed `period - 1` candles, the NEXT call is candle `period`.
        assert!(ema.is_ready(), "ready right before the period-th candle");

        // The `period`-th candle: Some + still ready.
        let out = ema.next(&candle_close("100"));
        assert!(out.is_some(), "period-th candle → Some");
        assert!(ema.is_ready(), "stays ready once warm");
    }

    #[test]
    fn ema_matches_reference_within_epsilon() {
        // ta-rs EMA is seeded: out[0] = close[0]; thereafter
        //   out[t] = k*close[t] + (1-k)*out[t-1],  k = 2/(period+1).
        // We gate emission to start at candle `period`, but the recursive state
        // is the same as the hand-computed sequence.
        let period = 3u32;
        let k = 2.0 / (f64::from(period) + 1.0); // 0.5
        let closes = [2.0_f64, 5.0, 1.0, 6.25, 10.0];

        // Hand-compute the full seeded EMA sequence.
        let mut reference = Vec::new();
        let mut prev = closes[0];
        reference.push(prev);
        for &c in &closes[1..] {
            prev = k * c + (1.0 - k) * prev;
            reference.push(prev);
        }

        let mut ema = Ema::new(period).expect("period >= 1");
        for (i, &c) in closes.iter().enumerate() {
            let candle = candle_close(&c.to_string());
            let out = ema.next(&candle);
            let idx = i + 1; // 1-based candle index
            if idx < period as usize {
                assert_eq!(out, None, "warmup candle {idx} → None");
            } else {
                let got: f64 = out.expect("warm → Some").to_string().parse().unwrap();
                let expected = reference[i];
                assert!(
                    (got - expected).abs() < 1e-6,
                    "candle {idx}: got {got}, expected {expected}"
                );
            }
        }
    }

    #[test]
    fn ema_is_deterministic_across_repeated_runs() {
        let period = 4u32;
        let closes = [
            "100.123456789",
            "101.5",
            "99.987654321",
            "102.25",
            "103.0",
            "101.75",
        ];

        let run = || -> Vec<Option<Decimal>> {
            let mut ema = Ema::new(period).expect("period >= 1");
            closes.iter().map(|c| ema.next(&candle_close(c))).collect()
        };

        let first = run();
        let second = run();
        // Byte-identical (exact `Decimal` equality) — the self-determinism
        // guarantee (NFR-2).
        assert_eq!(
            first, second,
            "repeated runs yield identical Vec<Option<Decimal>>"
        );
    }
}
