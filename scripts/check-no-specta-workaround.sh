#!/usr/bin/env bash
# d5 — no specta/tauri-specta generator workaround remains (r1.s5).
#
# The demo line this script binds asserts that the workspace carries NO
# pinned-version workaround of the kind `r1.s5.w2` deleted: `w1` had pinned
# specta `2.0.0-rc.22` because rc.24+ needs `core::fmt::from_fn` (unstable on
# Rust 1.92.0), and rc.21's generator emitted a `TAURI_CHANNEL` type that was
# both imported and locally declared — which `post_process_bindings` repaired
# in post. `w2` bumped the toolchain (ADR-0022) and the specta trio (rc.25),
# read the raw generator output, found both defects absent, and deleted
# `post_process_bindings` outright. This gate fails if any of that ever
# regresses:
#
#   1. `post_process_bindings` (the deleted repair pass) is absent from `src/`.
#   2. The generator-output repair is not re-introduced under another name
#      (a post-write transform of `ui/src/bindings.ts` in build scripts).
#   3. The specta trio is not re-pinned to the pre-bump versions (rc.21/rc.22),
#      which could only mean the workaround came back with it.
#
# Anchored greps throughout: the unanchored `specta =` pattern once matched
# `tauri-specta`'s line and produced a false AC failure (r1.s5.w2 report §7).
#
# Exit 0 iff every assertion holds; exit 1 listing every failure.

set -uo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

failures=()

# --- 1. The deleted repair pass stays deleted ---------------------------------
# Comments are blanked first (the w1 lesson): src/tauri/mod.rs's own doc comment
# documents the deletion and would otherwise fire on the word it explains.
src_code="$(find "$repo_root/src" -name '*.rs' -exec sed 's|^\s*//.*||' {} +)"
if grep -n "post_process_bindings" <<<"$src_code" >/dev/null 2>&1; then
  failures+=("post_process_bindings reappears in src/ code -- the deleted generator repair came back")
fi

# --- 2. No post-write transform of the generated bindings ----------------------
# `ui/src/bindings.ts` must be the generator's raw output: nothing in build
# scripts or build.rs may rewrite it after export (the failure mode the deleted
# function existed for). A legitimate regeneration invocation is fine; a
# sed/patch/transform of the file is not. Matched in BOTH token orders --
# `bindings.ts ... sed` and `sed ... bindings.ts` -- since a real reintroduction
# is just as likely to put the transform verb first (e.g. `sed -i '...'
# ui/src/bindings.ts`). This script's own basename is excluded from the scan:
# its match target below necessarily contains both `bindings.ts` and the verb
# words as string literals, which would otherwise make the gate match itself.
if grep -rnE --exclude="$(basename "${BASH_SOURCE[0]}")" \
  "(bindings\.ts.*(sed|perl|patch|post_process|transform)|(sed|perl|patch|post_process|transform).*bindings\.ts)" \
  "$repo_root/build.rs" "$repo_root/src" "$repo_root/scripts" >/dev/null 2>&1; then
  failures+=("a post-write transform of ui/src/bindings.ts reappears in build scripts -- bindings must be the raw generator output")
fi

# --- 3. The specta trio is not re-pinned to the workaround era -----------------
# rc.21/rc.22 pins are only reachable by undoing w2's bump; anchored so a
# `tauri-specta =` line cannot satisfy the `specta =` check (r1.s5.w2 report §7).
for pin in 'specta = { version = "=2.0.0-rc.2[12]"' 'tauri-specta = { version = "=2.0.0-rc.2[12]"'; do
  if grep -qE "^${pin}" "$repo_root/Cargo.toml"; then
    failures+=("Cargo.toml re-pins the workaround-era version: ${pin}")
  fi
done
if ! grep -qE '^specta = \{ version = "=2\.0\.0-rc\.2[4-9]' "$repo_root/Cargo.toml"; then
  failures+=("Cargo.toml does not pin specta to a from_fn-era rc (rc.24+) -- the toolchain-bump premise of ADR-0022 is not holding")
fi

# --- report -------------------------------------------------------------------
if ((${#failures[@]} > 0)); then
  printf 'FAIL: %s\n' "${failures[@]}" >&2
  echo "check-no-specta-workaround: ${#failures[@]} failure(s)" >&2
  exit 1
fi

echo "check-no-specta-workaround: OK (no generator workaround, no repair pass, no pre-bump pins)"
