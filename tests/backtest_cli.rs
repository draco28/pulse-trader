//! Offline smoke/integration test for `pulse backtest` (VS-1.2.1 work-1.04).
//!
//! **Fixture independence (R3 parallelism, spec §3 + handoff §8):** this test
//! does NOT consume 1.05's golden `rsi-oversold-long.json` (it does not exist in
//! this worktree). Instead it **synthesizes a minimal valid DSL into a tempdir**
//! and runs the CLI against the committed `tests/fixtures/btcusdt-1m-store/`
//! candle fixture — keeping 1.04 ∥ 1.05 file-disjoint.
//!
//! The DSL is the demo-1 "RSI Oversold" shape (long; RSI(14) < 30; 5% stop = 1R;
//! 2R take-profit; 1% risk/trade) — a complete document that passes `validate`
//! (≥1 exit, a `StopLoss` to anchor the `TakeProfit`) so the engine never returns
//! `NoStopLoss`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::Command;

/// A minimal, valid DSL document (serde JSON) — long RSI-oversold with a 5% stop
/// (1R), a 2R take-profit, and 1% risk per trade. Decimals serialize as strings
/// (`rust_decimal` `serde-with-str`); `schema_version` is the CURRENT `"1.0.0"`.
const MINIMAL_DSL: &str = r#"{
  "schema_version": "1.0.0",
  "name": "RSI Oversold (smoke)",
  "direction": "long",
  "entry": {
    "type": "Compare",
    "lhs": { "type": "Indicator", "spec": { "indicator": "Rsi", "period": 14 } },
    "op": "Lt",
    "rhs": { "type": "Constant", "value": "30" }
  },
  "filters": [],
  "exits": [
    { "type": "StopLoss", "distance_pct": "0.05" },
    { "type": "TakeProfit", "target_r": "2.0" }
  ],
  "risk": {
    "risk_per_trade_pct": "0.01",
    "max_leverage": "3"
  }
}"#;

/// Write the minimal DSL into a fresh tempdir and return (dir, dsl-path-string).
/// The `TempDir` is returned so the caller keeps it alive for the run's duration.
fn write_minimal_dsl() -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().expect("create tempdir for DSL");
    let path = dir.path().join("strategy.json");
    std::fs::write(&path, MINIMAL_DSL).expect("write minimal DSL");
    let path_str = path.to_str().expect("DSL path is utf8").to_owned();
    (dir, path_str)
}

/// AC-5/AC-6 + the user demo criterion: a full `pulse backtest` run over the
/// fixture store succeeds, and the human-readable trade log carries the
/// fee/funding/slippage cost columns + the cost-breakdown footer.
#[test]
fn backtest_cli_runs_over_fixture_and_shows_costs() {
    let (_dir, dsl_path) = write_minimal_dsl();

    let output = Command::new(env!("CARGO_BIN_EXE_pulse"))
        .args([
            "backtest",
            "--dsl",
            &dsl_path,
            "--pair",
            "BTCUSDT",
            "--tf",
            "M15",
            "--store",
            "tests/fixtures/btcusdt-1m-store",
        ])
        .output()
        .expect("run pulse backtest");

    assert!(
        output.status.success(),
        "status={:?}\nstderr={}\nstdout={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout is utf8");

    // Header row names the cost columns the demo criterion reads ("confirm
    // fees/funding/slippage are deducted").
    assert!(
        stdout.contains("fees") && stdout.contains("funding") && stdout.contains("slippage"),
        "header must name fee/funding/slippage columns; stdout was:\n{stdout}"
    );
    // The cost-breakdown footer totals + net P&L.
    assert!(
        stdout.contains("net_pnl"),
        "footer must report net P&L; stdout was:\n{stdout}"
    );
    assert!(
        stdout.contains("trades="),
        "footer must report the trade count; stdout was:\n{stdout}"
    );
}

/// `--json` emits a structured object carrying the trades + run-level cost
/// totals (the same `BacktestResult` surface the demo reads, machine-parseable).
#[test]
fn backtest_cli_json_emits_structured_result() {
    let (_dir, dsl_path) = write_minimal_dsl();

    let output = Command::new(env!("CARGO_BIN_EXE_pulse"))
        .args([
            "backtest",
            "--dsl",
            &dsl_path,
            "--pair",
            "BTCUSDT",
            "--tf",
            "M15",
            "--store",
            "tests/fixtures/btcusdt-1m-store",
            "--json",
        ])
        .output()
        .expect("run pulse backtest --json");

    assert!(
        output.status.success(),
        "status={:?}\nstderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout is utf8");
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).expect("stdout is JSON");
    assert!(
        parsed.get("trades").is_some(),
        "JSON must carry the trade log; got:\n{stdout}"
    );
    assert!(
        parsed.get("net_pnl").is_some(),
        "JSON must carry net P&L; got:\n{stdout}"
    );
    assert!(
        parsed.get("fees_total").is_some()
            && parsed.get("funding_total").is_some()
            && parsed.get("slippage_total").is_some(),
        "JSON must carry the cost totals; got:\n{stdout}"
    );
}

/// A bad `--dsl` path surfaces a clear error and a non-zero exit (no panic) —
/// the spec §3 error-surfacing contract via `anyhow`.
#[test]
fn backtest_cli_missing_dsl_errors_non_zero() {
    let output = Command::new(env!("CARGO_BIN_EXE_pulse"))
        .args([
            "backtest",
            "--dsl",
            "/nonexistent/strategy.json",
            "--pair",
            "BTCUSDT",
            "--tf",
            "M15",
            "--store",
            "tests/fixtures/btcusdt-1m-store",
        ])
        .output()
        .expect("run pulse backtest with bad dsl");

    assert!(
        !output.status.success(),
        "a missing --dsl file must exit non-zero"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("pulse:"),
        "the error must be surfaced via the binary shim (no panic); stderr was:\n{stderr}"
    );
}
