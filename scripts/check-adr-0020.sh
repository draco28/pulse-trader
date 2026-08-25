#!/usr/bin/env bash
# AC-1 — ADR-0020 content gate (r1.s1.w1).
#
# This script asserts CONTENT, not existence. The first draft of this work item's
# criteria proved only that files existed, which a stub ADR would have satisfied.
# Every check below reads a specific claim out of a specific section and fails when
# the claim is absent, so a malformed or empty ADR-0020 cannot pass.
#
# What it asserts:
#   1. ADR-0020 exists and its `Status` section is exactly `Proposed` (it is flipped
#      to `Accepted` at spine close, by the close ceremony, never by the implementer).
#   2. Its `Decision` section names Tauri v2, React, Vite, TypeScript, WKWebView,
#      `tauri-specta`, and the step-1 executable topology (single binary, argv dispatch).
#   3. Its `Consequences` section names BOTH recorded risks by name: the
#      "WKWebView is not Chromium" rendering risk and the fixed 1440x900 canvas.
#   4. It carries the MADR-lite section set the 0001-0019 series uses.
#   5. The CLASS sweep, not just the instance:
#      - ADR-0001's decision-queue entry for ADR-0003 now points at ADR-0020.
#      - ADR-0019's "Deliberately out of scope" section no longer claims the desktop
#        shell is undecided.
#
# Exit 0 iff every assertion holds; exit 1 listing every failure (it does NOT halt
# on the first one -- a single run should tell you everything that is wrong).

set -uo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
adr_0020="$repo_root/docs/adr/0020-desktop-shell-tauri-react.md"
adr_0001="$repo_root/docs/adr/0001-record-architecture-decisions.md"
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

# --- 1. ADR-0020 exists and is non-trivial ----------------------------------
if [[ ! -f "$adr_0020" ]]; then
  echo "FAIL: ADR-0020 is missing at $adr_0020" >&2
  echo "check-adr-0020: 1 failure(s)" >&2
  exit 1
fi

if [[ "$(wc -c <"$adr_0020")" -lt 1500 ]]; then
  fail "ADR-0020 is under 1500 bytes -- that is a stub, not a decision record"
fi

# --- 2. MADR-lite section set ------------------------------------------------
for heading in "Status" "Context" "Decision" "Consequences" "Alternatives considered"; do
  if ! grep -qx "## $heading" "$adr_0020"; then
    fail "ADR-0020 is missing the '## $heading' section (MADR-lite shape)"
  fi
done

# --- 3. Status is exactly Proposed -------------------------------------------
status_body="$(section "$adr_0020" "Status")"
status_word="$(printf '%s\n' "$status_body" | grep -m1 -E '^[A-Za-z]+' | tr -d '[:space:]')"
if [[ "$status_word" != "Proposed" ]]; then
  fail "ADR-0020 Status is '${status_word:-<empty>}', expected exactly 'Proposed' (spine close flips it to Accepted)"
fi

# --- 4. Decision names the stack + the executable topology --------------------
decision_body="$(section "$adr_0020" "Decision")"
if [[ -z "$decision_body" ]]; then
  fail "ADR-0020 has an empty '## Decision' section"
fi

decision_requires=(
  "Tauri v2:Tauri v2"
  "React:React"
  "Vite:Vite"
  "TypeScript:TypeScript"
  "WKWebView:WKWebView"
  "tauri-specta:tauri-specta"
)
for pair in "${decision_requires[@]}"; do
  label="${pair%%:*}"; needle="${pair#*:}"
  if ! printf '%s\n' "$decision_body" | grep -qiF -- "$needle"; then
    fail "ADR-0020 '## Decision' does not name $label"
  fi
done

# The step-1 executable topology: one binary that dispatches on argv. Assert the
# CONCEPT is present (single binary + argv dispatch), not one exact phrasing.
if ! printf '%s\n' "$decision_body" | grep -qiE 'single (binary|executable)|one (binary|executable)'; then
  fail "ADR-0020 '## Decision' does not record the single-binary executable topology (step 1)"
