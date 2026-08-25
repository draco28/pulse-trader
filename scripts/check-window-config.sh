#!/usr/bin/env bash
# AC-7 — window configuration gate (r1.s1.w1, ADR-0020 step 3; grill gates G1 / A5).
#
# Two halves, and the second is the one that is easy to lose:
#
#   1. `tauri.conf.json` pins the main window to **1440x900, `resizable: false`**, with
#      an **undecorated** title bar (the app draws its own chrome -- every screen in the
#      mock already does).
#   2. **No scale-transform survived the port.** The mock carried an `installFit()` /
#      inline `fit()` script applying a CSS `transform: scale()` to the whole canvas.
#      That existed because the mock ran in a browser tab whose size it did not control.
#      An application that owns its own window does not need it, and keeping it would
#      blur text and desynchronize hit targets from their painted positions. Responsive
#      layout is DEFERRED with a recorded admission condition (ADR-0020), not SIMULATED
#      by scaling -- so this half asserts the simulation is absent.
#
# Requires: python3. Exit 0 iff every assertion holds; exit 1 listing every failure.

set -uo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if ! command -v python3 >/dev/null 2>&1; then
  echo "FAIL: python3 is required to parse tauri.conf.json" >&2
  exit 1
fi

python3 - "$repo_root" <<'PY'
import json
import pathlib
import re
import sys

repo = pathlib.Path(sys.argv[1])
failures = []

def fail(msg):
    failures.append(msg)

EXPECTED_WIDTH = 1440
EXPECTED_HEIGHT = 900

# --- 1. The window is pinned ------------------------------------------------
conf_path = repo / "tauri.conf.json"
if not conf_path.is_file():
    print(f"FAIL: no tauri.conf.json at {conf_path}", file=sys.stderr)
    print("check-window-config: 1 failure(s)", file=sys.stderr)
    sys.exit(1)

try:
    conf = json.loads(conf_path.read_text())
except json.JSONDecodeError as e:
    print(f"FAIL: tauri.conf.json is not valid JSON: {e}", file=sys.stderr)
    print("check-window-config: 1 failure(s)", file=sys.stderr)
    sys.exit(1)

windows = conf.get("app", {}).get("windows")
if not isinstance(windows, list) or not windows:
    fail("tauri.conf.json declares no `app.windows`")
else:
    main = next((w for w in windows if w.get("label") == "main"), None)
    if main is None:
        fail("tauri.conf.json has no window labelled `main` "
             "(capabilities/default.json scopes its grant to that label)")
    else:
        if main.get("width") != EXPECTED_WIDTH:
            fail(f"main window width is {main.get('width')!r}, expected {EXPECTED_WIDTH} "
                 "(grill G1/A5: the canvas is a fixed 1440x900)")
        if main.get("height") != EXPECTED_HEIGHT:
            fail(f"main window height is {main.get('height')!r}, expected {EXPECTED_HEIGHT} "
                 "(grill G1/A5: the canvas is a fixed 1440x900)")
        if main.get("resizable") is not False:
            fail(f"main window `resizable` is {main.get('resizable')!r}, expected false -- "
                 "a resizable window over a fixed-pixel `.layout` is broken, not responsive")
        if main.get("decorations") is not False:
            fail(f"main window `decorations` is {main.get('decorations')!r}, expected false -- "
                 "ADR-0020 pins an undecorated title bar the app decorates itself")
        for forbidden, why in (
            ("fullscreen", "a fixed 1440x900 canvas cannot fill an arbitrary screen"),
            ("maximized", "maximizing a non-resizable fixed canvas is a contradiction"),
        ):
            if main.get(forbidden) is True:
                fail(f"main window sets `{forbidden}: true` -- {why}")

# --- 2. No scale-transform survived the port --------------------------------
# Scan the frontend sources (not `dist`, which is build output, and not this
# repository's markdown, where the ban is DESCRIBED and must stay describable).
SCAN_ROOTS = [repo / "ui" / "src", repo / "ui"]
SCAN_SUFFIXES = {".ts", ".tsx", ".js", ".jsx", ".css", ".html"}

