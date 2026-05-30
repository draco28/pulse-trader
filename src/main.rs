//! Thin binary shim (audit C1).
//!
//! `main.rs` and `lib.rs` compile as SEPARATE crates: this binary may only
//! call the library's `pub` API (`pulse::run`), never any `pub(crate)`
//! internals. All logic lives library-side. `run()` is a placeholder here
//! (WI-05 fills it with CLI argument parsing).
use std::process::ExitCode;

fn main() -> ExitCode {
    match pulse::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("pulse: {err:#}");
            ExitCode::FAILURE
        }
    }
}
