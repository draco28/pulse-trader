//! Regenerate the `tauri-specta` TypeScript bindings (ADR-0020, AC-8).
//!
//! ```text
//! cargo run --quiet --example export-bindings -- ui/src/bindings.ts
//! ```
//!
//! `ui/src/bindings.ts` is **committed**, and `scripts/check-bindings.sh` runs this into
//! a temporary file and diffs. So a command added or a bus type changed without
//! regenerating is a failing check with a readable diff, rather than a `TypeError` the
//! first time someone opens that screen.
//!
//! **Why an example and not a test.** A test that writes into the source tree is a test
//! with a side effect, and `cargo nextest` runs tests in parallel — two of them racing on
//! one file is a flake waiting to happen. An example is an explicit, single-purpose
//! invocation with an explicit destination.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let Some(out) = args.next() else {
        eprintln!("usage: cargo run --example export-bindings -- <output path>");
        eprintln!("  e.g. cargo run --example export-bindings -- ui/src/bindings.ts");
        return ExitCode::FAILURE;
    };

    let out = PathBuf::from(out);
    if let Some(parent) = out.parent()
        && !parent.as_os_str().is_empty()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        eprintln!("export-bindings: cannot create {}: {e}", parent.display());
        return ExitCode::FAILURE;
    }

    match pulse::export_bindings(&out) {
        Ok(()) => {
            eprintln!("export-bindings: wrote {}", out.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("export-bindings: {e:#}");
            ExitCode::FAILURE
        }
    }
}
