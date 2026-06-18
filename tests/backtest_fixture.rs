//! End-to-end golden backtest over the 1-month BTCUSDT fixture (VS-1.2.1
//! work-1.05; FR-5 / FR-6, BACKLOG-4).
//!
//! This is the slice's **regression anchor**: it loads the canonical
//! known-strategy DSL (`tests/fixtures/strategies/rsi-oversold-long.json` — the
//! demo-1/demo-2 "RSI(14) < 30, 5% stop, 2R take-profit, 1% risk" long), drives
//! `run_backtest` (the library API, NOT the CLI — keeps this independent of
//! work-1.04) over the real, gap-free 1-month M15 candle store
//! (`tests/fixtures/btcusdt-1m-store/`), and asserts the **frozen golden** trade
//! count + net P&L plus the non-vacuity invariants.
//!
//! ## Golden regeneration (deliberate engine change ⇒ reviewed golden diff)
//!
//! The golden constants below were captured from the first calibrated run. To
//! regenerate after an *intentional* engine change, run the print probe and copy
//! the emitted `GOLDEN_TRADE_COUNT` / `GOLDEN_NET_PNL` lines into the constants:
//!
//! ```text
//! cargo test --test backtest_fixture print_golden_for_regeneration -- --ignored --nocapture
//! ```
//!
//! A silent drift fails `golden_backtest_reproduces_frozen_trade_count_and_pnl`
//! loudly; a deliberate change is a one-line, reviewed golden diff.
//!
//! ## Calibration (C2 — done before freezing)
//!
//! A param grid was swept on this exact fixture; the canonical demo params
//! (RSI period 14, oversold 30, 5% stop, 2R TP) yield 6 completed trades — all 6
//! holding across an 8h funding boundary — so the auto-demo is non-vacuous
//! (>= 3 trades, >= 1 funding crossing, total funding != 0). No strategy swap was
//! needed; the canonical RSI-oversold-long strategy satisfies C2 directly.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use pulse::{
    BacktestConfig, BacktestResult, CandleSeries, CandleStore, Migrator, Pair, Timeframe, Trade,
    compile, run_backtest, validate,
};
use rust_decimal::Decimal;

/// Frozen golden: the exact number of completed trades the canonical strategy
/// produces on the 1-month BTCUSDT fixture. See the module header for
/// regeneration. (RSI(14) < 30 long, 5% stop, 2R take-profit, 1% risk.)
const GOLDEN_TRADE_COUNT: usize = 6;

/// Frozen golden: the exact net P&L (quote currency, already net of
/// fees/funding/slippage) across the run. Exact `Decimal` — a silent engine
/// drift changes this and fails the golden assertion. See the module header for
/// regeneration.
const GOLDEN_NET_PNL: &str = "142.96792368029048810681316863";

/// The RSI lookback period the canonical fixture uses. The streaming indicator
/// engine warms RSI(period) at candle index `period + 1` (the first index where
/// `is_warm()` becomes true; matches the CLI `rsi:14_first_row=15` readout), so
/// the first entry CANNOT fill before then (G6 real-data readiness gate).
const RSI_PERIOD: usize = 14;

/// Load the primary M15 candle series from the committed offline fixture store
/// (no network / LLM / SQLite): `with_base_dir` -> `read_head` -> `read_snapshot`.
fn load_primary() -> CandleSeries {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/btcusdt-1m-store");
    let store = CandleStore::with_base_dir(base);
    let pair = Pair::new("BTCUSDT");
    let head = store
        .read_head(&pair, Timeframe::M15)
        .expect("read M15 HEAD")
        .expect("M15 HEAD present in fixture store");
    store
        .read_snapshot(&pair, Timeframe::M15, &head)
        .expect("read M15 snapshot")
}

/// Compile the canonical known-strategy DSL fixture through the full read-path
/// the engine consumes: `Migrator::v1().load` (FR-4 version-safe) -> `validate`
/// (FR-3) -> `compile` -> the executable evaluator tree.
fn run_golden(primary: &CandleSeries) -> BacktestResult {
    let json = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/strategies/rsi-oversold-long.json"),
    )
    .expect("read strategy fixture json");
    let loaded = Migrator::v1().load(&json).expect("load (migrate) strategy");
    let validated = validate(&loaded.dsl).expect("strategy validates");
    let compiled = compile(&validated).expect("strategy compiles");
    // Single-TF M15: no HTF feed (htf = None). Default cost model: 4bps taker
    // fee, 1bp slippage, 10_000 starting equity.
    run_backtest(&compiled, primary, None, &BacktestConfig::default())
        .expect("backtest runs over the fixture")
}

/// Whether a trade's hold crossed at least one 8h funding boundary: a fixture
/// candle that carries a `funding_rate` and whose `open_time` lies strictly
/// after the entry fill and at or before the exit fill (the exact window the
/// engine accrues funding over).
fn crosses_funding_boundary(primary: &CandleSeries, trade: &Trade) -> bool {
    primary.candles.iter().any(|c| {
        c.funding_rate.is_some()
            && c.open_time > trade.entry_fill_time
            && c.open_time <= trade.exit_fill_time
    })
}

