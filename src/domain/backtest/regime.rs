//! Market-regime value types + pure classification (FR-5 / FR-6, BACKLOG-5).
//!
//! This is the **pure** half of the EMA50/200 + ADX(14) regime classifier (the
//! stateful detector that holds the indicator adapters lives in
//! [`crate::adapters::backtest::regime`], mirroring how `engine.rs` composes the
//! `IndicatorEngine`). It is zero-I/O, `Decimal`-only (NFR-2): no `f64`
//! arithmetic, no candle iteration, no `Trade` read. 2.04 feeds
//! [`RegimeBreakdown::record`] the `(regime, realized_pnl)` pairs — so this file
//! stays free of the `Trade.regime` field 2.04 adds.
//!
//! # Classification (slice README C7)
//!
//! Given **warm** indicator values (the caller supplies only `Some` values —
//! warmup → [`Regime::Unknown`] is the detector's concern, not [`classify`]'s):
//!
//! - `adx <= ADX_TREND_THRESHOLD` → [`Regime::Ranging`] (no trend strength);
//! - else `ema50 > ema200` → [`Regime::TrendingUp`];
//! - else `ema50 < ema200` → [`Regime::TrendingDown`];
//! - else (`ema50 == ema200`, degenerate) → [`Regime::Ranging`] (no directional
//!   bias).
//!
//! All comparisons are **exact `Decimal`** (the `Indicator` port emits
//! `Option<Decimal>` after the convert seam).

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// The ADX(14) threshold separating a trending market from a ranging one
/// (slice README C7). `adx <= ADX_TREND_THRESHOLD` is ranging; a strictly
/// greater ADX is required for a directional regime. Exact `Decimal` (25) — no
/// `f64` (NFR-2).
///
/// Constructed via `Decimal::from_parts` because `Decimal::new` is **not** a
/// `const fn` in `rust_decimal` 1.42 (a `const` initialiser requires the const
/// constructor). `from_parts(25, 0, 0, false, 0)` is the exact integer `25`
/// (mantissa `25`, scale `0`) — bit-for-bit equal to `Decimal::new(25, 0)`,
/// asserted in the unit tests.
pub const ADX_TREND_THRESHOLD: Decimal = Decimal::from_parts(25, 0, 0, false, 0);

/// The market regime a bar sits in: the EMA50-vs-EMA200 trend direction gated by
/// ADX(14) trend strength, plus [`Regime::Unknown`] while the indicators warm.
///
/// `Unknown` is a **first-class** state, never a silently-defaulted `Ranging`
/// (issue #16: a readiness gate must not collapse into a live state). The pure
/// [`classify`] never returns `Unknown` — only the detector does, while warming.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Regime {
    /// EMA50 > EMA200 with ADX above the trend threshold — an up-trend.
    TrendingUp,
    /// EMA50 < EMA200 with ADX above the trend threshold — a down-trend.
    TrendingDown,
    /// ADX at/below the trend threshold (or EMAs exactly equal) — no directional
    /// trend.
    Ranging,
    /// At least one indicator is still warming (#16) — regime is undetermined.
    Unknown,
}

/// Classify the regime from **warm** indicator values (slice README C7).
///
/// The caller (the detector) supplies only warm `Decimal` values; warmup →
/// [`Regime::Unknown`] is handled upstream, so this pure function returns only
/// one of the three non-`Unknown` variants. All comparisons are exact `Decimal`.
#[must_use]
pub fn classify(ema50: Decimal, ema200: Decimal, adx: Decimal) -> Regime {
    if adx <= ADX_TREND_THRESHOLD {
        // Below the trend-strength gate: ranging regardless of EMA order.
        Regime::Ranging
    } else if ema50 > ema200 {
        Regime::TrendingUp
    } else if ema50 < ema200 {
        Regime::TrendingDown
    } else {
        // ema50 == ema200: degenerate, no directional bias → ranging.
        Regime::Ranging
    }
}

