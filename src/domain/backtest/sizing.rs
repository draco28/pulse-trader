//! Risk-based position sizing (G5 / #20).
//!
//! [`position_size`] is the inline, constant-equity sizer this slice uses (the
//! shared `pulse-broker::compute_position_size` extraction + exchange
//! constraints are VS-1.2.2). Pure, `Decimal`-only.
//!
//! # The sizing identity (G5)
//!
//! `qty = (equity × risk_per_trade_pct) / |entry_price − stop_price|`
//!
//! i.e. position size is whatever quantity makes a stop-out cost exactly
//! `risk_per_trade_pct` of equity. The result is then **leverage-capped** so the
//! notional never exceeds `equity × max_leverage`:
//!
//! `qty × entry_price ≤ equity × max_leverage`
//!
//! A **zero stop distance** (`entry == stop`) has no risk denominator, so sizing
//! refuses with [`BacktestError::NoStopLoss`] rather than dividing by zero or
//! inventing a fallback (G5 / #20). Sizing is off `starting_equity` (constant-
//! equity, S3); compounding is the VS-1.2.4 equity curve's concern.

use rust_decimal::Decimal;

use super::error::BacktestError;

/// Compute the risk-based, leverage-capped position size (G5).
///
/// `equity` is the (constant) account equity; `risk_per_trade_pct` is a decimal
/// fraction (`0.01` = 1%, matching the DSL `RiskParams` convention);
/// `entry_price` / `stop_price` are the trade geometry; `max_leverage` is a
/// plain multiplier cap (`3` = 3×).
///
/// Returns the base-asset quantity. The risk-derived size is reduced to the
/// leverage cap when it would exceed `equity × max_leverage / entry_price`; it
/// is never increased.
///
/// # Errors
///
/// Returns [`BacktestError::NoStopLoss`] when `entry_price == stop_price` (zero
/// stop distance — no risk denominator, G5 / #20).
pub fn position_size(
    equity: Decimal,
    risk_per_trade_pct: Decimal,
    entry_price: Decimal,
    stop_price: Decimal,
    max_leverage: Decimal,
) -> Result<Decimal, BacktestError> {
    let stop_distance = (entry_price - stop_price).abs();
    if stop_distance.is_zero() {
        return Err(BacktestError::NoStopLoss);
    }

    let risk_qty = equity * risk_per_trade_pct / stop_distance;

    // Leverage cap: qty * entry <= equity * max_leverage.
    let max_notional = equity * max_leverage;
    let max_qty = max_notional / entry_price;

    Ok(risk_qty.min(max_qty))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::position_size;
    use crate::domain::backtest::error::BacktestError;
    use rust_decimal::Decimal;

    fn dec(n: i64, scale: u32) -> Decimal {
        Decimal::new(n, scale)
    }

    #[test]
    fn risk_based_size_when_under_leverage_cap() {
        // equity 10_000, risk 1% (0.01), entry 100, stop 95 → dist 5.
        // qty = 10_000 * 0.01 / 5 = 100 / 5 = 20.
        // notional = 20 * 100 = 2_000 <= 10_000 * 10 = 100_000 → uncapped.
        let qty = position_size(
            dec(10_000, 0),
            dec(1, 2), // 0.01
            dec(100, 0),
            dec(95, 0),
            dec(10, 0), // 10x cap
        )
        .unwrap();
        assert_eq!(qty, dec(20, 0));
    }

    #[test]
    fn leverage_cap_binds_for_a_tight_stop() {
        // A very tight stop drives the risk-size huge; the leverage cap binds.
        // equity 10_000, risk 1% (0.01), entry 100, stop 99.9 → dist 0.1.
        // risk_qty = 10_000 * 0.01 / 0.1 = 100 / 0.1 = 1_000.
        // max_qty = (10_000 * 3) / 100 = 30_000 / 100 = 300 < 1_000 → capped 300.
        let qty = position_size(
            dec(10_000, 0),
            dec(1, 2), // 0.01
            dec(100, 0),
            dec(999, 1), // 99.9
            dec(3, 0),   // 3x cap
        )
        .unwrap();
        assert_eq!(qty, dec(300, 0));
    }

    #[test]
    fn cap_exactly_at_boundary_keeps_risk_size() {
        // Construct equality: risk_qty == max_qty.
        // equity 10_000, risk 1%, entry 100, stop 96.666... is messy; instead
        // pick dist so risk_qty = max_qty. max_qty (3x) = 300 → need risk_qty 300
        // → dist = 10_000*0.01/300 = 100/300. Use a cleaner pair: equity 9_000,
        // risk 1%, entry 100, 3x → max_qty = 270; dist so 9_000*0.01/dist = 270
        // → dist = 90/270 = 1/3 → stop 100 - 1/3. Keep it exact via a 3-scale.
        // Simpler exactness: equity 1_000, risk 10% (0.1), entry 10, stop 9 →
        // dist 1, risk_qty = 1_000*0.1/1 = 100; max_qty (1x) = 1_000*1/10 = 100.
        // Equal → result 100 (cap does not reduce below the risk size).
        let qty = position_size(
            dec(1_000, 0),
            dec(1, 1), // 0.1
            dec(10, 0),
            dec(9, 0),
            dec(1, 0), // 1x cap
        )
        .unwrap();
        assert_eq!(qty, dec(100, 0));
    }

    #[test]
    fn zero_stop_distance_errors_nostoploss() {
        // entry == stop → NoStopLoss (G5 / #20), no divide-by-zero.
        let err = position_size(
            dec(10_000, 0),
            dec(1, 2),
            dec(100, 0),
            dec(100, 0),
            dec(10, 0),
        )
        .unwrap_err();
        assert_eq!(err, BacktestError::NoStopLoss);
    }
}
