#!/usr/bin/env bash
# AC-2 -- r1.s1.w6's own content gate, modelled on check-design-system.sh.
#
# A BACKSTOP, not the primary evidence (spec step 8, audit finding C6): after
# C6, round-3 requirements 1 and 2 (exactly one nav row active; no row a dead
# link) are proved by the RENDERED vitest tests in `App.test.tsx` -- this gate
# exists so those assertions cannot be quietly deleted, and so the harness
# cannot silently fall out of `just check`. It asserts CONTENT, never mere
# existence (planning-audit finding #6): every check below reads a specific
# claim out of the real source files, not just whether a path exists.
#
# What it asserts, at minimum (spec step 8):
#   1. every `ROUTES` entry that declares a `nav` has `path === "/" + nav`;
#   2. every `nav` value corresponds to a real nav-row id in `AppShell.tsx`'s
#      `NAV_MAIN`/`NAV_BOTTOM`;
#   3. no `href="#"` survives anywhere under `ui/src`;
#   4. `Sidebar` is rendered WITH an `active` prop (passed, not merely
#      mentioned);
#   5. the justfile's `ui` recipe runs the UI test target, so the harness
#      cannot silently fall out of `just check`.
#
# Requires: python3. Exit 0 iff every assertion holds; exit 1 listing every
# failure (no halt on the first one).

set -uo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if ! command -v python3 >/dev/null 2>&1; then
  echo "FAIL: python3 is required to run check-shell-navigation.sh" >&2
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


def blank_line_comments(text):
    """Strip `//` line comments (repo convention, see check-window-config.sh),
    so a banned/required token mentioned only in a comment cannot fool the
    scan in either direction."""
    return "\n".join(line.split("//", 1)[0] for line in text.split("\n"))


# --- read the route table and the nav-row tables -----------------------------
routes_path = ui_src / "routes.ts"
if not routes_path.is_file():
    fail("ui/src/routes.ts does not exist")
    routes_text = ""
else:
    routes_text = routes_path.read_text()

app_shell_path = ui_src / "shell" / "AppShell.tsx"
if not app_shell_path.is_file():
    fail("ui/src/shell/AppShell.tsx does not exist")
    app_shell_text = ""
else:
    app_shell_text = app_shell_path.read_text()


def parse_route_entries(text):
    """Each `{ ... }` object literal inside the `ROUTES` array, as a dict of
    its `field: "value"` string-literal assignments. Good enough for this
    table's one-append-only-entry-per-screen shape (tauri_bus_contract.rs
    asserts that shape already); this does not need a full TS parser."""
    match = re.search(r"export const ROUTES[^=]*=\s*\[(.*?)\n\];", text, re.S)
    if match is None:
        return []
    body = match.group(1)
    entries = []
    for block in re.findall(r"\{([^{}]*)\}", body, re.S):
        fields = {}
        for line in block.splitlines():
            m = re.match(r'\s*(\w+):\s*"([^"]*)"', line)
            if m:
                fields[m.group(1)] = m.group(2)
        if fields:
            entries.append(fields)
    return entries


def parse_nav_ids(text, table_name):
    match = re.search(rf"{table_name}[^=]*=\s*\[(.*?)\n\];", text, re.S)
    if match is None:
        return []
    body = match.group(1)
    ids = []
    for block in re.findall(r"\{([^{}]*)\}", body, re.S):
        m = re.search(r'\bid:\s*"([^"]+)"', block)
        if m:
            ids.append(m.group(1))
    return ids


route_entries = parse_route_entries(routes_text)
nav_main_ids = parse_nav_ids(app_shell_text, "NAV_MAIN")
nav_bottom_ids = parse_nav_ids(app_shell_text, "NAV_BOTTOM")
all_nav_ids = set(nav_main_ids) | set(nav_bottom_ids)

if routes_text and not route_entries:
    fail("ui/src/routes.ts: found no ROUTES entries to check -- did the table's shape change?")
if app_shell_text and not all_nav_ids:
    fail("ui/src/shell/AppShell.tsx: found no NAV_MAIN/NAV_BOTTOM nav-row ids to check")

# --- 1 & 2: path === "/" + nav, and nav matches a real nav-row id -----------
for entry in route_entries:
    nav = entry.get("nav")
    if nav is None:
        continue
    path = entry.get("path")
    if path != "/" + nav:
        fail(f"ROUTES entry with nav={nav!r} has path={path!r}, expected "
             f"'/{nav}' -- the path === \"/\" + nav convention (G7) is broken")
    if all_nav_ids and nav not in all_nav_ids:
        fail(f"ROUTES entry declares nav={nav!r}, which is not a real nav-row "
             "id in AppShell.tsx's NAV_MAIN/NAV_BOTTOM")

# --- 3: no href="#" survives anywhere under ui/src ---------------------------
if ui_src.is_dir():
    for path in sorted(ui_src.rglob("*")):
        if not path.is_file() or path.suffix not in {".ts", ".tsx", ".js", ".jsx"}:
            continue
        code = blank_line_comments(path.read_text())
        for line_no, line in enumerate(code.splitlines(), start=1):
            if re.search(r'href\s*=\s*"#"', line):
                fail(f"{path.relative_to(repo)}:{line_no}: href=\"#\" survives -- "
                     "every nav row must navigate (step 5)")

# --- 4: Sidebar is rendered WITH an active prop (passed, not just mentioned) -
if app_shell_text:
    app_tsx_path = ui_src / "App.tsx"
    if not app_tsx_path.is_file():
        fail("ui/src/App.tsx does not exist")
    else:
        app_tsx_code = blank_line_comments(app_tsx_path.read_text())
        # `<Sidebar ... active= ... >` -- the prop appears before the tag closes.
        if not re.search(r"<Sidebar\b(?:(?!/?>).)*?\bactive\s*=", app_tsx_code, re.S):
            fail("ui/src/App.tsx: <Sidebar /> is not rendered WITH an `active` "
                 "prop -- the prop must be PASSED, not merely mentioned "
                 "elsewhere in the file")

# --- 5: the justfile's `ui` recipe runs the UI test target -------------------
justfile_path = repo / "justfile"
if not justfile_path.is_file():
    fail("justfile does not exist")
else:
    justfile_text = justfile_path.read_text()
    ui_recipe_match = re.search(r"^ui:.*?\n((?:    .*\n?)*)", justfile_text, re.M)
    if ui_recipe_match is None:
        fail("justfile: no `ui:` recipe found")
    else:
        recipe_body = ui_recipe_match.group(1)
        if "vitest" not in recipe_body and "npm run test" not in recipe_body:
            fail("justfile's `ui` recipe does not run the UI test target (G9) -- "
                 "the harness could fall silently out of `just check`")

# --- report -------------------------------------------------------------------
if failures:
    for f in failures:
        print(f"FAIL: {f}", file=sys.stderr)
    print(f"check-shell-navigation: {len(failures)} failure(s)", file=sys.stderr)
    sys.exit(1)

print(f"check-shell-navigation: OK ({len(route_entries)} route entr(y/ies), "
      f"{len(all_nav_ids)} nav row id(s), no href=\"#\", Sidebar gets `active`, "
      "ui recipe runs the UI test target)")
PY
