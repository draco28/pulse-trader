#!/usr/bin/env bash
# The coach rail's round-trip gate (r1.s4.w3) — the spine's `d13` ledger line.
#
# One bounded command that proves the desktop coach rail does the three things
# Release 1 closes on, and exits non-zero the moment any of them stops being true:
#
#   1. **One terminal outcome per turn, never silence.** `tests/tauri_coach.rs`
#      drives the real `coach_turn_core` over a real `DesktopState`, a migrated
#      temp database and the committed candle fixture, with a scripted provider as
#      the only double: a proposal, or one typed failure carrying its named
#      recovery — and a ledger row whose cost and prompt version are the ones the
#      DTO shows.
#   2. **At most one child and one re-backtest per accept.** The same binary
#      accepts, counts the rows, accepts again, and gets the same two ids back.
#   3. **The active operation survives navigation, and never duplicates.** The
#      three UI files below cover the rail's states and `#141`'s reattachment: run,
#      leave, come back, and the same operation is still there — with no second
#      invocation and no second persisted run.
#
# It also re-runs `check-bindings.sh`, because a rail that renders is worth
# nothing if the committed `ui/src/bindings.ts` no longer matches the commands it
# calls: that mismatch is invisible to both suites above and appears as a runtime
# "command not found" the first time a trader opens the Lab.
#
# Bounded on purpose: it runs ONE Rust test binary, THREE UI test files and one
# existing gate — not `just check`. This is a demo-ledger line re-run at every
# future spine close, and a line that takes minutes is a line people stop running.
#
# Offline by construction: no live LLM, no network, no Keychain. The provider is
# scripted in-process and every other layer is real.
#
# Exit 0 iff all four stages pass; exit 1 naming EVERY stage that failed (no halt
# on the first — one run should tell you everything that is wrong).

set -uo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root" || exit 1

failures=()
note_failure() { failures+=("$1"); }

# The three UI files, relative to vitest's root (`ui/`). Named explicitly rather
# than globbed: this gate makes a claim about THESE properties, and a glob would
# quietly widen or narrow it as files are added and renamed.
ui_tests=(
  "src/screens/BacktestLabScreen.test.tsx"
  "src/App.test.tsx"
  "src/hooks/useActiveOperations.test.tsx"
)

for rel in "${ui_tests[@]}"; do
  if [[ ! -f "ui/$rel" ]]; then
    echo "FAIL: ui/$rel does not exist — this gate names its test files explicitly," >&2
    echo "      so a renamed or deleted file is a failure rather than a silent skip" >&2
    note_failure "ui/$rel is missing"
  fi
done

# --- 1. the backend round trip ----------------------------------------------
echo "check-coach-roundtrip: [1/4] cargo test --test tauri_coach"
if ! cargo test --test tauri_coach; then
  note_failure "cargo test --test tauri_coach"
fi

# --- 2. the rail and the #141 reattachment ----------------------------------
echo "check-coach-roundtrip: [2/4] npm run test -- --run (rail + remount)"
if ! npm run test -- --run "${ui_tests[@]}"; then
  note_failure "npm run test -- --run ${ui_tests[*]}"
fi

# --- 3. the generated bindings still match the commands ----------------------
echo "check-coach-roundtrip: [3/4] bash scripts/check-bindings.sh"
if ! bash scripts/check-bindings.sh; then
  note_failure "bash scripts/check-bindings.sh"
fi

# --- 4. the two commands are actually registered -----------------------------
# A cheap structural backstop over the two stages above: both suites can pass
# while a command is registered in one list and not the other, and the failure
# mode of that is a screen that invokes into nothing.
echo "check-coach-roundtrip: [4/4] the two coach commands are registered"
for command_name in coach_turn coach_decide; do
  if ! grep -q "\"$command_name\"," src/tauri/commands.rs; then
    note_failure "BUS_COMMANDS does not list $command_name"
  fi
  if ! grep -q "commands::$command_name," src/tauri/mod.rs; then
    note_failure "collect_commands! does not register $command_name"
  fi
done

# --- report -------------------------------------------------------------------
if ((${#failures[@]} > 0)); then
  for failure in "${failures[@]}"; do
    echo "FAIL: $failure" >&2
  done
  echo "check-coach-roundtrip: ${#failures[@]} failure(s)" >&2
  exit 1
fi

echo "check-coach-roundtrip: OK (one terminal outcome per turn, at most one child"
echo "  and one re-backtest per accept, the active operation reattaches across"
echo "  navigation, and the committed bindings match the registered commands)"
