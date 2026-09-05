#!/usr/bin/env bash
# AC-2 — the coach-boundary content gate (r1.s4.w1, #131 / #132).
#
# The seal r1.s4.w1 puts on the coach turn is a property of the SOURCE, and source
# properties decay silently: a later item adds one `pub`, re-points one caller at
# `save_session`, or advertises the honesty tool on its own, and nothing fails. Every
# assertion below is one of those decays, made loud. The check-adr-0021.sh discipline
# applies: assert the PROPERTY, not one phrasing of it, and report every failure in
# one run rather than halting on the first.
#
# What it asserts:
#
#   1. THE FRAGMENT SURFACE IS GONE. No `Coach::new` and no `Coach::run_turn` is
#      reachable from outside the crate — neither as a `pub` method nor through a
#      `lib.rs` re-export of the bare `Coach` type. That surface is what #132
#      enumerated six false-but-individually-valid audit rows against: a run and a
#      version that never met, a session naming another turn's ledger row, a turn
#      written only after the call.
#   2. `save_session` HAS NO PRODUCTION CALLER. It survives with its initial-only
#      contract for the repository's own tests, but production creates a turn by
#      CLAIMING it before the call and settling it once (`finish_session`). A
#      production caller of `save_session` would be a turn with no claim behind it —
#      exactly the crash window migration 0008 exists to close.
#   3. THE TWO TOOLS ARE ADVERTISED TOGETHER. `record_inapplicable` on its own would
#      be a coach that can only decline; `propose_mutation` on its own is the
#      pre-#131 state in which structural advice had to be approximated with the
#      nearest parameter. One advertisement site, both tools, in that order.
#   4. Positive controls, so a gate reading an empty or renamed file cannot pass
#      vacuously: the sealed module exists, exposes `run_coach_turn`, and settles
#      through `finish_session`.
#
# Exit 0 iff every assertion holds; exit 1 listing every failure.

set -uo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root" || exit 1

sealed="src/application/coach.rs"
tools="src/agent/tools.rs"

failures=()
fail() { failures+=("$1"); }

# Every Rust source file under src/, NUL-free paths (this tree has none).
rust_sources() {
  find src -name '*.rs' -type f | sort
}

# A file's code with line comments blanked, so prose naming a symbol never trips a
# code assertion (the `blank_comments` discipline the source-scan tests use).
code_of() {
  sed -E 's://.*$::' "$1"
}

# --- 1. the fragment surface is gone -----------------------------------------

# 1a. No PUBLIC `run_turn`. `pub(crate)` / `pub(super)` are fine — the spec allows
# the fragments to be narrowed rather than deleted; what may not exist is a caller
# outside this crate.
while IFS= read -r file; do
  if code_of "$file" | grep -qE '\bpub[[:space:]]+(async[[:space:]]+)?fn[[:space:]]+run_turn\b'; then
    fail "$file declares a PUBLIC \`run_turn\` — the coach turn is reached through the sealed application module, not through an exported method (#132)"
  fi
done < <(rust_sources)

