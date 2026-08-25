//! `Tauri` ring (outer): the desktop entry point, the command bus and the typed event
//! channel (ADR-0020).
//!
//! **This module's name collides with the `tauri` crate on purpose.** ADR-0020's
//! registered touch surface is `src/tauri/**`, so the ring keeps its name; the extern
//! crate is reached as `tauri::` from inside these files (the crate root, where
//! `mod tauri` is in scope, uses `crate::tauri::` for the ring). Renaming the
//! *dependency* is not an option — `#[tauri::command]` and `generate_handler!` expand to
//! hard-coded `::tauri::` paths.
//!
//! Layout, matching the spec's step 4:
//!
//! | File | Holds |
//! |---|---|
//! | `mod.rs` | the app builder, managed-state wiring, [`run_desktop`] |
//! | `commands.rs` | the bus: the registration list, managed state, the commands |
//! | `events.rs` | the typed per-invocation channel and its payloads |
//! | `error.rs` | the one serializable error shape |
//!
//! **Where the boundary sits.** This ring depends inward on the domain and the adapters
//! and is depended on by nothing — it is the outermost ring, and `run_desktop` is its
//! only entry point. The transport-free command *cores* (`shell_info_core`,
//! `demo_stream_core`) are what carry behaviour; the `#[tauri::command]` wrappers do
//! nothing but adapt the transport, which is why the bus contract is testable without an
//! app handle.

pub(crate) mod commands;
pub(crate) mod error;
pub(crate) mod events;

pub use commands::{
    BUS_COMMANDS, DesktopState, ShellInfo, StreamOutcome, demo_stream_core, shell_info_core,
};
pub use error::{BusError, BusErrorCode};
pub use events::{BusEvent, BusEventPayload, EventSink, RunId};

/// Build the `tauri-specta` builder that owns the command registry.
///
/// **One place, one list.** `collect_commands!` here and [`BUS_COMMANDS`] in
/// `commands.rs` are the two halves of clause 4, and
/// `tests/tauri_bus_contract.rs::command_registration_is_one_append_only_list` asserts
/// they cannot drift: every name in the list must have a matching `async fn`, and the
/// count of `#[tauri::command]` functions must equal the list's length.
///
/// Adding a screen in round 3 means appending **one line** here, one line to
/// `BUS_COMMANDS`, one `async fn`, and one row in `ui/src/routes.ts`.
fn specta_builder() -> tauri_specta::Builder {
    tauri_specta::Builder::<tauri::Wry>::new().commands(tauri_specta::collect_commands![
        commands::shell_info,
        commands::bus_selftest_failure,
        commands::start_demo_stream,
    ])
}

/// Export the generated TypeScript bindings to `path`.
///
/// Called by `examples/export-bindings.rs`, which `scripts/check-bindings.sh` (AC-8)
/// drives into a temporary file and diffs against the committed `ui/src/bindings.ts`.
/// Generation lives here, next to the registry it reflects, so a command added without
/// regenerating is a **diff**, not a runtime surprise.
///
/// # Errors
///
/// Returns an [`anyhow::Error`] if the bindings cannot be generated or written.
pub fn export_bindings(path: &std::path::Path) -> anyhow::Result<()> {
    specta_builder()
        .export(specta_typescript::Typescript::default(), path)
        .map_err(|e| anyhow::anyhow!("export tauri-specta bindings to {}: {e}", path.display()))?;

    let generated = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("read back generated bindings {}: {e}", path.display()))?;
    std::fs::write(path, post_process_bindings(&generated))
        .map_err(|e| anyhow::anyhow!("write post-processed bindings {}: {e}", path.display()))
}