BANNED_PATTERNS = [
    (re.compile(r"transform\s*:\s*[^;\"']*\bscale\s*\("),
     "CSS `transform: scale()` -- the mock's browser-tab fit hack (grill G1/A5)"),
    # The same hack written from JS: `el.style.transform = "scale(0.8)"`. Without this
    # the CSS-syntax pattern above is trivially side-stepped by setting the property
    # imperatively, which is exactly how the mock's fit() script did it.
    (re.compile(r"""transform\s*=\s*[`\"'][^`\"']*\bscale\s*\("""),
     "a JS `style.transform = \"scale(...)\"` -- the mock's fit hack, set imperatively"),
    (re.compile(r"""\bsetProperty\s*\(\s*[`\"']transform[`\"']\s*,\s*[`\"'][^`\"']*\bscale\s*\("""),
     "a JS `setProperty(\"transform\", \"scale(...)\")` -- the same hack again"),
    (re.compile(r"\binstallFit\b"),
     "`installFit()` -- the mock's fit script must not be ported"),
    (re.compile(r"\bfunction\s+fit\s*\("),
     "a `fit()` function -- the mock's inline fit script must not be ported"),
    (re.compile(r"\bzoom\s*:\s*[0-9.]"),
     "a CSS `zoom` -- the same canvas-scaling hack by another name"),
]

# Comments are BLANKED (not deleted) before scanning, so the ban can be DOCUMENTED in
# the very files it applies to and line numbers still point at the real line. This is
# the repo's existing convention -- see tests/determinism_guard.rs, which strips `//`
# comments for exactly the same reason. Blanking rather than removing also means a
# banned token cannot be smuggled past the scan by hiding it inside a comment that
# happens to end mid-line.
BLOCK_COMMENTS = {
    ".html": [(r"<!--", r"-->")],
    ".css": [(r"/\*", r"\*/")],
    ".ts": [(r"/\*", r"\*/")],
    ".tsx": [(r"/\*", r"\*/")],
    ".js": [(r"/\*", r"\*/")],
    ".jsx": [(r"/\*", r"\*/")],
}
# `//` is a LINE comment in JS/TS but NOT in CSS or HTML (where it would eat URLs).
LINE_COMMENT_SUFFIXES = {".ts", ".tsx", ".js", ".jsx"}


def blank_comments(text, suffix):
    """Replace comment bodies with spaces, preserving newlines and offsets."""
    for opener, closer in BLOCK_COMMENTS.get(suffix, []):
        pattern = re.compile(f"{opener}.*?{closer}", re.S)
        text = pattern.sub(lambda m: re.sub(r"[^\n]", " ", m.group(0)), text)
    if suffix in LINE_COMMENT_SUFFIXES:
        text = "\n".join(line.split("//", 1)[0] for line in text.split("\n"))
    return text


seen = set()
for root in SCAN_ROOTS:
    if not root.is_dir():
        continue
    for path in sorted(root.rglob("*")):
        if not path.is_file() or path.suffix not in SCAN_SUFFIXES:
            continue
        if "dist" in path.relative_to(repo).parts or "node_modules" in path.parts:
            continue
        if path in seen:
            continue
        seen.add(path)
        raw_lines = path.read_text().splitlines()
        code_lines = blank_comments(path.read_text(), path.suffix).splitlines()
        for line_no, code in enumerate(code_lines, start=1):
            for pattern, why in BANNED_PATTERNS:
                if pattern.search(code):
                    rel = path.relative_to(repo)
                    offending = raw_lines[line_no - 1].strip() if line_no <= len(raw_lines) else ""
                    fail(f"{rel}:{line_no}: {why}\n         offending line: {offending}")

if not seen:
    fail("scanned no frontend source files -- the scale-transform half of this check is "
         "a false green (did `ui/` move?)")

# --- report -----------------------------------------------------------------
if failures:
    for f in failures:
        print(f"FAIL: {f}", file=sys.stderr)
    print(f"check-window-config: {len(failures)} failure(s)", file=sys.stderr)
    sys.exit(1)

print(f"check-window-config: OK (main window {EXPECTED_WIDTH}x{EXPECTED_HEIGHT}, "
      f"resizable=false, decorations=false; no scale-transform in {len(seen)} "
      "frontend source file(s))")
PY
