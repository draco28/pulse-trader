//! The command bus (ADR-0020, bus contract clauses 2–4).
//!
//! # The contract this file pins
//!
//! **Clause 2 — async and cancellation.** Every `#[tauri::command]` here is an
//! `async fn`. A synchronous command occupies the IPC thread for its whole duration and
//! the window stops repainting, so "commands are async" is not a style preference — it
//! is the property that keeps a slow query from freezing the app. When a screen
//! unmounts, its channel is dropped and the next send fails; a streaming command reads
//! that failure as **cancellation** and stops, rather than running to completion
//! emitting into nothing or surfacing an error into a screen that no longer exists.
//!
//! **Clause 3 — managed state ownership.** [`DesktopState`] holds the things that are
//! expensive, shared and long-lived: the `SQLite` pool (opened and migrated **once**, at
//! startup) and the repositories built over it. A command constructs per call only what
//! is cheap and request-scoped. Opening a pool per command would serialize every request
//! behind a fresh connection and defeat WAL.
//!
//! **Clause 4 — one registration point, append-only.** [`BUS_COMMANDS`] is the single
//! list, one entry per line, and `generate_handler!` in `super` wires exactly those. Two
//! work items each adding one screen therefore conflict **textually** — adjacent lines
//! in one file, resolved by keeping both — and never **semantically**. This is what
//! keeps `r1.s1.w3` and `r1.s1.w4` parallel in round 3; the DAG dropped that edge on
//! this property, so weakening it re-creates a dependency the plan was authored without.
//!
//! `tests/tauri_bus_contract.rs` (AC-3) gates all four clauses.

// The `#[tauri::command]` macro expands to a wrapper whose generated signature takes its
// arguments by value and whose body is generated code we do not own. Two pedantic lints
// fire on that expansion rather than on anything written here. Scoped to this module so
// the crate-wide pedantic posture is untouched everywhere else.
#![allow(clippy::needless_pass_by_value, clippy::used_underscore_binding)]

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::error::BusError;
use super::events::{BusEvent, BusEventPayload, EventSink, RunId};
use crate::adapters::clock::SystemClock;
use crate::adapters::db::{Db, SqliteStrategyRepo, default_db_path, open_migrated};
use crate::domain::EngineFingerprint;
use crate::domain::StrategyRepository;

// ---------------------------------------------------------------------------
// Clause 4 — the ONE registration point
// ---------------------------------------------------------------------------

/// **The** command registration list. One entry per line, append-only.
///
/// Adding a screen means adding **one line here** and one `#[tauri::command] async fn`
/// below, and one line to `ui/src/routes.ts`. Nothing else. Do not introduce a second
/// list, do not group entries onto one line, and do not reorder — every one of those
/// turns a clean textual merge conflict into a silent semantic one.
///
/// `tests/tauri_bus_contract.rs::command_registration_is_one_append_only_list` enforces
/// the shape; `super::run_desktop`'s `generate_handler!` is the code that consumes it.
///
/// **`#[rustfmt::skip]` is deliberate and load-bearing, not a style preference.**
/// rustfmt collapses a short array onto one line, and one line is precisely what breaks
/// this contract: two work items each appending a command would then edit the SAME line
/// and produce a conflict a merge tool resolves by picking ONE side — silently dropping
/// the other item's command. One entry per line makes that conflict a two-added-lines
/// diff that is resolved by keeping both. Do not remove this attribute.
#[rustfmt::skip]
pub const BUS_COMMANDS: &[&str] = &[
    "shell_info",
    "bus_selftest_failure",
    "start_demo_stream",
];

// ---------------------------------------------------------------------------
// Clause 3 — managed state
// ---------------------------------------------------------------------------

/// What Tauri's managed state owns, shared by every command for the app's lifetime.
///
/// Currently the migrated `SQLite` pool. Repositories are handed out over it by
/// [`DesktopState::strategy_repo`] — cheap wrappers around a cloned pool handle, not new
/// connections. Round 3 adds the backtest-run and LLM-call repos on the same pattern.
pub struct DesktopState {
    db: Db,
}

impl DesktopState {
    /// Open (and migrate) the database at `path` and take ownership of the pool.
    ///
    /// Uses `open_migrated` — migrate-then-open — so a migration failure **refuses to
    /// start** rather than running the shell against a half-migrated database. That is
    /// the same startup discipline the CLI uses (MASTER-SPEC §7.4).
    ///
    /// # Errors
    ///
    /// Returns a [`BusError`] if the migration or the pool open fails.
    pub async fn open(path: &Path) -> Result<Self, BusError> {
        let db = open_migrated(path).await?;
        Ok(Self { db })
    }

