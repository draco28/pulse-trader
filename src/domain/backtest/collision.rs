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
//! - **Open gaps fire first:** the `open` is the chronologically-first price of
//!   the bar, so a bar that *opens* already beyond a level fills at the open
//!   before any intra-bar travel could reach the other level. Both open-gap
//!   checks (through the stop and through the take-profit) are therefore resolved
//!   *before* the intra-bar `low`/`high` checks. (A single bar cannot open beyond
//!   both levels at once — the stop sits below the TP for a long, above it for a
//!   short — so the two open-gap cases never collide.)
//! - **Intra-bar touch:** fill at the level itself (`stop_price` / `tp_price`).
//!   Slippage ([`apply_slippage`](super::cost::apply_slippage)) is the caller's
//!   later, separate step.
//! - **SL-first ties (G2):** only the genuinely *ambiguous* intra-bar case — a
//!   bar whose `[low, high]` range reaches **both** levels with the open inside
//!   the channel — resolves to the stop (the pessimistic outcome).
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
    // Open gaps resolve first (the open is the bar's first price). These two are
    // mutually exclusive for a long (stop < tp), so order between them is moot.
    if open <= stop_price {
        // Gapped down through the stop → honest fill at the (worse) open.
        return Some(IntraBarExit {
            reason: ExitReason::StopLoss,
            price: open,
        });
    }
    if open >= tp_price {
        // Gapped up through the take-profit → fill at the open.
        return Some(IntraBarExit {
            reason: ExitReason::TakeProfit,
            price: open,
        });
    }
    // Open inside the channel → intra-bar touches, SL-first (stop wins ties, G2).
    if low <= stop_price {
        return Some(IntraBarExit {
            reason: ExitReason::StopLoss,
            price: stop_price,
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
    // Open gaps resolve first (the open is the bar's first price). These two are
    // mutually exclusive for a short (tp < stop), so order between them is moot.
    if open >= stop_price {
        // Gapped up through the stop → fill at the (worse) open.
        return Some(IntraBarExit {
            reason: ExitReason::StopLoss,
            price: open,
        });
    }
    if open <= tp_price {
        // Gapped down through the take-profit → fill at the open.
        return Some(IntraBarExit {
            reason: ExitReason::TakeProfit,
            price: open,
        });
    }
    // Open inside the channel → intra-bar touches, SL-first (stop wins ties, G2).
    if high >= stop_price {
        return Some(IntraBarExit {
            reason: ExitReason::StopLoss,
            price: stop_price,
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

    #[test]
    fn long_opens_past_tp_and_reaches_stop_resolves_to_tp_at_open() {
        // opens at 112 (already above tp 110 → the chronologically-first price is
        // past the TP) while the bar's range also dips to the stop (low 90 <= 95).
        // An open gap is not intra-bar-ambiguous: the TP fills at the open, BEFORE
        // price could travel down to the stop → TakeProfit@open, NOT StopLoss.
        assert_eq!(
            resolve_intra_bar_exit(d(112), d(115), d(90), d(STOP_L), d(TP_L), Direction::Long),
            Some(IntraBarExit {
                reason: ExitReason::TakeProfit,
                price: d(112),
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

    #[test]
    fn short_opens_past_tp_and_reaches_stop_resolves_to_tp_at_open() {
        // Short: tp 90 (below), stop 105 (above). Opens at 88 (already below tp →
        // TP fills at the open) while the bar's range also rises to the stop
        // (high 110 >= 105). The open gap beats the intra-bar stop → TakeProfit@open.
        assert_eq!(
            resolve_intra_bar_exit(d(88), d(110), d(85), d(STOP_S), d(TP_S), Direction::Short),
            Some(IntraBarExit {
                reason: ExitReason::TakeProfit,
                price: d(88),
            })
        );
    }
}
