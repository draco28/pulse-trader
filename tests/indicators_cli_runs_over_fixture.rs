//! Offline end-to-end test for `pulse indicators` over the committed fixture.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::Command;

#[test]
fn indicators_cli_runs_over_fixture() {
    let output = Command::new(env!("CARGO_BIN_EXE_pulse"))
        .args(["indicators", "--pair", "BTCUSDT", "--tf", "M15"])
        .output()
        .expect("run pulse indicators");

    assert!(
        output.status.success(),
        "status={:?}\nstderr={}\nstdout={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout is utf8");
    let lines = stdout.lines().collect::<Vec<_>>();
    assert_eq!(lines.first(), Some(&"open_time\trsi:14\tema:50\tadx:14"));
    assert_eq!(lines.len(), 2_978, "header + 2976 candle rows + summary");

    assert_eq!(
        lines[1].split('\t').collect::<Vec<_>>()[1..],
        ["—", "—", "—"]
    );
    assert_eq!(lines[14].split('\t').collect::<Vec<_>>()[1], "—");
    assert_ne!(lines[15].split('\t').collect::<Vec<_>>()[1], "—");
    assert_eq!(lines[27].split('\t').collect::<Vec<_>>()[3], "—");
    assert_ne!(lines[28].split('\t').collect::<Vec<_>>()[3], "—");
    assert_eq!(lines[49].split('\t').collect::<Vec<_>>()[2], "—");
    assert_ne!(lines[50].split('\t').collect::<Vec<_>>()[2], "—");

    assert_eq!(
        lines.last(),
        Some(
            &"summary\tcandles=2976\trsi:14_first_row=15\tema:50_first_row=50\tadx:14_first_row=28"
        )
    );
}
