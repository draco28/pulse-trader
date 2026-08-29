#!/usr/bin/env bash
# AC-6 — least-privilege capability gate (r1.s1.w1, ADR-0020 step 7).
#
# The spec's wording is the whole point of this script: the frontend gets the commands
# it needs and nothing else, and the check "asserts their absence rather than trusting
# review". An ungranted permission is only a security property if something FAILS when
# it silently reappears.
#
# It asserts absence at BOTH levels, because either one alone is a false green:
#
#   1. CAPABILITY level — no granted permission belongs to the filesystem, shell or
#      HTTP families. This is what the webview is allowed to call.
#   2. DEPENDENCY level — no `tauri-plugin-fs` / `-shell` / `-http` is in Cargo.toml,
#      and nothing registers one with `.plugin(...)`. A capability file that omits a
#      permission proves nothing if the plugin is present with a permissive default
#      bundle, and a future `core:default`-style blanket grant would pick it up.
#
# It also asserts the capability set is an EXPLICIT ALLOWLIST: every capability names
# the windows it applies to, and no permission is a wildcard.
#
# Requires: python3 (JSON needs a real parser; grep-ing JSON is how a check like this
# quietly stops checking). Present on macOS and on both GitHub runner images.
#
# Exit 0 iff every assertion holds; exit 1 listing EVERY failure in one run.

set -uo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if ! command -v python3 >/dev/null 2>&1; then
  echo "FAIL: python3 is required to parse the capability JSON" >&2
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

# The permission families that must NOT be reachable from the webview. Matched on the
# permission identifier's namespace prefix, which is how Tauri v2 namespaces them
# (`fs:read-all`, `shell:allow-execute`, `http:default`, ...).
FORBIDDEN_NAMESPACES = ("fs", "shell", "http")
FORBIDDEN_PLUGINS = ("tauri-plugin-fs", "tauri-plugin-shell", "tauri-plugin-http")

# --- 1. The capability directory exists and is non-empty ---------------------
cap_dir = repo / "capabilities"
if not cap_dir.is_dir():
    fail(f"no capability directory at {cap_dir} -- the frontend's permission set must be "
         "declared explicitly, not left to Tauri's defaults")
    print("\n".join(f"FAIL: {f}" for f in failures), file=sys.stderr)
    print(f"check-capabilities: {len(failures)} failure(s)", file=sys.stderr)
    sys.exit(1)

cap_files = sorted(p for p in cap_dir.iterdir() if p.suffix == ".json")
if not cap_files:
    fail(f"{cap_dir} contains no .json capability file")

granted = []          # (file, permission-identifier)
for path in cap_files:
    try:
        doc = json.loads(path.read_text())
    except json.JSONDecodeError as e:
        fail(f"{path.name} is not valid JSON: {e}")
        continue

    # --- 2. Explicit allowlist shape -----------------------------------------
    if "identifier" not in doc:
        fail(f"{path.name} has no `identifier`")

    windows = doc.get("windows")
    if not isinstance(windows, list) or not windows:
        fail(f"{path.name} must name the windows it applies to explicitly "
             "(`windows: [...]`), so a future window does not inherit this grant")
    else:
        for w in windows:
            if not isinstance(w, str) or "*" in w:
                fail(f"{path.name} scopes to window {w!r} -- a wildcard window scope is "
                     "not an allowlist")

    permissions = doc.get("permissions")
    if not isinstance(permissions, list):
        fail(f"{path.name} must declare a `permissions` array (an explicit allowlist)")
        continue

    for perm in permissions:
        # A permission is either a plain identifier string or an object with an
        # `identifier` key (the scoped form).
        if isinstance(perm, str):
            ident = perm
        elif isinstance(perm, dict) and isinstance(perm.get("identifier"), str):
            ident = perm["identifier"]
        else:
            fail(f"{path.name}: unrecognised permission entry {perm!r}")
            continue

        if "*" in ident:
            fail(f"{path.name}: permission {ident!r} contains a wildcard -- "
                 "least privilege means enumerating, not globbing")
        granted.append((path.name, ident))

# --- 3. No filesystem / shell / HTTP permission is granted -------------------
for filename, ident in granted:
    namespace = ident.split(":", 1)[0] if ":" in ident else ident
    if namespace in FORBIDDEN_NAMESPACES:
        fail(f"{filename} grants {ident!r} -- the {namespace!r} family must not be "
             "reachable from the frontend (ADR-0020 least-privilege)")

if not granted and cap_files:
    fail("no permission is granted by any capability -- an empty set is suspicious "
         "enough to be a mistake; grant the core permissions the window needs")

# --- 4. The forbidden plugins are not even dependencies ----------------------
cargo_toml = (repo / "Cargo.toml").read_text()
for plugin in FORBIDDEN_PLUGINS:
    # Match a dependency declaration line, not a mention in a comment.
    pattern = rf'(?m)^\s*{re.escape(plugin)}\s*='
    if re.search(pattern, cargo_toml):
        fail(f"Cargo.toml declares {plugin} -- absence at the capability level is a "
             "false green while the plugin's commands exist")

# --- 5. Nothing registers one of those plugins in code -----------------------
tauri_src = repo / "src" / "tauri"
if tauri_src.is_dir():
    for rs in sorted(tauri_src.rglob("*.rs")):
        source = rs.read_text()
        for line in source.splitlines():
            code = line.split("//", 1)[0]
            for plugin in FORBIDDEN_PLUGINS:
                crate_ident = plugin.replace("-", "_")
                if f"{crate_ident}::init" in code or f".plugin({crate_ident}" in code:
                    fail(f"{rs.relative_to(repo)} registers {plugin}: {line.strip()}")

# --- report -------------------------------------------------------------------
if failures:
    for f in failures:
        print(f"FAIL: {f}", file=sys.stderr)
    print(f"check-capabilities: {len(failures)} failure(s)", file=sys.stderr)
    sys.exit(1)

names = ", ".join(sorted({ident for _, ident in granted}))
print(f"check-capabilities: OK ({len(granted)} permission(s) granted, none in "
      f"fs/shell/http; no such plugin is a dependency)")
print(f"  granted: {names}")
PY
