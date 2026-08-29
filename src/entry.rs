//! Executable topology — the argv dispatch that decides GUI vs CLI (ADR-0020).
//!
//! ADR-0015 pins **one shippable artifact, zero sidecars**. ADR-0020 resolves what that
//! means once one bundle has to be both a desktop application and a CLI: the bundle
//! contains **one binary**, and that binary decides which surface to become by looking
//! at its own arguments.
//!
//! - **No arguments** — the shape of a Finder / `LaunchServices` double-click, which
//!   attaches no terminal — selects [`LaunchMode::Gui`].
//! - **Any argument** selects [`LaunchMode::Cli`], and `pulse::run()` behaves exactly as
//!   it did before the shell existed.
//!
//! The decision is a **pure function over an argument iterator** rather than a branch
//! buried in `main`, for one reason that matters: both directions are then testable
//! without a display server (`tests/entry_topology.rs`, AC-2). `main` does nothing but
//! call it.
//!
//! **Why the OS-injected filter exists.** macOS does not always hand a bundled
//! executable an empty argument list. `LaunchServices` historically injects a
//! process-serial-number argument (`-psn_0_1234567`), and `AppKit` injects
//! `-NSDocumentRevisionsDebugMode YES` when launched from Xcode. Neither is user
//! intent. Counting them as arguments would send a Finder double-click into `clap`,
//! which would print a usage error to a terminal nobody is watching and exit — the
//! failure would look like "the app does not open".

use std::ffi::OsStr;

/// Which surface this process should become, decided once at startup from argv.
///
/// See [`launch_mode`] for the rule and ADR-0020 for why the topology is one binary
/// rather than two.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LaunchMode {
    /// Start the Tauri desktop shell. Selected by a launch carrying no user arguments.
    Gui,
    /// Run the `clap` command-line surface. Selected by any user argument.
    Cli,
}

/// The macOS `LaunchServices` process-serial-number argument prefix.
const PSN_PREFIX: &str = "-psn_";

/// The `AppKit` argument-namespace prefix (e.g. `-NSDocumentRevisionsDebugMode`). These
/// arrive as a flag followed by a separate value token, so both are consumed.
const APPKIT_PREFIX: &str = "-NS";

/// Decide the launch mode from a full argument vector (**including** `argv[0]`).
///
/// `argv[0]` is the program path, never user intent, so it is always discarded.
/// OS-injected launch arguments (see the module docs) are discarded too. If anything
/// survives that filter, the user meant the CLI; if nothing does, this is a bare
/// launch and the desktop shell is what was wanted.
///
/// An empty iterator degrades to [`LaunchMode::Gui`] rather than falling through to a
/// `clap` parse — a process with no `argv[0]` is pathological, and opening a window is
/// the safer of the two wrong answers.
///
/// ```
/// use pulse::{LaunchMode, launch_mode};
///
/// assert_eq!(launch_mode(["pulse"]), LaunchMode::Gui);
/// assert_eq!(launch_mode(["pulse", "-psn_0_1234567"]), LaunchMode::Gui);
/// assert_eq!(launch_mode(["pulse", "runs", "list"]), LaunchMode::Cli);
/// ```
#[must_use]
pub fn launch_mode<I, S>(argv: I) -> LaunchMode
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut remaining = argv.into_iter();
    // Discard argv[0]: the program path is not an argument.
    if remaining.next().is_none() {
        return LaunchMode::Gui;
    }

    // True when the previous token was an AppKit flag whose value token follows.
    let mut consuming_appkit_value = false;

    for arg in remaining {
        if consuming_appkit_value {
            consuming_appkit_value = false;
            continue;
        }

        // Lossy is correct here: we only ever compare against ASCII prefixes, and a
        // non-UTF-8 argument is by definition not one of the OS-injected tokens, so
        // it must count as user intent (the `else` branch below).
        let arg = arg.as_ref().to_string_lossy();

        if arg.starts_with(PSN_PREFIX) {
            continue;
        }
        if arg.starts_with(APPKIT_PREFIX) {
            consuming_appkit_value = true;
            continue;
        }

        return LaunchMode::Cli;
    }

    LaunchMode::Gui
}

/// The process's own launch mode, read from [`std::env::args_os`].
///
/// This is the call `main` makes; [`launch_mode`] is the testable core it delegates to.
#[must_use]
pub fn launch_mode_from_env() -> LaunchMode {
    launch_mode(std::env::args_os())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{LaunchMode, launch_mode};

    #[test]
    fn argv_zero_alone_is_a_gui_launch() {
        assert_eq!(launch_mode(["pulse"]), LaunchMode::Gui);
    }

    #[test]
    fn a_single_user_argument_is_a_cli_launch() {
        assert_eq!(launch_mode(["pulse", "runs"]), LaunchMode::Cli);
    }

    #[test]
    fn appkit_flag_consumes_exactly_one_value_token() {
        // The value token is consumed...
        assert_eq!(
            launch_mode(["pulse", "-NSDocumentRevisionsDebugMode", "YES"]),
            LaunchMode::Gui
        );
        // ...but only one, so a real argument after it still selects the CLI.
        assert_eq!(
            launch_mode(["pulse", "-NSDocumentRevisionsDebugMode", "YES", "runs"]),
            LaunchMode::Cli
        );
    }

    #[test]
    #[cfg(unix)]
    fn a_non_utf8_argument_counts_as_user_intent() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        // 0xFF is not valid UTF-8. It is not an OS-injected token, so it is a real
        // argument and must select the CLI (where clap will reject it properly).
        let bad = OsString::from_vec(vec![0xFF]);
        assert_eq!(launch_mode([OsString::from("pulse"), bad]), LaunchMode::Cli);
    }
}