/// One per-regime accumulator cell: how many trades closed in this regime and
/// their summed net P&L. `Decimal`-exact (NFR-2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegimeCell {
    /// Number of trades recorded against this regime.
    pub trade_count: usize,
    /// Summed net (realized) P&L of those trades.
    pub net_pnl: Decimal,
}

impl Default for RegimeCell {
    fn default() -> Self {
        Self {
            trade_count: 0,
            net_pnl: Decimal::ZERO,
        }
    }
}

/// Per-regime trade-count + net-P&L breakdown of a backtest run (FR-5).
///
/// One [`RegimeCell`] for each of the four [`Regime`] variants (including
/// `Unknown` — trades that opened while the regime was undetermined). 2.04 walks
/// the run feeding [`RegimeBreakdown::record`] the `(regime, realized_pnl)`
/// pairs; this type never reads `Trade` itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RegimeBreakdown {
    trending_up: RegimeCell,
    trending_down: RegimeCell,
    ranging: RegimeCell,
    unknown: RegimeCell,
}

impl RegimeBreakdown {
    /// A fresh, all-zero breakdown.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one closed trade's `(regime, net_pnl)` into the matching cell:
    /// increment its `trade_count` and add to its `net_pnl`. (2.04 feeds this.)
    pub fn record(&mut self, regime: Regime, net_pnl: Decimal) {
        let cell = match regime {
            Regime::TrendingUp => &mut self.trending_up,
            Regime::TrendingDown => &mut self.trending_down,
            Regime::Ranging => &mut self.ranging,
            Regime::Unknown => &mut self.unknown,
        };
        cell.trade_count = cell.trade_count.saturating_add(1);
        cell.net_pnl += net_pnl;
    }

    /// The [`Regime::TrendingUp`] cell.
    #[must_use]
    pub fn trending_up(&self) -> RegimeCell {
        self.trending_up
    }

    /// The [`Regime::TrendingDown`] cell.
    #[must_use]
    pub fn trending_down(&self) -> RegimeCell {
        self.trending_down
    }

    /// The [`Regime::Ranging`] cell.
    #[must_use]
    pub fn ranging(&self) -> RegimeCell {
        self.ranging
    }

    /// The [`Regime::Unknown`] cell.
    #[must_use]
    pub fn unknown(&self) -> RegimeCell {
        self.unknown
    }

