//! Conservative intra-bar SL/TP collision resolution (G2).
//!
//! With OHLC-only data we cannot know the *order* in which a bar touched its
//! stop and its take-profit. The conservative rule (G2) is **SL-first**: if a
//! single bar's `[low, high]` range reaches both levels, the stop wins (the
//! pessimistic outcome). [`resolve_intra_bar_exit`] is a pure decision function
//! over one bar's open/high/low + the two trigger levels + the trade direction.
//!
//! # Fill geometry
//!
//! - **Intra-bar touch:** fill at the level itself (`stop_price` / `tp_price`).
//!   Slippage ([`apply_slippage`](super::cost::apply_slippage)) is the caller's
//!   later, separate step.
//! - **Gap-through (worse for us):** if the bar *opens* already beyond the stop,
//!   the honest fill is the **open** (a long gapping down through its stop fills
//!   at the open, below the stop). A take-profit gap fills at the open **only
//!   when the stop was not also gapped** — the stop still wins ties (G2).
//!
//! This module performs no sizing, no P&L, and no slippage — just "which level
//! fired and at what price".

use rust_decimal::Decimal;

use super::trade::ExitReason;
use crate::domain::Direction;

/// The resolved intra-bar exit for one bar: which level fired and the (pre-
/// slippage) fill price. [`ExitReason::StopLoss`] or [`ExitReason::TakeProfit`]
/// only — `Signal` / `EndOfData` exits are not intra-bar (they are the loop's
/// concern).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntraBarExit {
    /// Which level fired.
    pub reason: ExitReason,
    /// The (pre-slippage) fill price: the level itself, or the bar open on a
    /// gap-through.
    pub price: Decimal,
}

/// Resolve which exit (if any) fires on this bar for an open position (G2).
///
/// Conservative SL-first: a bar reaching **both** levels resolves to the stop.
/// Gap-through fills at the bar `open`; the stop wins gap ties. Returns `None`
/// when neither level is reached on this bar (the position stays open).
///
/// `stop_price` / `tp_price` are the geometry levels (from
/// [`stop_price`](crate::domain::stop_price) /
/// [`take_profit_price`](crate::domain::take_profit_price)); slippage is applied
/// later by the caller.
#[must_use]
pub fn resolve_intra_bar_exit(
    open: Decimal,
    high: Decimal,
    low: Decimal,
    stop_price: Decimal,
    tp_price: Decimal,
    direction: Direction,
) -> Option<IntraBarExit> {
    match direction {
        Direction::Long => resolve_long(open, high, low, stop_price, tp_price),
        Direction::Short => resolve_short(open, high, low, stop_price, tp_price),
    }
}

/// Long position: stop sits *below* entry (reached when price falls to it), TP
/// sits *above* (reached when price rises to it).
fn resolve_long(
    open: Decimal,
    high: Decimal,
    low: Decimal,
    stop_price: Decimal,
    tp_price: Decimal,
) -> Option<IntraBarExit> {
    // Stop first (SL-first / stop wins ties).
    if open <= stop_price {
        // Gapped down through the stop → honest fill at the (worse) open.
        return Some(IntraBarExit {
            reason: ExitReason::StopLoss,
            price: open,
        });
    }
    if low <= stop_price {
        return Some(IntraBarExit {
            reason: ExitReason::StopLoss,
            price: stop_price,
        });
    }
    // Stop not reached → consider the take-profit.
    if open >= tp_price {
        return Some(IntraBarExit {
            reason: ExitReason::TakeProfit,
            price: open,
        });
    }
    if high >= tp_price {
        return Some(IntraBarExit {
            reason: ExitReason::TakeProfit,
            price: tp_price,
        });
    }
    None
}

