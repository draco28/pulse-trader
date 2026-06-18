//! Sequential backtest loop over an aligned candle feed.

use rust_decimal::Decimal;

use crate::adapters::indicators::engine::IndicatorEngine;
use crate::domain::{
    BacktestError, BacktestResult, Candle, CandleSeries, CompiledCondition, CompiledExit,
    CompiledStrategy, Direction, ExitReason, Fill, IntraBarExit, Side, Trade, TradeSource, align,
    apply_slippage, funding_payment, position_size, realized_pnl, realized_r,
    resolve_intra_bar_exit, stop_price, take_profit_price, taker_fee,
};

/// Runtime knobs for the deterministic backtest loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BacktestConfig {
    /// Starting account equity used for constant-equity sizing.
    pub starting_equity: Decimal,
    /// Taker fee in basis points.
    pub taker_fee_bps: Decimal,
    /// Adverse fill slippage in basis points.
    pub slippage_bps: Decimal,
}

impl Default for BacktestConfig {
    fn default() -> Self {
        Self {
            starting_equity: Decimal::new(10_000, 0),
            taker_fee_bps: Decimal::new(4, 0),
            slippage_bps: Decimal::ONE,
        }
    }
}

/// Run one sequential, deterministic backtest.
///
/// # Errors
///
/// Returns [`BacktestError`] for strategy preconditions or sizing failures.
pub fn run_backtest(
    compiled: &CompiledStrategy,
    primary: &CandleSeries,
    htf: Option<&CandleSeries>,
    config: &BacktestConfig,
) -> Result<BacktestResult, BacktestError> {
    let exit_plan = ExitPlan::from_strategy(compiled)?;
    let mut engine = IndicatorEngine::new(compiled)
        .map_err(|err| BacktestError::UnsupportedExit(format!("indicator engine: {err}")))?;
    let mut state = LoopState::default();
    let direction = compiled.direction();

    for bar in align(primary, htf) {
        fill_pending_entry(&mut state, bar.primary, direction, &exit_plan, config)?;
        close_on_bar_open_or_price(&mut state, primary, bar.primary, config)?;

        engine.step(bar.primary);

        if state.position.is_some()
            && state.pending_exit.is_none()
            && exit_plan.signal_triggered(&engine)
        {
            state.pending_exit = Some(PendingExit {
                signal_time: bar.primary.close_time,
                reason: ExitReason::Signal,
            });
        }

        if state.position.is_none()
            && state.pending_entry.is_none()
            && bar.index > 0
            && engine.is_warm()
            && compiled.entry().eval(&engine)
        {
            state.pending_entry = Some(PendingEntry {
                signal_time: bar.primary.close_time,
            });
        }
    }

    close_end_of_data(&mut state, primary, config)?;
    Ok(state.into_result())
}

#[derive(Debug, Clone)]
struct ExitPlan<'a> {
    stop_distance_pct: Decimal,
    take_profit_target_r: Option<Decimal>,
    signal_exits: Vec<&'a CompiledCondition>,
    risk_per_trade_pct: Decimal,
    max_leverage: Decimal,
}

impl<'a> ExitPlan<'a> {
    fn from_strategy(compiled: &'a CompiledStrategy) -> Result<Self, BacktestError> {
        let Some(stop_distance_pct) = stop_distance(compiled.exits()) else {
            return Err(BacktestError::NoStopLoss);
        };
        reject_unsupported(compiled.exits())?;
        Ok(Self {
            stop_distance_pct,
            take_profit_target_r: take_profit_target(compiled.exits()),
            signal_exits: signal_exits(compiled.exits()),
            risk_per_trade_pct: compiled.risk().risk_per_trade_pct,
            max_leverage: compiled.risk().max_leverage,
        })
    }

    fn signal_triggered(&self, engine: &IndicatorEngine) -> bool {
        self.signal_exits
            .iter()
            .any(|condition| condition.eval(engine))
    }
}

#[derive(Debug, Default)]
struct LoopState {
    pending_entry: Option<PendingEntry>,
    pending_exit: Option<PendingExit>,
    position: Option<OpenPosition>,
    trades: Vec<Trade>,
}

impl LoopState {
    fn into_result(self) -> BacktestResult {
        let mut result = BacktestResult {
            trades: self.trades,
            net_pnl: Decimal::ZERO,
            fees_total: Decimal::ZERO,
            funding_total: Decimal::ZERO,
            slippage_total: Decimal::ZERO,
        };
        for trade in &result.trades {
            result.net_pnl += trade.realized_pnl;
            result.fees_total += trade.fees_total;
            result.funding_total += trade.funding_total;
            result.slippage_total += trade.slippage_total;
        }
        result
    }
}