/// The candle index at which the first trade's entry filled.
fn first_entry_fill_index(primary: &CandleSeries, result: &BacktestResult) -> usize {
    primary
        .candles
        .iter()
        .position(|c| c.open_time == result.trades[0].entry_fill_time)
        .expect("first entry fill resolves to a candle index")
}

/// The slice regression anchor: the canonical strategy on the 1-month fixture
/// reproduces the **frozen golden** trade count and net P&L exactly. A silent
/// engine drift fails this loudly; a deliberate change is a reviewed golden diff
/// (see the module-header regeneration note).
#[test]
fn golden_backtest_reproduces_frozen_trade_count_and_pnl() {
    let primary = load_primary();
    let result = run_golden(&primary);

    assert_eq!(
        result.trades.len(),
        GOLDEN_TRADE_COUNT,
        "golden trade_count drifted: expected {GOLDEN_TRADE_COUNT}, got {} \
         (regenerate the golden only on a deliberate engine change)",
        result.trades.len()
    );
    let expected_net_pnl = Decimal::from_str_exact(GOLDEN_NET_PNL).expect("GOLDEN_NET_PNL parses");
    assert_eq!(
        result.net_pnl, expected_net_pnl,
        "golden net P&L drifted: expected {expected_net_pnl}, got {}",
        result.net_pnl
    );
}

/// C2 non-vacuity: the auto-demo would be meaningless if the chosen params
/// produced 0 (or only intra-bar) trades, or never held across a funding
/// boundary. Assert >= 3 completed trades, >= 1 trade crossing an 8h funding
/// boundary, and total funding != 0 — so a degenerate "0 trades / no funding"
/// golden fails loudly rather than passing as "expected".
#[test]
fn golden_run_is_non_vacuous_at_least_three_trades_and_funding_applied() {
    let primary = load_primary();
    let result = run_golden(&primary);

    assert!(
        result.trades.len() >= 3,
        "expected at least 3 completed trades (non-vacuous demo), got {}",
        result.trades.len()
    );

    let funding_crossings = result
        .trades
        .iter()
        .filter(|t| crosses_funding_boundary(&primary, t))
        .count();
    assert!(
        funding_crossings >= 1,
        "expected at least 1 trade whose hold crosses an 8h funding boundary, got {funding_crossings}"
    );

    assert_ne!(
        result.funding_total,
        Decimal::ZERO,
        "total funding must be non-zero (funding is exercised end-to-end)"
    );
}

/// G6 real-data readiness: the first trade's entry index must be at or beyond the
/// RSI warmup (no pre-warmup / bar-0 entry on real data). RSI(period) is not
/// `is_warm()` until candle index `period + 1`, and the loop additionally
/// requires `bar.index > 0`, so the earliest possible fill is `period + 1`.
#[test]
fn first_entry_respects_rsi_warmup() {
    let primary = load_primary();
    let result = run_golden(&primary);

    let first_idx = first_entry_fill_index(&primary, &result);
    // The earliest possible fill is `RSI_PERIOD + 1` (warmup); `> RSI_PERIOD` is
    // the same bound for `usize` and is clippy-`int_plus_one`-clean.
    assert!(
        first_idx > RSI_PERIOD,
        "first entry filled at candle index {first_idx}, before the RSI({RSI_PERIOD}) warmup \
         (must be > {RSI_PERIOD}, i.e. >= {})",
        RSI_PERIOD + 1
    );
}

/// Regeneration probe (ignored by default): prints the golden trade count, exact
/// net P&L, funding-crossing count, and cost roll-ups. Run with `--ignored
/// --nocapture` after a deliberate engine change; copy the emitted constants up.
#[test]
#[ignore = "regeneration probe; run with --ignored --nocapture to refresh the golden"]
fn print_golden_for_regeneration() {
    let primary = load_primary();
    let result = run_golden(&primary);
    let funding_crossings = result
        .trades
        .iter()
        .filter(|t| crosses_funding_boundary(&primary, t))
        .count();

    eprintln!("GOLDEN_TRADE_COUNT = {}", result.trades.len());
    eprintln!("GOLDEN_NET_PNL = \"{}\"", result.net_pnl);
    eprintln!("funding_crossings = {funding_crossings}");
    eprintln!("funding_total = {}", result.funding_total);
    eprintln!("fees_total = {}", result.fees_total);
    eprintln!("slippage_total = {}", result.slippage_total);
    eprintln!(
        "first_entry_fill_index = {}",
        first_entry_fill_index(&primary, &result)
    );
    eprintln!(
        "fixture: {} M15 candles, {} funding-boundary candles",
        primary.candles.len(),
        primary
            .candles
            .iter()
            .filter(|c| c.funding_rate.is_some())
            .count()
    );
}
