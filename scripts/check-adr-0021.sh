#!/usr/bin/env bash
# AC-1 — ADR-0021 content gate (r1.s2.w1).
#
# This script asserts CONTENT, not existence — the check-adr-0020.sh precedent.
# ADR-0021 is the decision spine r1.s2 owes, and w2/w3 implement against it, so a
# stub would be worse than nothing: it would look decided. Every check below reads
# a specific claim out of a specific section and fails when the claim is absent.
#
# What it asserts:
#   1. ADR-0021 exists at docs/adr/0021-coach-agent-one-mutation-framework.md and
#      is not a stub.
#   2. It carries the MADR-lite section set the 0001-0023 series uses.
#   3. Its `Status` section is exactly `Proposed` — authored `Proposed` as this
#      spine's first deliverable (the class-declaration rung-3 rule), flipped to
#      `Accepted` at r1.s2's close. The flip and this gate's update are made in the
#      same act, per the bones protocol (check-adr-0020.sh set that precedent).
#   4. Its `Decision` section names the SIX decided points the spine owes, so that
#      w2 and w3 implement a decided design rather than re-deciding one:
#        (a) the parameter-only typed mutation vocabulary (grill L1),
#        (b) the apply -> validate -> compile pipeline,
#        (c) migration 0005's schema shape incl. the dormant disposition columns
#            for r1.s4 (grill L2),
#        (d) one provider call per turn with terminal typed failures (grill L3),
#        (e) the bounded CoachContext projection (grill L4 as amended by audit C1),
#        (f) mutation validity is established at use-time by apply() and is never
#            persisted (audit C4),
#      plus (g) the C6 path-grammar reuse of validate.rs's dotted/indexed locators.
#   5. The number 0021 is occupied by exactly ONE file — the reserved number was
#      held for this decision and nothing else may take it.
#
# Exit 0 iff every assertion holds; exit 1 listing every failure (it does NOT halt
# on the first one -- a single run should tell you everything that is wrong).

set -uo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
adr_0021="$repo_root/docs/adr/0021-coach-agent-one-mutation-framework.md"

failures=()

fail() { failures+=("$1"); }

# Print the body of a `## <name>` section, up to the next `## ` heading.
section() {
  local file="$1" name="$2"
  awk -v want="## $name" '
    $0 == want { inside = 1; next }
    inside && /^## / { exit }
    inside { print }
  ' "$file"
}

# --- 1. ADR-0021 exists and is non-trivial ----------------------------------
if [[ ! -f "$adr_0021" ]]; then
  echo "FAIL: ADR-0021 is missing at $adr_0021" >&2
  echo "check-adr-0021: 1 failure(s)" >&2
  exit 1
fi

if [[ "$(wc -c <"$adr_0021")" -lt 1500 ]]; then
  fail "ADR-0021 is under 1500 bytes -- that is a stub, not a decision record"
fi

# --- 2. MADR-lite section set ------------------------------------------------
for heading in "Status" "Context" "Decision" "Consequences" "Alternatives considered"; do
  if ! grep -qx "## $heading" "$adr_0021"; then
    fail "ADR-0021 is missing the '## $heading' section (MADR-lite shape)"
  fi
done

# --- 3. Status is exactly Proposed --------------------------------------------
status_body="$(section "$adr_0021" "Status")"
status_word="$(printf '%s\n' "$status_body" | grep -m1 -E '^[A-Za-z]+' | tr -d '[:space:]')"
if [[ "$status_word" != "Proposed" ]]; then
  fail "ADR-0021 Status is '${status_word:-<empty>}', expected exactly 'Proposed' (r1.s2's close is the only thing that may flip it to Accepted, and it updates this gate in the same act)"
fi

# --- 4. Decision names the six decided points + the C6 grammar reuse ----------
decision_body="$(section "$adr_0021" "Decision")"
if [[ -z "$decision_body" ]]; then
  fail "ADR-0021 has an empty '## Decision' section"
fi

# Each entry is "<label>:<extended regex>". The regexes assert the CONCEPT is
# recorded, not one exact phrasing -- the check-adr-0020.sh discipline.
decision_requires=(
  "(a) the parameter-only mutation vocabulary (L1):SetParam"
  "(a) the parameter-only mutation vocabulary (L1):parameter-only"
  "(b) the apply -> validate -> compile pipeline:apply.{0,12}validate.{0,12}compile"
  "(c) migration 0005's schema shape (L2):0005"
  "(c) migration 0005's dormant disposition columns for r1.s4 (L2):disposition"
  "(d) one provider call per coach turn (L3):one (provider )?call"
  "(d) terminal typed failures, never silence (L3):terminal"
  "(e) the bounded CoachContext projection (L4/C1):CoachContext"
  "(f) use-time validity, never persisted (C4):use-time"
  "(g) the C6 reuse of validate.rs's locator grammar:validate\.rs"
)
for pair in "${decision_requires[@]}"; do
  label="${pair%%:*}"; needle="${pair#*:}"
  if ! printf '%s\n' "$decision_body" | grep -qiE -- "$needle"; then
    fail "ADR-0021 '## Decision' does not record $label"
  fi
done

# --- 5. The reserved number 0021 is occupied by exactly one file ---------------
occupants="$(find "$repo_root/docs/adr" -maxdepth 1 -name '0021-*.md' | wc -l | tr -d '[:space:]')"
if [[ "$occupants" != "1" ]]; then
  fail "docs/adr has $occupants files numbered 0021; the reserved number must be taken by ADR-0021 alone"
fi

# --- report -------------------------------------------------------------------
if ((${#failures[@]} > 0)); then
  printf 'FAIL: %s\n' "${failures[@]}" >&2
  echo "check-adr-0021: ${#failures[@]} failure(s)" >&2
  exit 1
fi

echo "check-adr-0021: OK (ADR-0021 Proposed — six decided points + the C6 grammar reuse recorded, number 0021 uniquely occupied)"
