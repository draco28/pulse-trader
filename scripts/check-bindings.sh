#!/usr/bin/env bash
# AC-8 — generated-bindings freshness gate (r1.s1.w1, ADR-0020 step 6).
#
# `ui/src/bindings.ts` is the tauri-specta-generated typed seam between the Rust command
# bus and the frontend, and it is **committed**. This script regenerates it into a
# temporary file and fails on ANY diff.
#
# Why committed-plus-diffed rather than generated-at-build-time: the frontend's
# typecheck, a fresh clone and an editor's language server all need the file to exist
# without a Rust build having run first. Committing it buys that; diffing it is what
# stops the committed copy from silently going stale. A command added or a bus type
# changed without regenerating is then a failing check with a READABLE DIFF, rather
# than a `TypeError` the first time someone opens that screen.
#
# It also asserts the file is actually TRACKED by git -- a bindings file that is
# gitignored would pass a diff check on the developer's machine and not exist in CI.
#
# Exit 0 iff the committed bindings match a fresh generation; exit 1 with the diff.

set -uo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bindings="$repo_root/ui/src/bindings.ts"
rel_bindings="ui/src/bindings.ts"

failures=0
note_failure() { failures=$((failures + 1)); }

tmp_dir="$(mktemp -d)"
cleanup() { rm -rf "$tmp_dir"; }
trap cleanup EXIT

generated="$tmp_dir/bindings.ts"

# --- 1. Regenerate ----------------------------------------------------------
# `--quiet` keeps cargo's progress out of the log; the example prints its own line to
# stderr. A build failure here is a real failure: bindings that cannot be generated are
# bindings nobody can refresh.
if ! (cd "$repo_root" && cargo run --quiet --example export-bindings -- "$generated") 2>&1; then
  echo "FAIL: could not regenerate the bindings (cargo run --example export-bindings failed)" >&2
  echo "check-bindings: 1 failure(s)" >&2
  exit 1
fi

if [[ ! -s "$generated" ]]; then
  echo "FAIL: the bindings exporter produced an empty file -- that would make any" >&2
  echo "      comparison vacuous, so it is treated as a failure rather than a match" >&2
  echo "check-bindings: 1 failure(s)" >&2
  exit 1
fi

# --- 2. The committed file exists -------------------------------------------
if [[ ! -f "$bindings" ]]; then
  echo "FAIL: $rel_bindings does not exist. It is a COMMITTED artifact." >&2
  echo "      Generate it with:" >&2
  echo "        cargo run --quiet --example export-bindings -- $rel_bindings" >&2
  echo "check-bindings: 1 failure(s)" >&2
  exit 1
fi

# --- 3. Tracked by git, not ignored -----------------------------------------
if command -v git >/dev/null 2>&1 && git -C "$repo_root" rev-parse --git-dir >/dev/null 2>&1; then
  if git -C "$repo_root" check-ignore -q "$rel_bindings" 2>/dev/null; then
    echo "FAIL: $rel_bindings is gitignored. It must be COMMITTED -- an ignored" >&2
    echo "      bindings file passes this check locally and is absent in CI." >&2
    note_failure
  fi
fi

# --- 4. No diff -------------------------------------------------------------
if ! diff -u "$bindings" "$generated" >"$tmp_dir/diff.txt" 2>&1; then
  echo "FAIL: $rel_bindings is STALE -- regenerating it produces a different file." >&2
  echo "      Refresh it with:" >&2
  echo "        cargo run --quiet --example export-bindings -- $rel_bindings" >&2
  echo "      Diff (committed vs. freshly generated):" >&2
  sed 's/^/      /' "$tmp_dir/diff.txt" >&2
  note_failure
fi

if ((failures > 0)); then
  echo "check-bindings: $failures failure(s)" >&2
  exit 1
fi

echo "check-bindings: OK ($rel_bindings matches a fresh generation, $(wc -l <"$bindings" | tr -d ' ') lines)"
