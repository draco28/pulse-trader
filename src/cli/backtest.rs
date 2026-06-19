//! `pulse backtest` — the v1 CLI proof-of-concept demo surface (VS-1.2.1
//! work-1.04, FR-5/FR-6).
//!
//! Loads a compiled strategy from a **DSL JSON file** (G7), loads candles offline
//! from a [`CandleStore`], runs the deterministic backtest engine (1.03), and
//! renders a human-readable **trade log** (one row per trade: entry/exit times,
//! direction, prices, qty, fees, funding, slippage, P&L, realized R) plus a
//! cost-breakdown footer (trade count, gross/net P&L, total fees/funding/
//! slippage). This realizes the **user demo criterion**: "read the resulting
//! trade log and confirm fees/funding/slippage are deducted."
//!
//! **Load path (G7):** `--dsl <path>` → read file → version-safe [`Migrator`]
//! load → [`validate`] → [`compile`] → [`run_backtest`]. The DB-load path
//! (`--version` from a `StrategyRepository`) is **deferred** (this slice is
//! persistence-free).
//!
//! **C5 flag defaults** match [`BacktestConfig::default`]: `--fee-bps 4`
//! (0.04% Binance USDⓂ taker), `--slippage-bps 1`, `--equity 10000` (USDT).
//!
//! **Error surfacing (spec §3):** a `NoStopLoss` engine error, a missing
//! snapshot, or a bad `--dsl` surface as typed `anyhow` errors with a clear
//! message + non-zero exit (mapped to `ExitCode::FAILURE` by the binary shim),
//! never a panic.
//!
//! **Rendering** follows the `pulse indicators` tab-separated + summary-footer
//! style (`src/cli/indicators.rs`) for consistency.

use std::path::PathBuf;

use rust_decimal::Decimal;

use crate::adapters::backtest::{BacktestConfig, run_backtest};
use crate::adapters::store::CandleStore;
use crate::domain::{
    BacktestResult, CandleSeries, Direction, ExitReason, Migrator, Pair, StrategyDsl, Timeframe,
    Trade, compile, validate,
};

use super::parse_one_tf;

/// `pulse backtest` arguments — the DSL path, the pair/timeframe(s), the candle
/// store, and the C5 cost knobs (`--fee-bps` / `--slippage-bps` / `--equity`).
/// The required flags are `--dsl`, `--pair`, and `--tf`; `--htf`, `--store`, and
/// `--json` are optional. See each field for its meaning + default.
#[derive(Debug, clap::Args)]
pub struct BacktestArgs {
    /// Path to the strategy DSL JSON document to backtest (G7 load path).
    #[arg(long)]
    pub dsl: PathBuf,
    /// The trading pair symbol (e.g. `BTCUSDT`).
    #[arg(long)]
    pub pair: String,
    /// Primary candle timeframe (`M15`/`15m` or `H4`/`4h`).
    #[arg(long)]
    pub tf: String,
    /// Optional higher timeframe for MTF alignment (`M15`/`H4`). Omitted ⇒
    /// single-timeframe backtest.
    #[arg(long)]
    pub htf: Option<String>,
    /// `CandleStore` root directory. Defaults to the platform Application Support
    /// data dir; the demo points it at the fixture store.
    #[arg(long)]
    pub store: Option<PathBuf>,
    /// Starting account equity (quote currency). C5 default matches
    /// `BacktestConfig::default`.
    #[arg(long, default_value = "10000")]
    pub equity: Decimal,
    /// Taker fee in basis points. C5 default `4` = 0.04% Binance USDⓂ taker.
    #[arg(long = "fee-bps", default_value = "4")]
    pub fee_bps: Decimal,
    /// Adverse-fill slippage in basis points. C5 default `1`.
    #[arg(long = "slippage-bps", default_value = "1")]
    pub slippage_bps: Decimal,
    /// Emit the result as a structured JSON object instead of the tab-separated
    /// trade log.
    #[arg(long)]
    pub json: bool,
}

