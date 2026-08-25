#!/usr/bin/env bash
# AC-2 — the ported design system's own content gate (r1.s1.w5).
#
# Modelled on r1.s1.w1's `check-adr-0020.sh`: assert CONTENT, not existence. The
# planning audit's finding #6 killed three existence-only checks in this spine for
# exactly this reason -- a file containing the word "Status" passed three of them.
# Every check below reads a specific claim, not merely whether a path exists.
#
# What it asserts:
#   1. `ui/src/styles/tokens.css` and `ui/src/styles/shared.css` exist and are
#      NON-TRIVIAL -- a line-count floor near the mock sources' 189 and 325 lines,
#      so an empty or stub file cannot pass.
#   2. The frontend entry point actually imports both, in tokens-then-shared order
#      (the load order `docs/design/mock/README.md` documents).
#   3. NO per-screen stylesheet from the mock was ported -- none of the nine
#      screen-specific `.css` files exist anywhere under `ui/src`.
#   4. `macos-window.jsx` (the abandoned native-chrome exploration ADR-0020 cites)
#      was not ported -- no file by that name, and no reference to it, anywhere
#      under `ui/src`.
#
# Exit 0 iff every assertion holds; exit 1 listing every failure (no halt on the
# first one -- one run should tell you everything that is wrong).

set -uo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if ! command -v python3 >/dev/null 2>&1; then
  echo "FAIL: python3 is required to run check-design-system.sh" >&2
  exit 1
fi

python3 - "$repo_root" <<'PY'
import pathlib
import re
import sys

repo = pathlib.Path(sys.argv[1])
ui_src = repo / "ui" / "src"
failures = []


def fail(msg):
    failures.append(msg)


# --- 1. tokens.css / shared.css exist and are non-trivial -------------------
# Line-count floors near the mock sources (189 / 325 lines) -- comfortably below
# either so a reformat does not false-fail, comfortably above zero so an empty or
# near-empty stub cannot pass.
REQUIRED = {
    "tokens.css": 150,
    "shared.css": 250,
}
styles_dir = ui_src / "styles"
found_line_counts = {}
for name, floor in REQUIRED.items():
    path = styles_dir / name
    if not path.is_file():
        fail(f"ui/src/styles/{name} does not exist -- it must be ported from "
             f"docs/design/mock/{name}")
        continue
    line_count = len(path.read_text().splitlines())
    found_line_counts[name] = line_count
    if line_count < floor:
        fail(f"ui/src/styles/{name} has only {line_count} lines (floor {floor}) -- "
             "that reads as a stub, not the ported stylesheet")

# --- 2. the entry point imports both, tokens before shared -------------------
entry_candidates = [ui_src / "main.tsx", ui_src / "main.ts"]
entry = next((p for p in entry_candidates if p.is_file()), None)
if entry is None:
    fail("no ui/src/main.tsx (or main.ts) entry point found to import the styles")
else:
    entry_text = entry.read_text()
    tokens_idx = entry_text.find("styles/tokens.css")
    shared_idx = entry_text.find("styles/shared.css")
    if tokens_idx == -1:
        fail(f"{entry.relative_to(repo)} does not import styles/tokens.css")
    if shared_idx == -1:
        fail(f"{entry.relative_to(repo)} does not import styles/shared.css")
    if tokens_idx != -1 and shared_idx != -1 and tokens_idx > shared_idx:
        fail(f"{entry.relative_to(repo)} imports shared.css before tokens.css -- "
             "the mock's own README pins tokens.css -> shared.css as the load order")

# --- 3. no per-screen stylesheet from the mock was ported --------------------
FORBIDDEN_SCREEN_SHEETS = [
    "strategy-library.css",
    "strategy-designer.css",
    "backtest-lab.css",
    "deployment-dashboard.css",
    "trade-journal.css",
    "analytics.css",
    "onboarding.css",
    "settings.css",
    "command-palette.css",
]
if ui_src.is_dir():
    present = {p.name for p in ui_src.rglob("*.css")}
    for forbidden in FORBIDDEN_SCREEN_SHEETS:
        if forbidden in present:
            fail(f"ui/src contains {forbidden} -- no per-screen stylesheet from the "
                 "mock may be ported by this item (w3/w4's screens own their own)")

# --- 4. macos-window.jsx (the abandoned exploration) was not ported ----------
if ui_src.is_dir():
    for path in ui_src.rglob("*"):
        if not path.is_file():
            continue
        if path.suffix not in {".ts", ".tsx", ".js", ".jsx"}:
            continue
        if "macos-window" in path.name.lower() or "macoswindow" in path.name.lower():
            fail(f"{path.relative_to(repo)}: macos-window.jsx must not be ported -- "
                 "it is an abandoned native-chrome exploration ADR-0020 cites as "
                 "unreferenced")
            continue
        text = path.read_text()
        if re.search(r"\bmacos-?window\b", text, re.IGNORECASE):
            fail(f"{path.relative_to(repo)}: references macos-window -- "
                 "that exploration must not be ported")

# --- report -------------------------------------------------------------------
if failures:
    for f in failures:
        print(f"FAIL: {f}", file=sys.stderr)
    print(f"check-design-system: {len(failures)} failure(s)", file=sys.stderr)
    sys.exit(1)

counts = ", ".join(f"{name} {n} lines" for name, n in found_line_counts.items())
print(f"check-design-system: OK ({counts}; entry imports both in order; "
      "no per-screen stylesheet or macos-window.jsx ported)")
PY