    /// Open the default `~/Library/Application Support/PulseTrader/pulse.db`.
    ///
    /// # Errors
    ///
    /// Returns a [`BusError`] if the path cannot be resolved or the open fails.
    pub async fn open_default() -> Result<Self, BusError> {
        let path = default_db_path()?;
        Self::open(&path).await
    }

    /// A strategy repository over the shared pool.
    #[must_use]
    pub fn strategy_repo(&self) -> SqliteStrategyRepo<SystemClock> {
        SqliteStrategyRepo::new(self.db.pool().clone())
    }

    /// The owned database handle.
    #[must_use]
    pub fn db(&self) -> &Db {
        &self.db
    }
}

// ---------------------------------------------------------------------------
// Round-trip command: shell metadata
// ---------------------------------------------------------------------------

/// The metadata the placeholder page renders — the one round-trip command this work
/// item ships.
///
/// Deliberately boring: no credential and no LLM-derived data crosses this boundary, so
/// no risk gate fires on this item. `r1.s1.w4` is where that changes and it carries the
/// controls.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ShellInfo {
    /// The crate version this bundle was built from.
    pub app_version: String,
    /// The build-time `engine_fingerprint` (FR-7) — proves the GUI and CLI share one core.
    pub engine_fingerprint: String,
    /// The compiled target triple.
    pub target_triple: String,
    /// How many strategies the database holds — a real read through managed state.
    pub strategy_count: u32,
}

/// The transport-free core of the `shell_info` command.
///
/// Split from the `#[tauri::command]` wrapper so it is drivable from a test without an
/// app handle. The wrapper does nothing but unwrap the managed state and call this.
///
/// # Errors
///
/// Returns a [`BusError`] if the strategy read fails.
pub async fn shell_info_core(state: &DesktopState) -> Result<ShellInfo, BusError> {
    let strategies = state.strategy_repo().list_strategies(true).await?;
    Ok(ShellInfo {
        app_version: env!("CARGO_PKG_VERSION").to_owned(),
        engine_fingerprint: EngineFingerprint::current().as_str().to_owned(),
        target_triple: EngineFingerprint::target().to_owned(),
        strategy_count: u32::try_from(strategies.len()).unwrap_or(u32::MAX),
    })
}

// ---------------------------------------------------------------------------
// Clause 2 — the streaming core, and what cancellation means
// ---------------------------------------------------------------------------

/// How a streaming run ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct StreamOutcome {
    /// The run this outcome describes.
    pub run_id: RunId,
    /// How many events actually reached the far end.
    pub emitted: u32,
    /// True when the far end went away mid-run (the screen unmounted).
    pub cancelled: bool,
}

/// Emit `steps` events for `run_id` into `sink`, stopping early if the far end dies.
///
/// The demo stream for this work item: `Started`, then `Progress`, then `Finished`,
/// with `seq` monotonic from 0. `r1.s1.w4` replaces the body with the real compose
/// stream; **the shape of this function is the part that is pinned** — a run id, a
/// sink, a `StreamOutcome`, and cancellation-by-failed-send.
///
/// **Cancellation is a normal return, not an error.** When a screen unmounts its channel
/// drops, and the next `send_event` fails. That is not a fault to report: there is no
/// screen left to report it to, and treating it as an error would put a spurious failure
/// in the log for every user who navigated away mid-run. The loop stops at the first
/// failed send and returns `cancelled: true`, so the caller can distinguish "the user
/// left" from "the run finished".
///
/// The `yield_now` between steps is what makes "a slow command does not block the
/// window" true in practice — it hands control back to the runtime between events
/// instead of monopolising the executor.
///
/// # Errors
///
/// Returns a [`BusError`] only for a genuine failure. A dead sink is cancellation.
pub async fn demo_stream_core<S>(
    run_id: &RunId,
    steps: u32,
    sink: &S,
) -> Result<StreamOutcome, BusError>
where
    S: EventSink + ?Sized,
{
    // A run always opens with `Started` and closes with `Finished`, so fewer than two
    // events is not expressible. A request for fewer is raised rather than rejected --
    // an unterminated one-event stream would leave a screen spinning forever.
    let steps = steps.max(2);

    let mut emitted = 0_u32;
    let mut cancelled = false;

    for seq in 0..steps {
        let payload = if seq == 0 {
            BusEventPayload::Started
        } else if seq + 1 == steps {
            BusEventPayload::Finished {
                message: format!("run complete after {steps} step(s)"),
            }
        } else {
            BusEventPayload::Progress {
                message: format!("step {seq} of {steps}"),
            }
        };

        if sink
            .send_event(BusEvent::new(run_id, seq, payload))
            .is_err()
        {
            cancelled = true;
            break;
        }
        emitted += 1;

        // Cooperative yield: the window stays responsive between events.
        tokio::task::yield_now().await;
    }

    Ok(StreamOutcome {
        run_id: run_id.clone(),
        emitted,
        cancelled,
    })
}