/// Run a backtest over a DSL file + a local candle store and render the result.
///
/// Pipeline (G7): read `--dsl` → version-safe [`Migrator`] load → [`validate`] →
/// [`compile`] → load the primary (+ optional HTF) `CandleSeries` from the
/// [`CandleStore`] → [`run_backtest`] → render.
///
/// # Errors
///
/// Returns an [`anyhow::Error`] on: an unreadable / malformed `--dsl` file, a DSL
/// that fails semantic validation, a compile error, an invalid pair/timeframe,
/// a missing candle snapshot, or an engine error (e.g. `NoStopLoss`). Every path
/// surfaces a clear message + non-zero exit; nothing panics.
pub fn run_backtest_cli(args: &BacktestArgs) -> anyhow::Result<()> {
    let pair = Pair::parse(args.pair.clone())
        .map_err(|e| anyhow::anyhow!("invalid pair argument: {e}"))?;
    let tf = parse_one_tf(&args.tf)?;
    let htf = match &args.htf {
        Some(raw) => Some(parse_one_tf(raw)?),
        None => None,
    };

    let compiled = load_compiled_strategy(&args.dsl)?;

    let store = match &args.store {
        Some(dir) => CandleStore::with_base_dir(dir.clone()),
        None => CandleStore::with_default_base_dir()
            .map_err(|e| anyhow::anyhow!("resolve default candle-store dir: {e}"))?,
    };

    let primary = load_series(&store, &pair, tf)?;
    let htf_series = match htf {
        Some(htf_tf) => Some(load_series(&store, &pair, htf_tf)?),
        None => None,
    };

    validate_cost_knobs(args.equity, args.fee_bps, args.slippage_bps)?;
    let config = BacktestConfig {
        starting_equity: args.equity,
        taker_fee_bps: args.fee_bps,
        slippage_bps: args.slippage_bps,
    };

    let result = run_backtest(&compiled, &primary, htf_series.as_ref(), &config)
        .map_err(|e| anyhow::anyhow!("backtest failed: {e}"))?;

    if args.json {
        render_json(&result)?;
    } else {
        render_human(&result);
    }
    Ok(())
}

/// Reject cost/equity knobs that would feed the sizing + fill math nonsensical
/// inputs: equity must be strictly positive (it is the sizing denominator), and
/// the fee / slippage rates must be non-negative (a negative rate would invent a
/// favorable "adverse" fill or a fee rebate this slice does not model).
fn validate_cost_knobs(
    equity: Decimal,
    fee_bps: Decimal,
    slippage_bps: Decimal,
) -> anyhow::Result<()> {
    if equity <= Decimal::ZERO {
        anyhow::bail!("--equity must be positive (got {equity})");
    }
    if fee_bps < Decimal::ZERO {
        anyhow::bail!("--fee-bps must be non-negative (got {fee_bps})");
    }
    if slippage_bps < Decimal::ZERO {
        anyhow::bail!("--slippage-bps must be non-negative (got {slippage_bps})");
    }
    Ok(())
}

/// Parse a DSL document through the **version-safe migrator** (FR-4), matching the
/// golden fixture's read path. A future `schema_version` (`> CURRENT`) is refused
/// rather than silently reinterpreted under current semantics; an older version is
/// migrated forward. (The previous raw `serde_json::from_str::<StrategyDsl>` would
/// have accepted any structurally-matching future document.)
fn parse_dsl(raw: &str) -> anyhow::Result<StrategyDsl> {
    let loaded = Migrator::v1()
        .load(raw)
        .map_err(|e| anyhow::anyhow!("load DSL (version-safe): {e}"))?;
    Ok(loaded.dsl)
}

/// Read + parse + validate + compile a DSL JSON file into a `CompiledStrategy`.
fn load_compiled_strategy(
    path: &std::path::Path,
) -> anyhow::Result<crate::domain::CompiledStrategy> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("read DSL file {}: {e}", path.display()))?;
    let dsl =
        parse_dsl(&raw).map_err(|e| anyhow::anyhow!("parse DSL file {}: {e}", path.display()))?;
    let validated =
        validate(&dsl).map_err(|e| anyhow::anyhow!("strategy failed validation: {e}"))?;
    compile(&validated).map_err(|e| anyhow::anyhow!("compile strategy: {e}"))
}

