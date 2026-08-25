#!/usr/bin/env bash
# AC-2 — ADR-0022 content gate (r1.s5.w1).
#
# This script asserts CONTENT, not existence — mirroring check-adr-0020.sh, the
# precedent this file follows line for line. Every check below reads a specific
# claim out of a specific section and fails when the claim is absent, so a
# malformed or empty ADR-0022 cannot pass.
#
# What it asserts:
#   1. ADR-0022 exists and its `Status` section is exactly `Proposed` (spine close
#      flips it to `Accepted`, never the implementer).
#   2. Its `Decision` section names `1.98.0`.
#   3. Its `Consequences` section states the fingerprint moves AND that `compare`
#      is a WARNING, never an error — the claim this spine exists to get right,
#      and the one an earlier draft of the plan got backwards.
#   4. Its `Alternatives considered` section names all three rejected options:
#      staying on 1.92.0, pinning 1.96.0, and floating `stable`.
#   5. It carries the MADR-lite section set the 0001-0021 series uses.
#   6. The CLASS sweep, not just the instance: ADR-0019's `Consequences` section
#      now points at ADR-0022 and no longer names `1.92.0`.
#
# Exit 0 iff every assertion holds; exit 1 listing every failure (it does NOT halt
# on the first one -- a single run should tell you everything that is wrong).

set -uo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
adr_0022="$repo_root/docs/adr/0022-toolchain-bump-1-98.md"
adr_0019="$repo_root/docs/adr/0019-stack-rust-core-embedded-sqlite-and-parquet.md"

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

# --- 1. ADR-0022 exists and is non-trivial ----------------------------------
if [[ ! -f "$adr_0022" ]]; then
  echo "FAIL: ADR-0022 is missing at $adr_0022" >&2
  echo "check-adr-0022: 1 failure(s)" >&2
  exit 1
fi

if [[ "$(wc -c <"$adr_0022")" -lt 1500 ]]; then
  fail "ADR-0022 is under 1500 bytes -- that is a stub, not a decision record"
fi

# --- 2. MADR-lite section set ------------------------------------------------
for heading in "Status" "Context" "Decision" "Consequences" "Alternatives considered"; do
  if ! grep -qx "## $heading" "$adr_0022"; then
    fail "ADR-0022 is missing the '## $heading' section (MADR-lite shape)"
  fi
done

# --- 3. Status is exactly Proposed -------------------------------------------
status_body="$(section "$adr_0022" "Status")"
status_word="$(printf '%s\n' "$status_body" | grep -m1 -E '^[A-Za-z]+' | tr -d '[:space:]')"
if [[ "$status_word" != "Proposed" ]]; then
  fail "ADR-0022 Status is '${status_word:-<empty>}', expected exactly 'Proposed' (spine close flips it to Accepted)"
fi

# --- 4. Decision names 1.98.0 -------------------------------------------------
decision_body="$(section "$adr_0022" "Decision")"
if [[ -z "$decision_body" ]]; then
  fail "ADR-0022 has an empty '## Decision' section"
fi
if ! printf '%s\n' "$decision_body" | grep -qF '1.98.0'; then
  fail "ADR-0022 '## Decision' does not name 1.98.0"
fi

# --- 5. Consequences: fingerprint moves AND compare is a warning, not an error -
consequences_body="$(section "$adr_0022" "Consequences")"
if [[ -z "$consequences_body" ]]; then
  fail "ADR-0022 has an empty '## Consequences' section"
fi
if ! printf '%s\n' "$consequences_body" | grep -qiE 'fingerprint moves'; then
  fail "ADR-0022 '## Consequences' does not state that the fingerprint moves"
fi
if ! printf '%s\n' "$consequences_body" | grep -qiE 'never an .Err.|never.*error|warning.*never.*error|not.*more than a WARNING'; then
  fail "ADR-0022 '## Consequences' does not record that EngineFingerprint::compare is a warning, never an error"
fi
if ! printf '%s\n' "$consequences_body" | grep -qiE 'Option<String>'; then
  fail "ADR-0022 '## Consequences' does not cite compare's Option<String> return type"
fi

# --- 6. Alternatives considered names all three rejected options --------------
alternatives_body="$(section "$adr_0022" "Alternatives considered")"
if [[ -z "$alternatives_body" ]]; then
  fail "ADR-0022 has an empty '## Alternatives considered' section"
fi
if ! printf '%s\n' "$alternatives_body" | grep -qF '1.92.0'; then
  fail "ADR-0022 '## Alternatives considered' does not name staying on 1.92.0"
fi
if ! printf '%s\n' "$alternatives_body" | grep -qF '1.96.0'; then
  fail "ADR-0022 '## Alternatives considered' does not name pinning 1.96.0"
fi
if ! printf '%s\n' "$alternatives_body" | grep -qiE 'float.*stable|floating.*stable|stable.*channel'; then
  fail "ADR-0022 '## Alternatives considered' does not name floating stable"
fi

# --- 7. Class sweep: ADR-0019's Consequences points at ADR-0022, no 1.92.0 ----
if [[ ! -f "$adr_0019" ]]; then
  fail "ADR-0019 is missing at $adr_0019 -- cannot verify the class sweep"
else
  adr_0019_consequences="$(section "$adr_0019" "Consequences")"
  if [[ -z "$adr_0019_consequences" ]]; then
    fail "ADR-0019 has no '## Consequences' section to sweep"
  else
    if ! printf '%s\n' "$adr_0019_consequences" | grep -qF 'ADR-0022'; then
      fail "ADR-0019's '## Consequences' does not point at ADR-0022 (class sweep, spec step 6)"
    fi
  fi
  if grep -qF '1.92.0' "$adr_0019"; then
    fail "ADR-0019 still names 1.92.0 somewhere -- the stale version should be gone or superseded by ADR-0022"
  fi
fi

# --- report -------------------------------------------------------------------
if ((${#failures[@]} > 0)); then
  printf 'FAIL: %s\n' "${failures[@]}" >&2
  echo "check-adr-0022: ${#failures[@]} failure(s)" >&2
  exit 1
fi

echo "check-adr-0022: OK (ADR-0022 Proposed, decision/consequences/alternatives recorded, ADR-0019 swept)"
