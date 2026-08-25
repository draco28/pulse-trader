//! AC-2 — the executable-topology gate (r1.s1.w1, ADR-0020 step 1).
//!
//! ADR-0015 pins **one shippable artifact, zero sidecars**. ADR-0020 resolves what
//! that means once the bundle has to start a GUI as well as a CLI: **one binary that
//! dispatches on argv** — no arguments (the shape of a Finder / `LaunchServices`
//! launch, which attaches no terminal) selects GUI startup; any argument selects the
//! CLI path, which behaves exactly as it did before.
//!
//! This test asserts the dispatch **in both directions**, plus the third clause that
//! makes the decision safe: the CLI's existing behaviour is unchanged.
//!
//! - The GUI direction is asserted against the pure [`pulse::launch_mode`] decision
//!   function rather than by launching a window: a test that opened a real window
//!   would need a display server, would be unrunnable in CI, and would prove less.
//!   `launch_mode` is the *entire* decision `main` makes, so testing it is testing
//!   the dispatch.
//! - The CLI direction is asserted twice: once through `launch_mode`, and once
//!   end-to-end through the REAL binary (`CARGO_BIN_EXE_pulse`), which is what proves
//!   the argv path still reaches `clap` and not the Tauri runtime.
//!
//! Every binary invocation here passes at least one argument **on purpose**. Invoking
//! the binary with no arguments would, by this very decision, try to open a window.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::Command;

use pulse::{LaunchMode, launch_mode};

// ---------------------------------------------------------------------------
// Direction 1: no arguments => GUI
// ---------------------------------------------------------------------------

#[test]
fn bare_launch_selects_the_gui() {
    // argv[0] alone is what a double-click from Finder looks like.
    assert_eq!(
        launch_mode(["/Applications/PulseTrader.app/Contents/MacOS/pulse"]),
        LaunchMode::Gui,
        "a launch carrying only argv[0] must select GUI startup"
    );
}

#[test]
fn an_empty_argv_selects_the_gui() {
    // Defensive: argv is never truly empty under a normal exec, but the decision
    // function must not panic or fall through to the CLI if it ever is.
    let empty: [&str; 0] = [];
    assert_eq!(
        launch_mode(empty),
        LaunchMode::Gui,
        "an empty argv must degrade to GUI, never to a clap parse"
    );
}

#[test]
fn os_injected_launch_arguments_still_select_the_gui() {
    // Finder / LaunchServices historically injects a process-serial-number argument,
    // and AppKit injects -NSDocumentRevisionsDebugMode (with a following value).
    // Neither is a user-supplied argument, so neither may flip the launch to CLI --
    // if one did, a Finder double-click would land in `clap` and exit with a usage
    // error instead of opening a window.
    assert_eq!(
        launch_mode([
            "/Applications/PulseTrader.app/Contents/MacOS/pulse",
            "-psn_0_1234567",
        ]),
        LaunchMode::Gui,
        "a bare -psn_ process-serial-number argument must not select the CLI"
    );

    assert_eq!(
        launch_mode(["pulse", "-NSDocumentRevisionsDebugMode", "YES"]),
        LaunchMode::Gui,
        "an -NS* AppKit argument and its value must not select the CLI"
    );

    assert_eq!(
        launch_mode([
            "pulse",
            "-psn_0_1234567",
            "-NSDocumentRevisionsDebugMode",
            "YES"
        ]),
        LaunchMode::Gui,
        "several OS-injected arguments together must still select the GUI"
    );
}

// ---------------------------------------------------------------------------
// Direction 2: arguments => CLI
// ---------------------------------------------------------------------------

#[test]
fn arguments_select_the_cli() {
    assert_eq!(
        launch_mode([
            "pulse",
            "fetch-data",
            "BTCUSDT",
            "--tf",
            "M15",
            "--years",
            "1"
        ]),
        LaunchMode::Cli,
        "a real subcommand must select the CLI path"
    );
    assert_eq!(
        launch_mode(["pulse", "--help"]),
        LaunchMode::Cli,
        "--help must select the CLI path"
    );
    assert_eq!(
        launch_mode(["pulse", "--version"]),
        LaunchMode::Cli,
        "--version must select the CLI path"
    );
    assert_eq!(
        launch_mode(["pulse", "not-a-subcommand"]),
        LaunchMode::Cli,
        "even an INVALID argument selects the CLI -- clap owns rejecting it, not the dispatch"
    );
}

#[test]
fn a_real_argument_beside_an_injected_one_selects_the_cli() {
    // The OS-injected filter must remove only the injected tokens, never swallow a
    // real one that happens to sit beside them.
    assert_eq!(
        launch_mode(["pulse", "-psn_0_1234567", "runs", "list"]),
        LaunchMode::Cli,
        "a real subcommand alongside a -psn_ argument must still select the CLI"
    );
}

// ---------------------------------------------------------------------------
// Direction 3: the CLI's existing behaviour is unchanged
// ---------------------------------------------------------------------------

#[test]
fn the_binary_still_serves_top_level_help_with_every_existing_subcommand() {
    let out = Command::new(env!("CARGO_BIN_EXE_pulse"))
        .arg("--help")
        .output()
        .expect("run the pulse binary with --help");

    assert!(
        out.status.success(),
        "`pulse --help` must still exit 0 (status {:?}), stderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    // Every subcommand that existed before the shell landed must still be listed.
    // A regression here means the GUI dispatch swallowed the CLI surface.
    for subcommand in [
        "fetch-data",
        "indicators",
        "strategy",
        "backtest",
        "runs",
        "llm-check",
        "compose",
    ] {
        assert!(
            stdout.contains(subcommand),
            "`pulse --help` no longer lists the `{subcommand}` subcommand -- the CLI regressed.\n\
             Full output:\n{stdout}"
        );
    }
}

#[test]
fn the_binary_still_parses_an_existing_subcommands_arguments() {
    let out = Command::new(env!("CARGO_BIN_EXE_pulse"))
        .args(["fetch-data", "--help"])
        .output()
        .expect("run the pulse binary with fetch-data --help");

    assert!(
        out.status.success(),
        "`pulse fetch-data --help` must still exit 0 (status {:?})",
        out.status.code()
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    for flag in ["--tf", "--years", "--json"] {
        assert!(
            stdout.contains(flag),
            "`pulse fetch-data --help` no longer documents `{flag}`.\nFull output:\n{stdout}"
        );
    }
}

#[test]
fn the_binary_still_rejects_an_unknown_subcommand_non_zero() {
    // The load-bearing negative: if argv dispatch had accidentally routed everything
    // to the GUI, this would hang or exit 0 instead of producing a clap usage error.
    let out = Command::new(env!("CARGO_BIN_EXE_pulse"))
        .arg("definitely-not-a-subcommand")
        .output()
        .expect("run the pulse binary with a bogus subcommand");

    assert!(
        !out.status.success(),
        "an unknown subcommand must still exit non-zero, got {:?}",
        out.status.code()
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("Usage") || stderr.contains("usage") || stderr.contains("unrecognized"),
        "an unknown subcommand must still produce a clap usage error, got stderr:\n{stderr}"
    );
}