/// Repair two defects in `tauri-specta` 2.0.0-rc.21's TypeScript output.
///
/// **This is not cosmetic — without it the generated file does not compile**, and
/// `npm run typecheck` (wired into `just check`, AC-9) fails on the generated artifact
/// rather than on anything a human wrote.
///
/// 1. **The `TAURI_CHANNEL` name collision.** rc.21 emits `export type
///    TAURI_CHANNEL<TSend> = null` as a *user* type **and** imports
///    `Channel as TAURI_CHANNEL`, which is `TS2440: Import declaration conflicts with
///    local declaration`. Worse than the error: if the local declaration won, every
///    channel-taking command would be typed `null` and the frontend could not pass a
///    real channel. tauri-specta rc.25 special-cases this (it recognises the
///    `TAURI_CHANNEL` remote type and emits the import alone), but rc.25's specta does
///    not build on this crate's pinned toolchain — see the `Cargo.toml` pin comment.
///    Dropping the bogus local declaration leaves exactly what rc.25 would have emitted.
/// 2. **Dead event machinery.** ADR-0020 chose per-invocation channels over a global
///    event bus, so `collect_events!` is deliberately empty — but rc.21 emits the
///    `__EventObj__` type, the `__makeEvents__` helper and their two imports anyway.
///    Under this project's `noUnusedLocals` they are hard errors. They are removed
///    rather than silenced, because removing genuinely dead generated code is correct
///    and `// @ts-nocheck` would have disabled checking of the whole typed seam.
///
/// **Deterministic and idempotent**, which is what makes AC-8's regenerate-and-diff
/// check meaningful: the same input always yields the same output, and re-running it on
/// already-processed output changes nothing. When a future upgrade fixes these upstream,
/// the transformations simply stop matching and become no-ops.
fn post_process_bindings(src: &str) -> String {
    // Exactly one occurrence == the declaration only, never a call site. If events are
    // ever registered, this is false and the machinery is left alone.
    let events_unused = src.matches("__makeEvents__").count() == 1;

    let mut out = String::with_capacity(src.len());
    let mut skipping_event_obj = false;

    for line in src.lines() {
        let trimmed = line.trim();

        // (1) the bogus local declaration that collides with the import
        if trimmed.starts_with("export type TAURI_CHANNEL<") {
            continue;
        }

        if events_unused {
            if trimmed.starts_with("import * as TAURI_API_EVENT") {
                continue;
            }
            if trimmed.starts_with("import") && trimmed.contains("__WebviewWindow__") {
                continue;
            }
            if trimmed.starts_with("type __EventObj__<") {
                skipping_event_obj = true;
                continue;
            }
            if skipping_event_obj {
                if trimmed == "};" {
                    skipping_event_obj = false;
                }
                continue;
            }
            // The helper is the last item in the generated file, so stopping here drops
            // it and nothing else.
            if trimmed.starts_with("function __makeEvents__") {
                break;
            }
        }

        out.push_str(line);
        out.push('\n');
    }

    // Normalise the tail so the committed file ends with exactly one newline; otherwise
    // the number of trailing blank lines would depend on where the cut landed and the
    // AC-8 diff would be noisy.
    format!("{}\n", out.trim_end())
}

/// The desktop entry point — what a Finder launch reaches (ADR-0020).
///
/// Startup order, and why it is this order:
///
/// 1. **Open and migrate the database first.** `DesktopState::open_default` uses
///    migrate-then-open, so a migration failure refuses to start rather than opening a
///    window onto a half-migrated database. Failing here surfaces as a non-zero exit
///    from `main`, which is the honest outcome — a window that renders an unusable app
///    is worse than no window.
/// 2. **Then build the app**, handing the state to Tauri's managed state so every
///    command shares one pool.
///
/// # Errors
///
/// Returns an [`anyhow::Error`] if the database cannot be opened/migrated, if the tokio
/// runtime cannot be built, or if the Tauri runtime fails to start.
pub fn run_desktop() -> anyhow::Result<()> {
    // The same sync -> async bridge shape the CLI uses (audit C3): a thin sync entry
    // that builds the runtime itself. Tauri's own event loop must own the main thread,
    // so the runtime is entered only for startup work, not wrapped around the app.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| anyhow::anyhow!("build tokio runtime: {e}"))?;

    let state = runtime
        .block_on(commands::DesktopState::open_default())
        .map_err(|e| anyhow::anyhow!("open the desktop database: {e}"))?;

    let builder = specta_builder();

    tauri::Builder::default()
        .invoke_handler(builder.invoke_handler())
        .manage(state)
        // Keep the tokio runtime alive for the app's lifetime: commands are async and
        // spawn onto it. Dropping it here would abort every in-flight command.
        .manage(runtime)
        .run(tauri::generate_context!())
        .map_err(|e| anyhow::anyhow!("run the desktop shell: {e}"))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::post_process_bindings;

    /// A faithful miniature of what tauri-specta 2.0.0-rc.21 emits with an empty
    /// `collect_events!` and one channel-taking command.
    const RC21_SHAPED_OUTPUT: &str = r#"export const commands = {
async startDemoStream(steps: number, channel: TAURI_CHANNEL<BusEvent>) : Promise<null> {}
}