#[derive(Debug, Clone, Copy)]
struct PendingEntry {
    signal_time: i64,
}

#[derive(Debug, Clone, Copy)]
struct PendingExit {
    signal_time: i64,
    reason: ExitReason,
}

#[derive(Debug, Clone, Copy)]
struct OpenPosition {
    direction: Direction,
    qty: Decimal,
    entry_price: Decimal,
    stop_price: Decimal,
    take_profit_price: Option<Decimal>,
    entry_signal_time: i64,
    entry_fill_time: i64,
    entry_fee: Decimal,
    entry_slippage: Decimal,
}

fn fill_pending_entry(
    state: &mut LoopState,
    candle: &Candle,
    direction: Direction,
    plan: &ExitPlan<'_>,
    config: &BacktestConfig,
) -> Result<(), BacktestError> {
    let Some(pending) = state.pending_entry.take() else {
        return Ok(());
    };
    if state.position.is_some() {
        return Ok(());
    }

    let raw_entry = candle.open;
    let entry_price = apply_slippage(raw_entry, config.slippage_bps, direction, Side::Entry);
    let stop = stop_price(entry_price, plan.stop_distance_pct, direction);
    let qty = position_size(
        config.starting_equity,
        plan.risk_per_trade_pct,
        entry_price,
        stop,
        plan.max_leverage,
    )?;
    let entry_fee = taker_fee(qty * entry_price, config.taker_fee_bps);
    state.position = Some(OpenPosition {
        direction,
        qty,
        entry_price,
        stop_price: stop,
        take_profit_price: plan.take_profit_target_r.map(|target| {
            take_profit_price(entry_price, plan.stop_distance_pct, target, direction)
        }),
        entry_signal_time: pending.signal_time,
        entry_fill_time: candle.open_time,
        entry_fee,
        entry_slippage: (entry_price - raw_entry).abs() * qty,
    });
    Ok(())
}

fn close_on_bar_open_or_price(
    state: &mut LoopState,
    primary: &CandleSeries,
    candle: &Candle,
    config: &BacktestConfig,
) -> Result<(), BacktestError> {
    let Some(position) = state.position else {
        state.pending_exit = None;
        return Ok(());
    };

    if let Some(exit) = price_exit(candle, &position) {
        close_position(state, primary, exit, config)?;
        state.pending_exit = None;
        return Ok(());
    }

    if let Some(pending) = state.pending_exit.take() {
        let exit = ExitFill {
            signal_time: pending.signal_time,
            fill_time: candle.open_time,
            raw_price: candle.open,
            reason: pending.reason,
        };
        close_position(state, primary, exit, config)?;
    }
    Ok(())
}

fn close_end_of_data(
    state: &mut LoopState,
    primary: &CandleSeries,
    config: &BacktestConfig,
) -> Result<(), BacktestError> {
    let Some(last) = primary.candles.last() else {
        return Ok(());
    };
    if state.position.is_none() {
        return Ok(());
    }
    let exit = ExitFill {
        signal_time: last.close_time,
        fill_time: last.close_time,
        raw_price: last.close,
        reason: ExitReason::EndOfData,
    };
    close_position(state, primary, exit, config)
}

#[derive(Debug, Clone, Copy)]
struct ExitFill {
    signal_time: i64,
    fill_time: i64,
    raw_price: Decimal,
    reason: ExitReason,
}

fn price_exit(candle: &Candle, position: &OpenPosition) -> Option<ExitFill> {
    let exit = match position.take_profit_price {
        Some(tp) => resolve_intra_bar_exit(
            candle.open,
            candle.high,
            candle.low,
            position.stop_price,
            tp,
            position.direction,
        ),
        None => stop_only_exit(candle, position),
    }?;
    Some(ExitFill {
        signal_time: candle.open_time,
        fill_time: candle.open_time,
        raw_price: exit.price,
        reason: exit.reason,
    })
}

fn stop_only_exit(candle: &Candle, position: &OpenPosition) -> Option<IntraBarExit> {
    match position.direction {
        Direction::Long if candle.open <= position.stop_price => Some(IntraBarExit {
            reason: ExitReason::StopLoss,
            price: candle.open,
        }),
        Direction::Long if candle.low <= position.stop_price => Some(IntraBarExit {
            reason: ExitReason::StopLoss,
            price: position.stop_price,
        }),
        Direction::Short if candle.open >= position.stop_price => Some(IntraBarExit {
            reason: ExitReason::StopLoss,
            price: candle.open,
        }),
        Direction::Short if candle.high >= position.stop_price => Some(IntraBarExit {
            reason: ExitReason::StopLoss,
            price: position.stop_price,
        }),
        _ => None,
    }
}

