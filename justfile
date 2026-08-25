# PulseTrader command runner. `just check` is the aggregate local gate
# mirrored by CI (.github/workflows/ci.yml).

# Aggregate gate: frontend, then Rust, then the desktop shell's check scripts.
#
# r1.s1.w1 (grill A4) grew this from `fmt clippy test` so that ONE command still
# gates EVERYTHING now that the repo has a second language and four shell gates.
# The alternative -- a Rust gate plus a separate frontend gate someone has to
# remember -- is how a TypeScript regression reaches `main` while `just check` is
# green.
#
# Order is deliberate: `ui` runs FIRST because `cargo` needs `ui/dist` to exist
# (`generate_context!` embeds it at compile time), so building the frontend before
# the Rust targets means clippy and the tests compile against the REAL bundle
# rather than build.rs's placeholder.

# The aggregate gate: frontend + Rust + the desktop shell's four check scripts.
check: ui fmt clippy test gates

# --- frontend ---------------------------------------------------------------

# Typecheck and build the frontend bundle Tauri embeds.
ui: ui-deps
    npm run typecheck
    npm run build

# Install node modules only when they are missing. `npm ci` (not `npm install`)
# so the committed package-lock.json is authoritative and a gate run can never
# silently change the dependency tree.

# Install node modules when missing (npm ci, lockfile-authoritative).
ui-deps:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ ! -d node_modules ]; then
        npm ci
    fi

# --- rust -------------------------------------------------------------------

# Verify formatting without rewriting.
fmt:
    cargo fmt --check

# Lint all targets, warnings as errors.
clippy:
    cargo clippy --all-targets -- -D warnings

# Run the test suite via nextest.
test:
    cargo nextest run

# --- desktop shell gates (r1.s1.w1, r1.s1.w5) -------------------------------

# The five content gates for the shell. Each asserts a property that review
# cannot be trusted to hold: ADR-0020's decision is recorded and the ADR-0001 /
# ADR-0019 class sweep landed; no fs/shell/http capability is reachable from the
# frontend; the window stays 1440x900 non-resizable with no scale-transform; the
# committed bindings match a fresh generation; and the ported design system
# (tokens.css/shared.css, no per-screen sheet, no macos-window.jsx) landed for
# real (r1.s1.w5, AC-2).

# Run the five desktop-shell content gates (ADR, capabilities, window, bindings, design system).
gates:
    bash scripts/check-adr-0020.sh
    bash scripts/check-capabilities.sh
    bash scripts/check-window-config.sh
    bash scripts/check-bindings.sh
    bash scripts/check-design-system.sh

# VS-1.1.4 work-1.01 — regenerate the committed .sqlx offline query cache
# (NFR-12). Needs sqlx-cli (a developer-local tool, NOT installed in this slice's
# pre-flight). Creates a temp sqlite file, runs the migrations against it, runs
# `cargo sqlx prepare`, then removes the temp file. Its real payoff is 1.03 onward,
# once `query!` macros exist; it does NOT run in CI's build.
prepare:
    rm -f pulse-prepare.db pulse-prepare.db-wal pulse-prepare.db-shm
    DATABASE_URL=sqlite://pulse-prepare.db sqlx database create
    DATABASE_URL=sqlite://pulse-prepare.db sqlx migrate run
    DATABASE_URL=sqlite://pulse-prepare.db cargo sqlx prepare
    rm -f pulse-prepare.db pulse-prepare.db-wal pulse-prepare.db-shm

# --- desktop bundle (r1.s1.w1) ----------------------------------------------

# Build PulseTrader.app for a LOCAL dev run — this is what r1.s1.w5's AC-11 manual
# walk needs.
#
# Deliberately NOT part of `just check`: it is a release build of the whole Tauri
# graph (minutes, not seconds), and gating every local check run on it would make the
# gate something people skip. Code signing, notarization and auto-update are out of
# scope for r1.s1.w1 (ADR-0020) — this produces an unsigned local artifact at
# `target/release/bundle/macos/PulseTrader.app`.

# Build an unsigned local PulseTrader.app (what AC-11's manual walk needs).
bundle:
    npm run bundle

# Run the desktop shell against the Vite dev server (hot reload).
desktop:
    npm run desktop