# 1b. No PUBLIC `new` inside an `impl ... Coach ...` block. The block is tracked by
# brace depth rather than by a line-local grep, because `pub fn new` is the most
# common line in the tree and only its ADDRESS makes it a violation.
while IFS= read -r file; do
  offenders="$(code_of "$file" | awk '
    # Enter an impl block whose self type is the bare `Coach` (`Coach<P>`, `Coach`).
    /^[[:space:]]*impl[[:space:]]/ && /[[:space:]]Coach[[:space:]<{]/ { inside = 1; depth = 0 }
    inside {
      n = gsub(/\{/, "{"); depth += n
      n = gsub(/\}/, "}"); depth -= n
      if (depth <= 0 && seen_open) { inside = 0; seen_open = 0 }
      else if (depth > 0) { seen_open = 1 }
      if (inside && $0 ~ /[[:space:]]*pub[[:space:]]+(async[[:space:]]+)?fn[[:space:]]+new[[:space:]]*[(<]/) { print NR }
    }
  ')"
  if [[ -n "$offenders" ]]; then
    fail "$file declares a PUBLIC \`Coach::new\` (line(s): $(echo "$offenders" | tr '\n' ' ')) — a caller that can build the turn's fragments can build a false audit row (#132)"
  fi
done < <(rust_sources)

# 1c. `lib.rs` does not re-export the bare `Coach` type. `CoachTurnError`,
# `CoachTurnRegistry`, `CoachWiring` and friends are deliberately fine: they are the
# turn's error, its registry handle and the composition root's wiring, none of which
# can assemble a turn by hand.
#
# Each `pub use` is JOINED to its terminating `;` before the match, because the
# braced form spans lines:
#
#     pub use agent::{
#         Coach,          <- not on a line beginning `pub use`
#     };
#
# Matching line-by-line reads that as no re-export at all, so the gate passes on
# exactly the shape a re-export is most likely to take.
pub_use_statements() {
  code_of src/lib.rs | awk '
    /^[[:space:]]*pub[[:space:]]+use/ { collecting = 1; stmt = "" }
    collecting { stmt = stmt " " $0 }
    collecting && /;/ { print stmt; collecting = 0 }
  '
}
if pub_use_statements | grep -qE '(^|[^A-Za-z0-9_])Coach([^A-Za-z0-9_]|$)'; then
  fail "src/lib.rs re-exports the bare \`Coach\` type — the sealed turn is reached through \`run_coach_with\`, and an exported \`Coach\` restores the fragment surface #132 names"
fi

# --- 2. save_session has no production caller ---------------------------------

# The port declares it, the two repository adapters implement it, and the repository
# tests drive it. Anything else calling it is a production write that skipped the
# claim.
allowed_save_session_regex='^(src/domain/port\.rs|src/adapters/db/coaching_repo\.rs|src/adapters/memory\.rs)$'
while IFS= read -r file; do
  [[ "$file" =~ $allowed_save_session_regex ]] && continue
  # A CALL, not a mention: `.save_session(` / `sessions.save_session(`.
  if code_of "$file" | grep -qE '\.save_session[[:space:]]*\('; then
    fail "$file calls \`save_session\` — production creates a turn with claim_session + finish_session, so a save_session caller is a turn with no claim behind it (r1.s4.w4 / migration 0008)"
  fi
done < <(rust_sources)

# --- 3. the two tools are advertised together ---------------------------------

if [[ ! -f "$tools" ]]; then
  fail "$tools is missing — the coach's tool definitions have moved, and this gate can no longer see them"
else
  for needle in \
    'PROPOSE_MUTATION_TOOL:[[:space:]]*&str[[:space:]]*=[[:space:]]*"propose_mutation"' \
    'RECORD_INAPPLICABLE_TOOL:[[:space:]]*&str[[:space:]]*=[[:space:]]*"record_inapplicable"'
  do
    if ! code_of "$tools" | grep -qE "$needle"; then
      fail "$tools no longer defines the tool-name constant matching /$needle/ — both tools are part of the #131 honesty protocol"
    fi
  done

  # The ONE advertisement site must name both definitions. Read the body of
  # `coach_tool_definitions` rather than the whole file, so a definition that exists
  # but is never advertised still fails.
  advertised="$(code_of "$tools" | awk '
    /fn[[:space:]]+coach_tool_definitions/ { inside = 1 }
    inside { print }
    inside && /^\}/ { exit }
  ')"
  if [[ -z "$advertised" ]]; then
    fail "$tools has no \`coach_tool_definitions\` — the coach advertises its tools in exactly one place, and this gate reads that place"
  else
    if ! printf '%s\n' "$advertised" | grep -q 'def_propose_mutation'; then
      fail "\`coach_tool_definitions\` does not advertise \`propose_mutation\` — a coach that can only record inapplicability proposes nothing"
    fi
    if ! printf '%s\n' "$advertised" | grep -q 'def_record_inapplicable'; then
      fail "\`coach_tool_definitions\` does not advertise \`record_inapplicable\` — without it, structural advice has to be approximated by the nearest parameter, which is the #131 dishonesty"
    fi
  fi
fi

# --- 4. positive controls ------------------------------------------------------

if [[ ! -f "$sealed" ]]; then
  fail "$sealed is missing — the sealed coach turn is the thing every assertion above is about"
else
  if ! code_of "$sealed" | grep -qE '\bpub\(crate\)[[:space:]]+async[[:space:]]+fn[[:space:]]+run_coach_turn\b'; then
    fail "$sealed does not expose a crate-private \`run_coach_turn\` — either the entry point moved or it stopped being crate-private"
  fi
  if ! code_of "$sealed" | grep -qE '\.finish_session[[:space:]]*\('; then
    fail "$sealed never calls \`finish_session\` — the claim it commits before the provider call has to be settled through the one settle path"
  fi
  if ! code_of "$sealed" | grep -qE '\.claim_session[[:space:]]*\('; then
    fail "$sealed never calls \`claim_session\` — the turn must claim the session before any provider I/O"
  fi
fi

# --- report --------------------------------------------------------------------

if ((${#failures[@]} > 0)); then
  printf 'FAIL: %s\n' "${failures[@]}" >&2
  echo "check-coach-boundary: ${#failures[@]} failure(s)" >&2
  exit 1
fi

echo "check-coach-boundary: OK (no public Coach::new/run_turn, no production save_session caller, both coach tools advertised together, sealed run_coach_turn claims and finishes)"