fn close_position(
    state: &mut LoopState,
    primary: &CandleSeries,
    exit: ExitFill,
    config: &BacktestConfig,
) -> Result<(), BacktestError> {
    let Some(position) = state.position.take() else {
        return Ok(());
    };
    let exit_price = apply_slippage(
        exit.raw_price,
        config.slippage_bps,
        position.direction,
        Side::Exit,
    );
    let exit_fee = taker_fee(position.qty * exit_price, config.taker_fee_bps);
    let funding_total = funding_between(primary, &position, exit.fill_time);
    let fees_total = position.entry_fee + exit_fee;
    let slippage_total =
        position.entry_slippage + (exit.raw_price - exit_price).abs() * position.qty;
    let gross = realized_pnl(
        position.entry_price,
        exit_price,
        position.qty,
        position.direction,
    );
    let net = gross + funding_total - fees_total - slippage_total;
    let realized_r = realized_r(
        position.entry_price,
        exit_price,
        position.stop_price,
        position.direction,
    )?;

    state.trades.push(Trade {
        direction: position.direction,
        qty: position.qty,
        entry_price: position.entry_price,
        exit_price,
        entry_signal_time: position.entry_signal_time,
        entry_fill_time: position.entry_fill_time,
        exit_signal_time: exit.signal_time,
        exit_fill_time: exit.fill_time,
        fills: vec![
            Fill {
                price: position.entry_price,
                qty: position.qty,
                time_ms: position.entry_fill_time,
                fee: position.entry_fee,
            },
            Fill {
                price: exit_price,
                qty: position.qty,
                time_ms: exit.fill_time,
                fee: exit_fee,
            },
        ],
        fees_total,
        funding_total,
        slippage_total,
        realized_pnl: net,
        realized_r,
        exit_reason: exit.reason,
        source: TradeSource::Backtest,
    });
    Ok(())
}

fn funding_between(
    primary: &CandleSeries,
    position: &OpenPosition,
    exit_fill_time: i64,
) -> Decimal {
    let notional = position.qty * position.entry_price;
    primary
        .candles
        .iter()
        .filter(|candle| {
            candle.open_time > position.entry_fill_time && candle.open_time <= exit_fill_time
        })
        .filter_map(|candle| candle.funding_rate)
        .map(|rate| funding_payment(rate, notional, position.direction))
        .sum()
}

fn stop_distance(exits: &[CompiledExit]) -> Option<Decimal> {
    exits.iter().find_map(|exit| match exit {
        CompiledExit::StopLoss { distance_pct } => Some(*distance_pct),
        _ => None,
    })
}

fn take_profit_target(exits: &[CompiledExit]) -> Option<Decimal> {
    exits.iter().find_map(|exit| match exit {
        CompiledExit::TakeProfit { target_r } => Some(*target_r),
        _ => None,
    })
}

fn signal_exits(exits: &[CompiledExit]) -> Vec<&CompiledCondition> {
    exits
        .iter()
        .filter_map(|exit| match exit {
            CompiledExit::SignalExit { condition } => Some(condition),
            _ => None,
        })
        .collect()
}

