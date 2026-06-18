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
//! **Load path (G7):** `--dsl <path>` → read file → `serde_json` into
//! [`StrategyDsl`] → [`validate`] → [`compile`] → [`run_backtest`]. The DB-load
//! path (`--version` from a `StrategyRepository`) is **deferred** (this slice is
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
    BacktestResult, CandleSeries, Direction, ExitReason, Pair, StrategyDsl, Timeframe, Trade,
    compile, validate,
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
/// Pipeline (G7): read `--dsl` → `serde_json::from_str::<StrategyDsl>` →
/// [`validate`] → [`compile`] → load the primary (+ optional HTF) `CandleSeries`
/// from the [`CandleStore`] → [`run_backtest`] → render.
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

/// Read + parse + validate + compile a DSL JSON file into a `CompiledStrategy`.
fn load_compiled_strategy(
    path: &std::path::Path,
) -> anyhow::Result<crate::domain::CompiledStrategy> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("read DSL file {}: {e}", path.display()))?;
    let dsl: StrategyDsl = serde_json::from_str(&raw)
        .map_err(|e| anyhow::anyhow!("parse DSL file {}: {e}", path.display()))?;
    let validated =
        validate(&dsl).map_err(|e| anyhow::anyhow!("strategy failed validation: {e}"))?;
    compile(&validated).map_err(|e| anyhow::anyhow!("compile strategy: {e}"))
}

/// Load the HEAD snapshot for `(pair, tf)` from the store, erroring clearly when
/// no snapshot exists.
fn load_series(store: &CandleStore, pair: &Pair, tf: Timeframe) -> anyhow::Result<CandleSeries> {
    let head = store.read_head(pair, tf)?.ok_or_else(|| {
        anyhow::anyhow!(
            "no HEAD snapshot for {pair} {} in the candle store",
            tf.binance_interval()
        )
    })?;
    store
        .read_snapshot(pair, tf, &head)
        .map_err(|e| anyhow::anyhow!("read snapshot for {pair} {}: {e}", tf.binance_interval()))
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

/// The summary footer: trade count + gross/net P&L + run-level cost totals. Gross
/// P&L = net P&L + every cost (the "costs are deducted" readout, FR-6).
fn render_footer(result: &BacktestResult) -> String {
    let gross = result.net_pnl + result.fees_total + result.funding_total + result.slippage_total;
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
    use super::{TRADE_HEADER, dec, render_footer, render_trade_row};
    use crate::domain::{BacktestResult, Direction, ExitReason, Fill, Trade, TradeSource};
    use rust_decimal::Decimal;

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
        // gross = net + fees + funding + slippage = 1484 + 12 + 1 + 3 = 1500.
        assert!(footer.contains("gross_pnl=1500"), "footer was: {footer}");
        assert!(footer.contains("fees_total=12"));
    }

    #[test]
    fn dec_normalizes_trailing_zeros() {
        assert_eq!(dec(Decimal::new(2_00, 2)), "2");
        assert_eq!(dec(Decimal::new(5, 1)), "0.5");
    }
}