fi
if ! printf '%s\n' "$decision_body" | grep -qiE 'argv|argument'; then
  fail "ADR-0020 '## Decision' does not record how the launch selects GUI vs CLI (argv dispatch)"
fi
if ! printf '%s\n' "$decision_body" | grep -qiE 'ADR-0015|one shippable artifact'; then
  fail "ADR-0020 '## Decision' does not reconcile the topology with ADR-0015's one-artifact rule"
fi

# --- 5. Consequences names BOTH recorded risks --------------------------------
consequences_body="$(section "$adr_0020" "Consequences")"
if [[ -z "$consequences_body" ]]; then
  fail "ADR-0020 has an empty '## Consequences' section"
fi
if ! printf '%s\n' "$consequences_body" | grep -qiE 'WKWebView is not Chromium'; then
  fail "ADR-0020 '## Consequences' does not record the 'WKWebView is not Chromium' risk by name"
fi
if ! printf '%s\n' "$consequences_body" | grep -qiE '1440'; then
  fail "ADR-0020 '## Consequences' does not record the fixed 1440x900 canvas risk"
fi

# --- 6. Class sweep: ADR-0001's queue entry for ADR-0003 points at ADR-0020 ----
adr_0003_entry="$(grep -n 'ADR-0003 — Single Tauri desktop shell' "$adr_0001" | head -1 | cut -d: -f1)"
if [[ -z "$adr_0003_entry" ]]; then
  fail "ADR-0001 no longer has the 'ADR-0003 — Single Tauri desktop shell' queue entry to sweep"
else
  entry_text="$(sed -n "${adr_0003_entry}p" "$adr_0001")"
  if ! printf '%s\n' "$entry_text" | grep -qF 'ADR-0020'; then
    fail "ADR-0001's ADR-0003 decision-queue entry does not point at ADR-0020 (class sweep, spec step 2)"
  fi
  if printf '%s\n' "$entry_text" | grep -qE '\*Proposed\.\*'; then
    fail "ADR-0001's ADR-0003 entry still reads '*Proposed.*' -- it is superseded by ADR-0020"
  fi
fi

# --- 7. Class sweep: ADR-0019 stops calling the shell undecided ----------------
oos_body="$(section "$adr_0019" "Deliberately out of scope")"
if [[ -z "$oos_body" ]]; then
  fail "ADR-0019 has no '## Deliberately out of scope' section to sweep"
else
  shell_bullet="$(printf '%s\n' "$oos_body" | grep -A 12 -iE 'Tauri/TypeScript desktop shell')"
  if [[ -z "$shell_bullet" ]]; then
    fail "ADR-0019's out-of-scope section no longer names the Tauri/TypeScript desktop shell"
  else
    if ! printf '%s\n' "$shell_bullet" | grep -qF 'ADR-0020'; then
      fail "ADR-0019's desktop-shell bullet does not point at ADR-0020 (it still reads as undecided)"
    fi
    if printf '%s\n' "$shell_bullet" | grep -qiE 'a direction, not a settled contract'; then
      fail "ADR-0019 still claims the desktop shell is 'a direction, not a settled contract' -- ADR-0020 decides it"
    fi
    if printf '%s\n' "$shell_bullet" | grep -qiE 'None of it exists'; then
      fail "ADR-0019 still claims 'None of it exists' for the desktop shell -- r1.s1.w1 builds it"
    fi
  fi
fi

# --- report -------------------------------------------------------------------
if ((${#failures[@]} > 0)); then
  printf 'FAIL: %s\n' "${failures[@]}" >&2
  echo "check-adr-0020: ${#failures[@]} failure(s)" >&2
  exit 1
fi

echo "check-adr-0020: OK (ADR-0020 Proposed, decision + risks recorded, ADR-0001/ADR-0019 swept)"