/// Refuse a series the backtester cannot interpret correctly: structural
/// corruption (unsorted / duplicate `open_time`) or any spacing **gap**. The
/// engine + indicator stream assume a gap-free, contiguous series (it does not
/// detect or fill holes), so a gapped snapshot would silently skew signals,
/// holding periods, and funding accrual — fail fast instead.
fn reject_if_gapped(series: &CandleSeries) -> anyhow::Result<()> {
    let gaps = series.validate().map_err(|e| {
        anyhow::anyhow!(
            "candle series for {} {} is structurally invalid: {e}",
            series.pair,
            series.timeframe.binance_interval()
        )
    })?;
    if let Some(first) = gaps.first() {
        anyhow::bail!(
            "candle series for {} {} has {} spacing gap(s) (first: expected open_time {}, found {}); \
             the backtester requires a gap-free series — re-fetch the snapshot",
            series.pair,
            series.timeframe.binance_interval(),
            gaps.len(),
            first.expected,
            first.found
        );
    }
    Ok(())
}

/// Load the HEAD snapshot for `(pair, tf)` from the store, erroring clearly when
/// no snapshot exists or when the series is not gap-free.
fn load_series(store: &CandleStore, pair: &Pair, tf: Timeframe) -> anyhow::Result<CandleSeries> {
    let head = store.read_head(pair, tf)?.ok_or_else(|| {
        anyhow::anyhow!(
            "no HEAD snapshot for {pair} {} in the candle store",
            tf.binance_interval()
        )
    })?;
    let series = store
        .read_snapshot(pair, tf, &head)
        .map_err(|e| anyhow::anyhow!("read snapshot for {pair} {}: {e}", tf.binance_interval()))?;
    reject_if_gapped(&series)?;
    Ok(series)
}

/// The tab-separated trade-log header (names every cost column the demo reads).
const TRADE_HEADER: &str = "entry_time\texit_time\tdir\tentry_price\texit_price\tqty\tfees\tfunding\tslippage\tpnl\tR\texit_reason";

/// Render the human-readable trade log + the cost-breakdown footer.
fn render_human(result: &BacktestResult) {
    println!("{TRADE_HEADER}");
    for trade in &result.trades {
        println!("{}", render_trade_row(trade));
    }
    println!("{}", render_footer(result));
}

/// One tab-separated trade row. Decimals are normalized so trailing zeros do not
/// clutter the readout (matching the `indicators` renderer convention).
fn render_trade_row(trade: &Trade) -> String {
    [
        trade.entry_fill_time.to_string(),
        trade.exit_fill_time.to_string(),
        direction_label(trade.direction).to_owned(),
        dec(trade.entry_price),
        dec(trade.exit_price),
        dec(trade.qty),
        dec(trade.fees_total),
        dec(trade.funding_total),
        dec(trade.slippage_total),
        dec(trade.realized_pnl),
        dec(trade.realized_r),
        exit_reason_label(trade.exit_reason).to_owned(),
    ]
    .join("\t")
}

/// The summary footer: trade count + gross/net P&L + run-level cost totals (the
/// "costs are deducted" readout, FR-6). `gross` is the P&L before costs; the engine
/// nets it as `net = gross_slipped + funding − fees` with slippage embedded in the
/// slipped fills, so inverting gives `gross = net + fees − funding + slippage`.
/// `funding_total` is SIGNED (negative when the position pays funding), so it is
/// SUBTRACTED here — adding it would move gross the wrong way by `2 × |funding|`.
fn render_footer(result: &BacktestResult) -> String {
    let gross = result.net_pnl + result.fees_total - result.funding_total + result.slippage_total;
    [
        "summary".to_owned(),
        format!("trades={}", result.trades.len()),
        format!("gross_pnl={}", dec(gross)),
        format!("net_pnl={}", dec(result.net_pnl)),
        format!("fees_total={}", dec(result.fees_total)),
        format!("funding_total={}", dec(result.funding_total)),
        format!("slippage_total={}", dec(result.slippage_total)),
    ]
    .join("\t")
}

