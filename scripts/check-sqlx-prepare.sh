#!/usr/bin/env bash
# AC-12 — the committed `.sqlx` offline query cache is not stale (r1.s3.w2).
#
# WHY THIS EXISTS. `.cargo/config.toml` pins `SQLX_OFFLINE = "true"`, so every
# `cargo build` / `cargo nextest` resolves `query!` macros against the COMMITTED
# `.sqlx/` cache instead of a live database. That is the right default — it keeps
# the build hermetic — but it also means a `query!` whose SQL changed can compile
# green locally against a cache that no longer describes it. The only thing that
# has ever caught that is CI's `prepare-check` job, and `just check` does not run
# it (`check: ui fmt clippy test gates`; none of the nine content gates touches
# sqlx). So a stale cache reached `main` green and red CI afterwards. This script
# closes that gap locally.
#
# WHY NOT `just prepare`. That recipe REGENERATES the cache. Run as a gate it
# would erase the very drift it was meant to report and always pass. The whole
# value here is `--check`: compare, never write.
#
# WHAT IT GUARANTEES.
#   1. No pre-existing database. The schema is built from the shipped
#      `migrations/` set every run, inside a `mktemp -d` outside the repo — which
#      also means a broken or out-of-order migration fails here, not later.
#   2. No mutation. `cargo sqlx prepare --check` compares a fresh generation
#      against `.sqlx/` and exits non-zero on any difference. Nothing under the
#      repo is written; the temp dir goes away on any exit path via `trap`.
#   3. It matches how the cache is GENERATED. `just prepare` runs a bare
#      `cargo sqlx prepare`, so this runs a bare `cargo sqlx prepare --check`. A
#      stricter flag set here (`--all-targets`, say) would red a cache that CI
#      accepts, which is a gate that trains people to ignore it.
#
# DELIBERATELY NOT IN `just check` / `just gates`. `sqlx-cli` is a developer-local
# tool that CI's ordinary check job does not install (`.github/workflows/ci.yml`
# says so explicitly, and installs it only in the separate `prepare-check` job).
# Wiring this into `gates` would red CI on a missing binary. It is a standalone
# work-item gate; CI keeps its own equivalent.
set -euo pipefail

cd "$(dirname "$0")/.."

if ! command -v sqlx >/dev/null 2>&1 || ! command -v cargo-sqlx >/dev/null 2>&1; then
  echo "check-sqlx-prepare: sqlx-cli is not installed." >&2
  echo "  install it pinned to the version Cargo.toml exact-pins (sqlx =0.8.6):" >&2
  echo "  cargo install sqlx-cli --version 0.8.6 --locked --no-default-features --features sqlite,rustls" >&2
  exit 1
fi

workdir="$(mktemp -d)"
trap 'rm -rf "$workdir"' EXIT

export DATABASE_URL="sqlite://$workdir/prepare.db"
sqlx database create
sqlx migrate run --source migrations >/dev/null

# `--check` regenerates into a temp location and diffs against the committed
# `.sqlx/`. It never writes the committed cache; a difference is a non-zero exit.
cargo sqlx prepare --check

echo "check-sqlx-prepare: OK (committed .sqlx matches a fresh generation over the shipped migrations)"
