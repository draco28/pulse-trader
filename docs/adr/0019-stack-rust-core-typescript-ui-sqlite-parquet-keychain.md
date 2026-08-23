# 19. Stack: Rust core + agent + Tauri backend, TypeScript/React UI, SQLite WAL + Parquet + Keychain

Date: 2026-08-23T00:00:00Z

## Status

**Accepted** for the backend and storage stack. **Proposed** for the desktop stack.

(Recorded at adoption on 2026-08-23 against baseline `49f229a`. Ossify's bones
protocol mints a decision `Accepted` when the adopted baseline already exercises it —
and that is true of only half of this one. The Rust core, SQLite/WAL, Parquet,
Keychain and filesystem tiers are exercised by three sprints of shipped work. The
**desktop stack is not**: the tree at this baseline contains no TypeScript files, no
package manifest, and no Tauri, React, Vite or tauri-specta dependency, and
`src/tauri/mod.rs` is an explicit empty stub. Marking those choices retrospectively
validated would mislead exactly the implementation and release planning that reads
this registry, so they stay `Proposed` until v1.5 exercises them.)

## Context

Two languages, chosen so the deterministic engine and the agent orchestration share one
runtime. Python was eliminated when PulseHive made in-Rust agent orchestration viable.

## Decision

**Exercised at this baseline (Accepted).** **Rust** for the core engine and agent
orchestration. Storage in **three** tiers: **SQLite** (WAL, single file) for entities
and event logs with sqlx migrations; **Parquet** for immutable `CandleSeries`; the
**macOS Keychain** for secrets. No Postgres, no Redis, no separate vector database.

**Not yet exercised (Proposed) — the filesystem tier.** Logs and exports were
specified as a fourth tier, and no production writer exists: the crate has no
`tracing` or `log` dependency at all, `eprintln!` is the CLI warning channel (as
`src/adapters/db/backtest_run_repo.rs` records, noting the spec names `tracing::warn`
aspirationally), LLM-call records go to SQLite, and there is no export path. Counting
it as validated would misdirect storage and release planning. It arrives with the
structured-logging work ADR-0017 defers.

**Not yet exercised (Proposed).** **TypeScript (React + Vite)** for the UI in Tauri's
WebView, a Tauri backend in the same Rust crate, glued by the Tauri command bus with
types generated via **tauri-specta**. None of this exists at `49f229a` — it is the
v1.5 shell, and the hexagonal layout (`src/tauri/mod.rs`, pinned as an empty stub at
WI-01) is the only thing reserving space for it. Treat it as a direction, not a
settled contract: the first slice that actually builds the shell may revisit it, and
doing so is not a bone violation.

## Consequences

One toolchain for all deterministic work, and no cross-language serialization boundary
in the hot path. Embedded storage keeps the app installable with no service to run,
at the cost of ruling out multi-machine deployment without revisiting this bone. A
second backend language or any server-side component is the revisit trigger.
