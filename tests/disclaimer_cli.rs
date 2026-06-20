//! CLI disclaimer assertions (VS-1.2.3 CX3 runtime disclaimer).
//!
//! Verifies that `pulse --help` carries the "not financial advice" disclaimer
//! via the clap `long_about` string.
//!
//! The disclaimer footer on `pulse indicators` output is covered by the
//! fixture-backed `tests/indicators_cli_runs_over_fixture.rs` (robust — has
//! proper candle fixture setup and a `lines.last()` assertion).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::Command;

/// `pulse --help` must contain the "not financial advice" disclaimer text.
#[test]
fn cli_help_shows_not_financial_advice() {
    let output = Command::new(env!("CARGO_BIN_EXE_pulse"))
        .arg("--help")
        .output()
        .expect("run pulse --help");

    // clap writes --help output to stdout; combine both streams for robustness.
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        combined.to_lowercase().contains("not financial advice"),
        "pulse --help must carry the disclaimer; got:\n{combined}"
    );
}