fn reject_unsupported(exits: &[CompiledExit]) -> Result<(), BacktestError> {
    for exit in exits {
        match exit {
            CompiledExit::TrailingStop { .. } => {
                return Err(BacktestError::UnsupportedExit("TrailingStop".to_owned()));
            }
            CompiledExit::TimeStop { .. } => {
                return Err(BacktestError::UnsupportedExit("TimeStop".to_owned()));
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{BacktestConfig, run_backtest};
    use crate::domain::{
        BacktestError, Candle, CandleSeries, Comparator, CompiledStrategy, Condition, DataVersion,
        Direction, ExitReason, ExitRule, Pair, PriceField, RiskParams, SchemaVersion, StrategyDsl,
        SweepableValue, Timeframe, ValueSource, compile, validate,
    };
    use rust_decimal::Decimal;

    fn d(value: i64) -> Decimal {
        Decimal::new(value, 0)
    }

    fn rate(mantissa: i64, scale: u32) -> Decimal {
        Decimal::new(mantissa, scale)
    }

    fn config() -> BacktestConfig {
        BacktestConfig {
            starting_equity: d(10_000),
            taker_fee_bps: Decimal::ZERO,
            slippage_bps: Decimal::ZERO,
        }
    }

    fn candle(idx: i64, open: i64, high: i64, low: i64, close: i64) -> Candle {
        Candle {
            open_time: idx * 60_000,
            close_time: idx * 60_000 + 59_999,
            open: d(open),
            high: d(high),
            low: d(low),
            close: d(close),
            volume: Decimal::ONE,
            funding_rate: None,
        }
    }

    fn funding_candle(idx: i64, open: i64, high: i64, low: i64, close: i64) -> Candle {
        Candle {
            funding_rate: Some(rate(1, 3)),
            ..candle(idx, open, high, low, close)
        }
    }

    fn series(candles: Vec<Candle>) -> CandleSeries {
        CandleSeries {
            pair: Pair::new("BTCUSDT"),
            timeframe: Timeframe::M15,
            version: DataVersion::new("test"),
            candles,
        }
    }

    fn price_entry() -> Condition {
        Condition::Compare {
            lhs: ValueSource::Price {
                field: PriceField::Close,
            },
            op: Comparator::Gt,
            rhs: ValueSource::Constant {
                value: Decimal::ZERO,
            },
        }
    }

    fn never_signal() -> Condition {
        Condition::Compare {
            lhs: ValueSource::Price {
                field: PriceField::Close,
            },
            op: Comparator::Lt,
            rhs: ValueSource::Constant {
                value: Decimal::ZERO,
            },
        }
    }

    fn signal_on_high_close() -> Condition {
        Condition::Compare {
            lhs: ValueSource::Price {
                field: PriceField::Close,
            },
            op: Comparator::Gt,
            rhs: ValueSource::Constant { value: d(150) },
        }
    }

    fn stop() -> ExitRule {
        ExitRule::StopLoss {
            distance_pct: SweepableValue::Fixed(rate(5, 2)),
        }
    }

    fn tp(target_r: i64) -> ExitRule {
        ExitRule::TakeProfit {
            target_r: SweepableValue::Fixed(d(target_r)),
        }
    }

    fn compiled(entry: Condition, exits: Vec<ExitRule>) -> CompiledStrategy {
        let dsl = StrategyDsl {
            schema_version: SchemaVersion::CURRENT,
            name: "test strategy".to_owned(),
            direction: Direction::Long,
            entry,
            filters: vec![],
            exits,
            risk: RiskParams {
                risk_per_trade_pct: SweepableValue::Fixed(rate(1, 2)),
                max_leverage: SweepableValue::Fixed(d(3)),
            },
        };
        compile(&validate(&dsl).unwrap()).unwrap()
    }

    fn base_strategy() -> CompiledStrategy {
        compiled(price_entry(), vec![stop(), tp(10)])
    }

    #[test]
    fn entry_fills_at_next_bar_open_not_signal_bar_close() {
        let primary = series(vec![
            candle(0, 100, 101, 99, 100),
            candle(1, 110, 112, 108, 111),
            candle(2, 120, 121, 119, 120),
        ]);
        let result = run_backtest(&base_strategy(), &primary, None, &config()).unwrap();

        assert_eq!(result.trades.len(), 1);
        let trade = &result.trades[0];
        assert_eq!(trade.entry_signal_time, primary.candles[1].close_time);
        assert_eq!(trade.entry_fill_time, primary.candles[2].open_time);
        assert_eq!(trade.entry_price, d(120));
    }

    #[test]
    fn pure_price_strategy_does_not_enter_on_bar_zero() {
        let primary = series(vec![
            candle(0, 100, 101, 99, 100),
            candle(1, 100, 101, 99, 100),
        ]);
        let result = run_backtest(&base_strategy(), &primary, None, &config()).unwrap();

        assert!(result.trades.is_empty());
    }

    #[test]
    fn stop_wins_when_entry_bar_straddles_stop_and_take_profit() {
        let primary = series(vec![
            candle(0, 100, 101, 99, 100),
            candle(1, 100, 101, 99, 100),
            candle(2, 100, 120, 90, 100),
        ]);
        let result = run_backtest(&base_strategy(), &primary, None, &config()).unwrap();

        assert_eq!(result.trades[0].exit_reason, ExitReason::StopLoss);
        assert_eq!(result.trades[0].exit_price, d(95));
    }

    #[test]
    fn held_position_accrues_one_positive_long_funding_payment() {
        let primary = series(vec![
            candle(0, 100, 101, 99, 100),
            candle(1, 100, 101, 99, 100),
            candle(2, 100, 101, 99, 100),
            funding_candle(3, 100, 101, 99, 100),
            candle(4, 100, 101, 99, 100),
        ]);
        let result = run_backtest(&base_strategy(), &primary, None, &config()).unwrap();

        assert_eq!(result.trades.len(), 1);
        assert_eq!(result.trades[0].funding_total, d(-2));
        assert_eq!(result.funding_total, d(-2));
    }

    #[test]
    fn entry_at_funding_bar_open_excludes_that_boundary() {
        let primary = series(vec![
            candle(0, 100, 101, 99, 100),
            candle(1, 100, 101, 99, 100),
            funding_candle(2, 100, 101, 99, 100),
        ]);
        let result = run_backtest(&base_strategy(), &primary, None, &config()).unwrap();

        assert_eq!(result.trades.len(), 1);
        assert_eq!(result.trades[0].funding_total, Decimal::ZERO);
    }

    #[test]
    fn intra_bar_exit_on_funding_bar_includes_that_boundary() {
        let primary = series(vec![
            candle(0, 100, 101, 99, 100),
            candle(1, 100, 101, 99, 100),
            candle(2, 100, 101, 99, 100),
            funding_candle(3, 100, 101, 94, 100),
        ]);
        let result = run_backtest(&base_strategy(), &primary, None, &config()).unwrap();

        assert_eq!(result.trades.len(), 1);
        assert_eq!(result.trades[0].exit_reason, ExitReason::StopLoss);
        assert_eq!(result.trades[0].funding_total, d(-2));
    }

    #[test]
    fn stopless_strategy_errors_before_iteration() {
        let primary = series(vec![candle(0, 100, 101, 99, 100)]);
        let strategy = compiled(
            price_entry(),
            vec![ExitRule::SignalExit {
                condition: never_signal(),
            }],
        );

        let err = run_backtest(&strategy, &primary, None, &config()).unwrap_err();
        assert_eq!(err, BacktestError::NoStopLoss);
    }

    #[test]
    fn trailing_and_time_exits_are_rejected() {
        let primary = series(vec![candle(0, 100, 101, 99, 100)]);
        let trailing = compiled(
            price_entry(),
            vec![
                stop(),
                ExitRule::TrailingStop {
                    trail_pct: SweepableValue::Fixed(rate(5, 2)),
                },
            ],
        );
        let time = compiled(
            price_entry(),
            vec![
                stop(),
                ExitRule::TimeStop {
                    max_bars: SweepableValue::Fixed(5),
                },
            ],
        );

        assert!(matches!(
            run_backtest(&trailing, &primary, None, &config()).unwrap_err(),
            BacktestError::UnsupportedExit(_)
        ));
        assert!(matches!(
            run_backtest(&time, &primary, None, &config()).unwrap_err(),
            BacktestError::UnsupportedExit(_)
        ));
    }

    #[test]
    fn open_position_at_series_end_is_force_closed() {
        let primary = series(vec![
            candle(0, 100, 101, 99, 100),
            candle(1, 100, 101, 99, 100),
            candle(2, 100, 101, 99, 103),
        ]);
        let result = run_backtest(&base_strategy(), &primary, None, &config()).unwrap();

        assert_eq!(result.trades.len(), 1);
        assert_eq!(result.trades[0].exit_reason, ExitReason::EndOfData);
        assert_eq!(result.trades[0].exit_price, d(103));
    }

    #[test]
    fn pending_entry_on_final_bar_is_dropped() {
        let primary = series(vec![
            candle(0, 100, 101, 99, 100),
            candle(1, 100, 101, 99, 100),
        ]);
        let result = run_backtest(&base_strategy(), &primary, None, &config()).unwrap();

        assert!(result.trades.is_empty());
    }

    #[test]
    fn signal_exit_fills_at_next_bar_open_when_no_price_exit_preempts() {
        let strategy = compiled(
            price_entry(),
            vec![
                stop(),
                tp(10),
                ExitRule::SignalExit {
                    condition: signal_on_high_close(),
                },
            ],
        );
        let primary = series(vec![
            candle(0, 100, 101, 99, 100),
            candle(1, 100, 101, 99, 100),
            candle(2, 100, 101, 99, 160),
            candle(3, 105, 106, 104, 105),
        ]);
        let result = run_backtest(&strategy, &primary, None, &config()).unwrap();

        assert_eq!(result.trades.len(), 1);
        assert_eq!(result.trades[0].exit_reason, ExitReason::Signal);
        assert_eq!(
            result.trades[0].exit_signal_time,
            primary.candles[2].close_time
        );
        assert_eq!(
            result.trades[0].exit_fill_time,
            primary.candles[3].open_time
        );
        assert_eq!(result.trades[0].exit_price, d(105));
    }
}
