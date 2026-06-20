//! CLI disclaimer assertions (VS-1.2.3 CX3 runtime disclaimer).
//!
//! Verifies that `pulse --help` carries the "not financial advice" disclaimer
//! and that `pulse indicators` output ends with the disclaimer footer.
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

/// `pulse indicators` output must end with the disclaimer footer line.
#[test]
fn indicators_output_ends_with_disclaimer_footer() {
    let output = Command::new(env!("CARGO_BIN_EXE_pulse"))
        .args(["indicators", "--pair", "BTCUSDT", "--tf", "M15"])
        .output()
        .expect("run pulse indicators");

    assert!(
        output.status.success(),
        "pulse indicators must succeed; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout is utf8");
    let last_line = stdout.lines().last().expect("output has lines");
    assert_eq!(
        last_line,
        "\u{26a0} Not financial advice \u{2014} hypothetical results. See DISCLAIMER.md",
        "last line of indicators output must be the disclaimer footer"
    );
}