/// Short position: stop sits *above* entry (reached when price rises to it), TP
/// sits *below* (reached when price falls to it).
fn resolve_short(
    open: Decimal,
    high: Decimal,
    low: Decimal,
    stop_price: Decimal,
    tp_price: Decimal,
) -> Option<IntraBarExit> {
    if open >= stop_price {
        // Gapped up through the stop → fill at the (worse) open.
        return Some(IntraBarExit {
            reason: ExitReason::StopLoss,
            price: open,
        });
    }
    if high >= stop_price {
        return Some(IntraBarExit {
            reason: ExitReason::StopLoss,
            price: stop_price,
        });
    }
    if open <= tp_price {
        return Some(IntraBarExit {
            reason: ExitReason::TakeProfit,
            price: open,
        });
    }
    if low <= tp_price {
        return Some(IntraBarExit {
            reason: ExitReason::TakeProfit,
            price: tp_price,
        });
    }
    None
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{IntraBarExit, resolve_intra_bar_exit};
    use crate::domain::Direction;
    use crate::domain::backtest::trade::ExitReason;
    use rust_decimal::Decimal;

    fn d(n: i64) -> Decimal {
        Decimal::new(n, 0)
    }

    // Long fixture: entry ~100, stop 95, tp 110.
    const STOP_L: i64 = 95;
    const TP_L: i64 = 110;

    #[test]
    fn long_neither_reached_stays_open() {
        // bar entirely inside (95, 110): open 100, high 108, low 97 → None.
        assert_eq!(
            resolve_intra_bar_exit(d(100), d(108), d(97), d(STOP_L), d(TP_L), Direction::Long),
            None
        );
    }

    #[test]
    fn long_stop_only_fills_at_stop_level() {
        // low touches the stop, high doesn't reach tp.
        assert_eq!(
            resolve_intra_bar_exit(d(100), d(102), d(94), d(STOP_L), d(TP_L), Direction::Long),
            Some(IntraBarExit {
                reason: ExitReason::StopLoss,
                price: d(STOP_L),
            })
        );
    }

    #[test]
    fn long_tp_only_fills_at_tp_level() {
        // high reaches tp, low stays above the stop.
        assert_eq!(
            resolve_intra_bar_exit(d(100), d(111), d(99), d(STOP_L), d(TP_L), Direction::Long),
            Some(IntraBarExit {
                reason: ExitReason::TakeProfit,
                price: d(TP_L),
            })
        );
    }

    #[test]
    fn long_both_reached_resolves_to_stop() {
        // wide bar: low 90 (below stop) AND high 115 (above tp) → SL-first.
        assert_eq!(
            resolve_intra_bar_exit(d(100), d(115), d(90), d(STOP_L), d(TP_L), Direction::Long),
            Some(IntraBarExit {
                reason: ExitReason::StopLoss,
                price: d(STOP_L),
            })
        );
    }

    #[test]
    fn long_gap_through_stop_fills_at_open() {
        // bar opens at 92, already below the 95 stop → fill at the (worse) open.
        assert_eq!(
            resolve_intra_bar_exit(d(92), d(96), d(91), d(STOP_L), d(TP_L), Direction::Long),
            Some(IntraBarExit {
                reason: ExitReason::StopLoss,
                price: d(92),
            })
        );
    }

    #[test]
    fn long_gap_through_tp_fills_at_open_when_stop_not_gapped() {
        // opens at 112, above tp 110, low 108 stays above the stop → TP at open.
        assert_eq!(
            resolve_intra_bar_exit(d(112), d(115), d(108), d(STOP_L), d(TP_L), Direction::Long),
            Some(IntraBarExit {
                reason: ExitReason::TakeProfit,
                price: d(112),
            })
        );
    }

    #[test]
    fn long_gap_through_both_stop_still_wins() {
        // opens at 92 (below stop) and the bar also spans tp (high 120). The
        // stop gap wins the tie → StopLoss at the open.
        assert_eq!(
            resolve_intra_bar_exit(d(92), d(120), d(90), d(STOP_L), d(TP_L), Direction::Long),
            Some(IntraBarExit {
                reason: ExitReason::StopLoss,
                price: d(92),
            })
        );
    }

    // Short fixture: entry ~100, stop 105 (above), tp 90 (below).
    const STOP_S: i64 = 105;
    const TP_S: i64 = 90;

    #[test]
    fn short_neither_reached_stays_open() {
        assert_eq!(
            resolve_intra_bar_exit(d(100), d(103), d(92), d(STOP_S), d(TP_S), Direction::Short),
            None
        );
    }

    #[test]
    fn short_stop_only_fills_at_stop_level() {
        // high reaches the stop (105), low stays above tp.
        assert_eq!(
            resolve_intra_bar_exit(d(100), d(106), d(98), d(STOP_S), d(TP_S), Direction::Short),
            Some(IntraBarExit {
                reason: ExitReason::StopLoss,
                price: d(STOP_S),
            })
        );
    }

    #[test]
    fn short_tp_only_fills_at_tp_level() {
        // low reaches tp (90), high stays below the stop.
        assert_eq!(
            resolve_intra_bar_exit(d(100), d(103), d(89), d(STOP_S), d(TP_S), Direction::Short),
            Some(IntraBarExit {
                reason: ExitReason::TakeProfit,
                price: d(TP_S),
            })
        );
    }

    #[test]
    fn short_both_reached_resolves_to_stop() {
        // wide bar spanning both: high 110 (>= stop) and low 85 (<= tp) → stop.
        assert_eq!(
            resolve_intra_bar_exit(d(100), d(110), d(85), d(STOP_S), d(TP_S), Direction::Short),
            Some(IntraBarExit {
                reason: ExitReason::StopLoss,
                price: d(STOP_S),
            })
        );
    }

    #[test]
    fn short_gap_through_stop_fills_at_open() {
        // opens at 108, already above the 105 stop → fill at the (worse) open.
        assert_eq!(
            resolve_intra_bar_exit(d(108), d(110), d(104), d(STOP_S), d(TP_S), Direction::Short),
            Some(IntraBarExit {
                reason: ExitReason::StopLoss,
                price: d(108),
            })
        );
    }

    #[test]
    fn short_gap_through_tp_fills_at_open_when_stop_not_gapped() {
        // opens at 88 (below tp 90), high 92 stays below the stop → TP at open.
        assert_eq!(
            resolve_intra_bar_exit(d(88), d(92), d(85), d(STOP_S), d(TP_S), Direction::Short),
            Some(IntraBarExit {
                reason: ExitReason::TakeProfit,
                price: d(88),
            })
        );
    }
}
