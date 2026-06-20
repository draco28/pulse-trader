//! `BacktestResult` — the pure in-memory aggregate the event loop (1.03)
//! produces: the trade log plus net P&L and the cost totals.
//!
//! This is the **whole** v1.2.1 output surface — no `SummaryStats`
//! (expectancy / Sharpe / drawdown / streaks) and no equity curve; those are
//! VS-1.2.4. All money figures are `Decimal` (NFR-2). 1.01 defines the shape;
//! 1.03 populates it; 1.04 renders it to stdout.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::regime::RegimeBreakdown;
use super::trade::Trade;
use crate::domain::sizing::SkippedEntryCounts;

/// The result of one backtest run: the trade log plus run-level totals.
///
/// `net_pnl` is the sum of each trade's `realized_pnl` (already net of costs);
/// the `*_total` fields are the run-wide cost roll-ups, surfaced separately so
/// the demo's "fees/funding/slippage are deducted" readout (1.04) has them
/// without re-summing the trade log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BacktestResult {
    /// Every trade the run produced, in chronological order.
    pub trades: Vec<Trade>,
    /// Net P&L across the run (quote currency, already net of costs).
    pub net_pnl: Decimal,
    /// Total taker fees paid across the run.
    pub fees_total: Decimal,
    /// Total signed funding P&L delta across the run — **negative when positions
    /// paid funding, positive when they received** (matches `funding_payment`).
    pub funding_total: Decimal,
    /// Total adverse slippage cost across the run.
    pub slippage_total: Decimal,

    /// Per-regime trade-count + net-P&L breakdown over the run (FR-5, VS-1.2.2
    /// work-2.04), aggregated in `into_result` by feeding each trade's
    /// `(regime, realized_pnl)` to [`RegimeBreakdown::record`]. **Deliberately
    /// NOT a frozen golden constant** — it is threshold-on-`f64`-EMA/ADX derived
    /// and inherits the deferred #29 cross-arch determinism caveat (deterministic
    /// on the v1 pinned toolchain, not byte-portable). 2.05 renders it.
    pub regime_breakdown: RegimeBreakdown,
    /// Per-reason tally of entries the exchange-constrained sizer suppressed over
    /// the run (audit C4): a bounded O(1) [`SkippedEntryCounts`] (sub-lot /
    /// sub-notional / leverage-capped), populated in `into_result`. 2.05 renders.
    pub skipped_entries: SkippedEntryCounts,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::BacktestResult;
    use rust_decimal::Decimal;

    fn empty_result() -> BacktestResult {
        BacktestResult {
            trades: Vec::new(),
            net_pnl: Decimal::ZERO,
            fees_total: Decimal::ZERO,
            funding_total: Decimal::ZERO,
            slippage_total: Decimal::ZERO,
            regime_breakdown: crate::domain::backtest::RegimeBreakdown::new(),
            skipped_entries: crate::domain::sizing::SkippedEntryCounts::new(),
        }
    }

    #[test]
    fn empty_result_has_no_trades_and_zero_totals() {
        let r = empty_result();
        assert!(r.trades.is_empty());
        assert_eq!(r.net_pnl, Decimal::ZERO);
        assert_eq!(r.fees_total, Decimal::ZERO);
    }

    #[test]
    fn result_serde_round_trips() {
        let r = empty_result();
        let json = serde_json::to_string(&r).expect("serialize BacktestResult");
        let back: BacktestResult = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(r, back);
    }
}