// ---------------------------------------------------------------------------
// The registered commands. One `async fn` per BUS_COMMANDS entry, same order.
// ---------------------------------------------------------------------------

/// Round-trip command: shell + core metadata for the placeholder page.
///
/// # Errors
///
/// Returns a [`BusError`] if the read through managed state fails.
#[tauri::command]
#[specta::specta]
pub async fn shell_info(state: tauri::State<'_, DesktopState>) -> Result<ShellInfo, BusError> {
    shell_info_core(&state).await
}

/// A command that fails **on purpose**, so the error path is demonstrated rather than
/// asserted only in a unit test.
///
/// The placeholder page invokes it and renders the resulting [`BusError`]. Keeping a
/// deliberate-failure command on the bus means the frontend's error rendering is
/// exercised by every developer who opens the app, not only when something breaks.
///
/// # Errors
///
/// Always. That is the point.
#[tauri::command]
#[specta::specta]
pub async fn bus_selftest_failure() -> Result<(), BusError> {
    // A real domain error, mapped through the real `From` impl -- not a synthetic
    // BusError, so this exercises the mapping the frontend actually depends on.
    Err(
        crate::domain::DataError::Parse("deliberate bus self-test failure (r1.s1.w1)".to_owned())
            .into(),
    )
}

/// Start the demo event stream on a **per-invocation** channel.
///
/// The `channel` argument is the whole correlation mechanism: Tauri mints one per
/// `invoke`, so a second run cannot reach the first run's screen.
///
/// # Errors
///
/// Returns a [`BusError`] on a genuine failure; a dropped channel is reported as
/// `cancelled` in the [`StreamOutcome`], not as an error.
#[tauri::command]
#[specta::specta]
pub async fn start_demo_stream(
    steps: u32,
    channel: tauri::ipc::Channel<BusEvent>,
) -> Result<StreamOutcome, BusError> {
    let run_id = RunId::new();
    demo_stream_core(&run_id, steps.min(64), &channel).await
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{BUS_COMMANDS, BusError, RunId, demo_stream_core};
    use crate::tauri::error::BusErrorCode;
    use crate::tauri::events::{BusEvent, EventSink};
    use std::cell::RefCell;

    struct Collector {
        events: RefCell<Vec<BusEvent>>,
    }

    impl EventSink for Collector {
        fn send_event(&self, event: BusEvent) -> Result<(), BusError> {
            self.events.borrow_mut().push(event);
            Ok(())
        }
    }

    #[tokio::test]
    async fn a_stream_opens_with_started_and_closes_with_finished() {
        let sink = Collector {
            events: RefCell::new(Vec::new()),
        };
        let run_id = RunId::new();
        let outcome = demo_stream_core(&run_id, 3, &sink).await.unwrap();

        assert_eq!(outcome.emitted, 3);
        assert!(!outcome.cancelled);

        let events = sink.events.borrow();
        assert!(matches!(
            events[0].payload,
            crate::tauri::events::BusEventPayload::Started
        ));
        assert!(matches!(
            events[2].payload,
            crate::tauri::events::BusEventPayload::Finished { .. }
        ));
    }

    #[tokio::test]
    async fn a_stream_can_never_be_left_unterminated() {
        // Edge case: a request for 0 or 1 steps cannot express both `Started` and
        // `Finished`, and an unterminated stream would leave a screen spinning. The
        // core raises the count instead of emitting a run with no end.
        for requested in [0_u32, 1] {
            let sink = Collector {
                events: RefCell::new(Vec::new()),
            };
            let outcome = demo_stream_core(&RunId::new(), requested, &sink)
                .await
                .unwrap();
            assert_eq!(
                outcome.emitted, 2,
                "a {requested}-step request must still emit Started + Finished"
            );
            let events = sink.events.borrow();
            assert!(matches!(
                events[0].payload,
                crate::tauri::events::BusEventPayload::Started
            ));
            assert!(matches!(
                events[1].payload,
                crate::tauri::events::BusEventPayload::Finished { .. }
            ));
        }
    }

    #[test]
    fn the_registration_list_is_not_empty() {
        assert!(!BUS_COMMANDS.is_empty());
        assert!(BUS_COMMANDS.contains(&"shell_info"));
    }

    #[test]
    fn internal_errors_carry_the_internal_code() {
        assert_eq!(BusError::internal("x").code, BusErrorCode::Internal);
    }
}
