//! Thin binary shim (audit C1) — and, since r1.s1.w1, the **one** executable in the
//! bundle (ADR-0020).
//!
//! `main.rs` and `lib.rs` compile as SEPARATE crates: this binary may only call the
//! library's `pub` API, never any `pub(crate)` internals. All logic lives library-side,
//! including the launch decision itself.
//!
//! **Executable topology (ADR-0015 + ADR-0020).** ADR-0015 pins one shippable artifact
//! and zero sidecar processes. So `PulseTrader.app` contains exactly one executable, and
//! that executable chooses its surface from its own arguments:
//!
//! - **no user arguments** (a Finder / `LaunchServices` launch, no terminal attached)
//!   → `pulse::run_desktop`, the Tauri shell;
//! - **any user argument** → `pulse::run`, the `clap` CLI, unchanged.
//!
//! The decision itself is `pulse::launch_mode_from_env` — a pure function over argv, so
//! `tests/entry_topology.rs` can assert both directions without a display server. This
//! shim stays trivial: dispatch, then map `Result` → `ExitCode`.
use std::process::ExitCode;

use pulse::LaunchMode;

fn main() -> ExitCode {
    let outcome = match pulse::launch_mode_from_env() {
        LaunchMode::Gui => pulse::run_desktop(),
        LaunchMode::Cli => pulse::run(),
    };

    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("pulse: {err:#}");
            ExitCode::FAILURE
        }
    }
}