export type BusEvent = { seq: number }
export type TAURI_CHANNEL<TSend> = null

/** tauri-specta globals **/

import {
	invoke as TAURI_INVOKE,
	Channel as TAURI_CHANNEL,
} from "@tauri-apps/api/core";
import * as TAURI_API_EVENT from "@tauri-apps/api/event";
import { type WebviewWindow as __WebviewWindow__ } from "@tauri-apps/api/webviewWindow";

type __EventObj__<T> = {
	listen: (cb: T) => void;
	emit: null extends T
		? (payload?: T) => void
		: (payload: T) => void;
};

export type Result<T, E> =
	| { status: "ok"; data: T }
	| { status: "error"; error: E };

function __makeEvents__<T extends Record<string, any>>(
	mappings: Record<keyof T, string>,
) {
	return null;
}
"#;

    #[test]
    fn the_channel_name_collision_is_removed() {
        let out = post_process_bindings(RC21_SHAPED_OUTPUT);
        assert!(
            !out.contains("export type TAURI_CHANNEL<"),
            "the bogus local TAURI_CHANNEL declaration must be dropped -- it is TS2440 \
             against the `Channel as TAURI_CHANNEL` import"
        );
        // The IMPORT, which is the one that must survive, does.
        assert!(
            out.contains("Channel as TAURI_CHANNEL"),
            "the real Channel import must survive; without it the command signature \
             has no type at all"
        );
        // And the command signature still refers to it.
        assert!(out.contains("channel: TAURI_CHANNEL<BusEvent>"));
    }

    #[test]
    fn dead_event_machinery_is_removed_but_result_is_kept() {
        let out = post_process_bindings(RC21_SHAPED_OUTPUT);
        for dead in [
            "__makeEvents__",
            "__EventObj__",
            "TAURI_API_EVENT",
            "__WebviewWindow__",
        ] {
            assert!(
                !out.contains(dead),
                "unused generated symbol `{dead}` must be removed (noUnusedLocals), got:\n{out}"
            );
        }
        // `Result` sits BETWEEN the dead type and the dead function; a naive
        // cut-to-end-of-file would take it with them, and every command returns it.
        assert!(
            out.contains("export type Result<T, E> ="),
            "the Result type must survive -- every command's return type names it"
        );
        assert!(out.contains("export type BusEvent"));
        assert!(out.contains("export const commands"));
    }

    #[test]
    fn post_processing_is_idempotent() {
        // AC-8 regenerates and diffs, so a transformation that changed its own output
        // on a second pass would make the check flap.
        let once = post_process_bindings(RC21_SHAPED_OUTPUT);
        let twice = post_process_bindings(&once);
        assert_eq!(once, twice);
    }

    #[test]
    fn registered_events_are_left_alone() {
        // If a later slice registers real events, `__makeEvents__` gains a call site and
        // the machinery must survive untouched.
        let with_events =
            format!("{RC21_SHAPED_OUTPUT}\nexport const events = __makeEvents__<X>({{}});\n");
        let out = post_process_bindings(&with_events);
        assert!(
            out.contains("function __makeEvents__"),
            "used event machinery must be preserved"
        );
        assert!(out.contains("type __EventObj__<"));
        // The channel collision is still repaired -- that half is unconditional.
        assert!(!out.contains("export type TAURI_CHANNEL<"));
    }

    #[test]
    fn output_ends_with_exactly_one_newline() {
        let out = post_process_bindings(RC21_SHAPED_OUTPUT);
        assert!(out.ends_with('\n'));
        assert!(!out.ends_with("\n\n"));
    }
}