    /// The cell for an arbitrary [`Regime`] (uniform accessor for renderers).
    #[must_use]
    pub fn cell(&self, regime: Regime) -> RegimeCell {
        match regime {
            Regime::TrendingUp => self.trending_up,
            Regime::TrendingDown => self.trending_down,
            Regime::Ranging => self.ranging,
            Regime::Unknown => self.unknown,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{ADX_TREND_THRESHOLD, Regime, RegimeBreakdown, classify};
    use rust_decimal::Decimal;

    fn dec(n: i64, scale: u32) -> Decimal {
        Decimal::new(n, scale)
    }

    #[test]
    fn adx_threshold_is_exactly_25() {
        // The C7 gate constant is exact Decimal 25 — no f64 drift (NFR-2).
        assert_eq!(ADX_TREND_THRESHOLD, dec(25, 0));
    }

    #[test]
    fn classify_truth_table() {
        // adx <= 25 → Ranging regardless of EMA order (the strength gate binds
        // first). Boundary: adx == 25 is ranging (<=, not <).
        assert_eq!(
            classify(dec(110, 0), dec(100, 0), dec(25, 0)),
            Regime::Ranging,
            "adx == threshold → Ranging even with ema50 > ema200"
        );
        assert_eq!(
            classify(dec(90, 0), dec(100, 0), dec(10, 0)),
            Regime::Ranging,
            "adx below threshold → Ranging even with ema50 < ema200"
        );

        // adx > 25 → directional, by EMA order.
        assert_eq!(
            classify(dec(110, 0), dec(100, 0), dec(30, 0)),
            Regime::TrendingUp,
            "ema50 > ema200 with strong adx → TrendingUp"
        );
        assert_eq!(
            classify(dec(90, 0), dec(100, 0), dec(30, 0)),
            Regime::TrendingDown,
            "ema50 < ema200 with strong adx → TrendingDown"
        );

        // Just-above-threshold (26 > 25) still trends — locks the strict `>`.
        assert_eq!(
            classify(dec(110, 0), dec(100, 0), dec(26, 0)),
            Regime::TrendingUp,
            "adx just above threshold → trend, not ranging"
        );

        // Degenerate ema50 == ema200 with strong adx → Ranging (no bias).
        assert_eq!(
            classify(dec(100, 0), dec(100, 0), dec(40, 0)),
            Regime::Ranging,
            "ema50 == ema200 (degenerate) → Ranging despite strong adx"
        );
    }

    #[test]
    fn classify_never_returns_unknown() {
        // The pure classifier only ever yields the three warm variants; Unknown
        // is exclusively the detector's warmup state (#16).
        for &(e50, e200, adx) in &[
            (110i64, 100i64, 30i64),
            (90, 100, 30),
            (100, 100, 10),
            (105, 100, 25),
        ] {
            let r = classify(dec(e50, 0), dec(e200, 0), dec(adx, 0));
            assert_ne!(r, Regime::Unknown, "classify must never emit Unknown");
        }
    }

    #[test]
    fn classify_uses_exact_decimal_fractions() {
        // Exact-Decimal comparison: ema50 = 100.00000001 > ema200 = 100 with a
        // strong adx → TrendingUp. A naive f64 round could collapse the gap; the
        // Decimal path preserves it (NFR-2).
        let ema50 = Decimal::new(10_000_000_001, 8); // 100.00000001
        let ema200 = dec(100, 0);
        assert_eq!(
            classify(ema50, ema200, dec(30, 0)),
            Regime::TrendingUp,
            "sub-cent EMA gap is preserved exactly → TrendingUp"
        );
    }

    #[test]
    fn breakdown_record_accumulates_per_regime() {
        let mut b = RegimeBreakdown::new();
        b.record(Regime::TrendingUp, dec(10, 0));
        b.record(Regime::TrendingUp, dec(5, 0));
        b.record(Regime::TrendingDown, dec(-3, 0));
        b.record(Regime::Ranging, dec(2, 0));
        b.record(Regime::Unknown, dec(7, 0));

        assert_eq!(b.trending_up().trade_count, 2);
        assert_eq!(b.trending_up().net_pnl, dec(15, 0));
        assert_eq!(b.trending_down().trade_count, 1);
        assert_eq!(b.trending_down().net_pnl, dec(-3, 0));
        assert_eq!(b.ranging().trade_count, 1);
        assert_eq!(b.ranging().net_pnl, dec(2, 0));
        // Trades that opened while warming land in the Unknown cell (#16) — they
        // are NOT silently merged into Ranging.
        assert_eq!(b.unknown().trade_count, 1);
        assert_eq!(b.unknown().net_pnl, dec(7, 0));
    }

    #[test]
    fn breakdown_starts_empty() {
        let b = RegimeBreakdown::new();
        for r in [
            Regime::TrendingUp,
            Regime::TrendingDown,
            Regime::Ranging,
            Regime::Unknown,
        ] {
            assert_eq!(b.cell(r).trade_count, 0);
            assert_eq!(b.cell(r).net_pnl, Decimal::ZERO);
        }
    }

    #[test]
    fn regime_serde_snake_case_round_trip() {
        // Regime serializes snake_case (matches the ExitReason/TradeSource
        // convention) and round-trips byte-stably.
        for (r, tag) in [
            (Regime::TrendingUp, "\"trending_up\""),
            (Regime::TrendingDown, "\"trending_down\""),
            (Regime::Ranging, "\"ranging\""),
            (Regime::Unknown, "\"unknown\""),
        ] {
            let json = serde_json::to_string(&r).expect("serialize Regime");
            assert_eq!(json, tag, "snake_case tag for {r:?}");
            let back: Regime = serde_json::from_str(&json).expect("deserialize Regime");
            assert_eq!(back, r);
        }
    }
}