/// Emit the full [`BacktestResult`] as a structured JSON object (`--json`).
fn render_json(result: &BacktestResult) -> anyhow::Result<()> {
    let line = serde_json::to_string_pretty(result)
        .map_err(|e| anyhow::anyhow!("serialize backtest result: {e}"))?;
    println!("{line}");
    Ok(())
}

/// A compact, trailing-zero-normalized decimal rendering.
fn dec(value: Decimal) -> String {
    value.normalize().to_string()
}

/// Human-readable trade-side label.
fn direction_label(direction: Direction) -> &'static str {
    match direction {
        Direction::Long => "long",
        Direction::Short => "short",
    }
}

/// Human-readable exit-reason label.
fn exit_reason_label(reason: ExitReason) -> &'static str {
    match reason {
        ExitReason::StopLoss => "stop_loss",
        ExitReason::TakeProfit => "take_profit",
        ExitReason::Signal => "signal",
        ExitReason::EndOfData => "end_of_data",
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{
        TRADE_HEADER, dec, parse_dsl, reject_if_gapped, render_footer, render_trade_row,
        validate_cost_knobs,
    };
    use crate::domain::{
        BacktestResult, Candle, CandleSeries, Comparator, Condition, DataVersion, Direction,
        ExitReason, ExitRule, Fill, Pair, PriceField, RiskParams, SchemaVersion, StrategyDsl,
        SweepableValue, Timeframe, Trade, TradeSource, ValueSource,
    };
    use rust_decimal::Decimal;

    /// A valid, current-schema price-only strategy (no indicators) for load tests.
    fn sample_dsl() -> StrategyDsl {
        StrategyDsl {
            schema_version: SchemaVersion::CURRENT,
            name: "t".to_owned(),
            direction: Direction::Long,
            entry: Condition::Compare {
                lhs: ValueSource::Price {
                    field: PriceField::Close,
                },
                op: Comparator::Gt,
                rhs: ValueSource::Constant {
                    value: Decimal::ZERO,
                },
            },
            filters: vec![],
            exits: vec![
                ExitRule::StopLoss {
                    distance_pct: SweepableValue::Fixed(Decimal::new(5, 2)),
                },
                ExitRule::TakeProfit {
                    target_r: SweepableValue::Fixed(Decimal::new(2, 0)),
                },
            ],
            risk: RiskParams {
                risk_per_trade_pct: SweepableValue::Fixed(Decimal::new(1, 2)),
                max_leverage: SweepableValue::Fixed(Decimal::new(3, 0)),
            },
        }
    }

    fn m15_candle(open_time: i64) -> Candle {
        Candle {
            open_time,
            close_time: open_time + 900_000 - 1,
            open: Decimal::new(100, 0),
            high: Decimal::new(101, 0),
            low: Decimal::new(99, 0),
            close: Decimal::new(100, 0),
            volume: Decimal::ONE,
            funding_rate: None,
        }
    }

    fn m15_series(open_times: &[i64]) -> CandleSeries {
        CandleSeries {
            pair: Pair::new("BTCUSDT"),
            timeframe: Timeframe::M15,
            version: DataVersion::new("test"),
            candles: open_times.iter().map(|t| m15_candle(*t)).collect(),
        }
    }

    fn sample_trade() -> Trade {
        Trade {
            direction: Direction::Long,
            qty: Decimal::new(5, 1), // 0.5
            entry_price: Decimal::new(30_000, 0),
            exit_price: Decimal::new(33_000, 0),
            entry_signal_time: 1,
            entry_fill_time: 2,
            exit_signal_time: 3,
            exit_fill_time: 4,
            fills: vec![Fill {
                price: Decimal::new(30_000, 0),
                qty: Decimal::new(5, 1),
                time_ms: 2,
                fee: Decimal::new(6, 0),
            }],
            fees_total: Decimal::new(12, 0),
            funding_total: Decimal::new(1, 0),
            slippage_total: Decimal::new(3, 0),
            realized_pnl: Decimal::new(1_484, 0),
            realized_r: Decimal::new(2, 0),
            exit_reason: ExitReason::TakeProfit,
            source: TradeSource::Backtest,
        }
    }

    #[test]
    fn header_names_every_cost_column() {
        assert!(TRADE_HEADER.contains("fees"));
        assert!(TRADE_HEADER.contains("funding"));
        assert!(TRADE_HEADER.contains("slippage"));
        assert!(TRADE_HEADER.contains("pnl"));
    }

    #[test]
    fn trade_row_is_tab_separated_with_all_fields() {
        let row = render_trade_row(&sample_trade());
        let cells = row.split('\t').collect::<Vec<_>>();
        // Same arity as the header (12 columns).
        assert_eq!(cells.len(), TRADE_HEADER.split('\t').count());
        assert_eq!(cells[2], "long");
        assert_eq!(cells[6], "12"); // fees
        assert_eq!(cells[7], "1"); // funding
        assert_eq!(cells[8], "3"); // slippage
        assert_eq!(cells[11], "take_profit");
    }

    #[test]
    fn footer_reports_counts_net_and_gross() {
        let result = BacktestResult {
            trades: vec![sample_trade()],
            net_pnl: Decimal::new(1_484, 0),
            fees_total: Decimal::new(12, 0),
            funding_total: Decimal::new(1, 0),
            slippage_total: Decimal::new(3, 0),
        };
        let footer = render_footer(&result);
        assert!(footer.contains("trades=1"));
        assert!(footer.contains("net_pnl=1484"));
        // funding is SIGNED (engine: net = gross_slipped + funding − fees), so gross
        // before costs = net + fees − funding + slippage = 1484 + 12 − 1 + 3 = 1498.
        assert!(footer.contains("gross_pnl=1498"), "footer was: {footer}");
        assert!(footer.contains("fees_total=12"));
    }

    #[test]
    fn dec_normalizes_trailing_zeros() {
        assert_eq!(dec(Decimal::new(2_00, 2)), "2");
        assert_eq!(dec(Decimal::new(5, 1)), "0.5");
    }

    #[test]
    fn cost_knobs_reject_nonpositive_equity_and_negative_costs() {
        let ok_eq = Decimal::new(10_000, 0);
        // Valid (zero fee/slippage allowed; equity must be strictly positive).
        assert!(validate_cost_knobs(ok_eq, Decimal::new(4, 0), Decimal::ONE).is_ok());
        assert!(validate_cost_knobs(ok_eq, Decimal::ZERO, Decimal::ZERO).is_ok());
        // Invalid.
        assert!(validate_cost_knobs(Decimal::ZERO, Decimal::new(4, 0), Decimal::ONE).is_err());
        assert!(
            validate_cost_knobs(Decimal::new(-1, 0), Decimal::new(4, 0), Decimal::ONE).is_err()
        );
        assert!(validate_cost_knobs(ok_eq, Decimal::new(-1, 0), Decimal::ONE).is_err());
        assert!(validate_cost_knobs(ok_eq, Decimal::new(4, 0), Decimal::new(-5, 0)).is_err());
    }

    #[test]
    fn gapped_series_is_rejected_clean_series_passes() {
        let step = 900_000; // M15 duration in ms
        let clean = m15_series(&[0, step, 2 * step]);
        assert!(reject_if_gapped(&clean).is_ok());

        // Skips the 2*step bar → a one-interval hole the backtester must refuse.
        let gapped = m15_series(&[0, step, 3 * step]);
        assert!(reject_if_gapped(&gapped).is_err());
    }

    #[test]
    fn parse_dsl_accepts_current_and_refuses_future_schema_version() {
        let json = serde_json::to_string(&sample_dsl()).unwrap();
        // A current-schema document loads through the version-safe migrator.
        assert!(parse_dsl(&json).is_ok());

        // A future major (> CURRENT) must be REFUSED, not silently reinterpreted
        // under current semantics (the raw `serde_json::from_str` it replaced would
        // have accepted it).
        let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
        v["schema_version"] = serde_json::Value::String("2.0.0".to_owned());
        let future = serde_json::to_string(&v).unwrap();
        assert!(
            parse_dsl(&future).is_err(),
            "a future schema_version must be refused by the version-safe loader"
        );
    }
}
