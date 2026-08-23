# 19. Stack: Rust core on SQLite + Parquet today; Keychain, filesystem and the Tauri/TypeScript shell proposed

Date: 2026-08-23T00:00:00Z

## Status

**Accepted** for the Rust core, SQLite and Parquet. **Proposed** for the macOS
Keychain, the filesystem tier and the desktop stack.

(Recorded at adoption on 2026-08-23 against baseline `49f229a`. Ossify's bones
protocol mints a decision `Accepted` when the adopted baseline already exercises it,
and this decision splits: the Rust core, SQLite/WAL and Parquet are exercised by
three sprints of shipped work, while three other components named in the title are
not. Each is separated in the Decision below with the evidence for why. Marking any
of them retrospectively validated would mislead exactly the implementation and
release planning that reads this registry, so each stays `Proposed` until something
exercises it.)

## Context

Two languages, chosen so the deterministic engine and the agent orchestration share one
runtime. Python was eliminated when PulseHive made in-Rust agent orchestration viable.

## Decision

**Exercised at this baseline (Accepted).** **Rust** for the core engine and agent
orchestration. Storage in **two** operational tiers: **SQLite** (WAL, single file) for
entities and event logs with sqlx migrations, and **Parquet** for immutable
`CandleSeries`. No Postgres, no Redis, no separate vector database.

**Decided but not yet operational — the macOS Keychain.** The Keychain remains the
intended secret store (ADR-0016) and the read path is implemented and tested, but no
credential can be *provisioned* through the shipped application: `keyring` binds the
data-protection keychain, which is scoped by code identity, so only the `pulse` binary
itself can seed a key it can later read back — and the verb that would do it,
`pulse setup-keys`, does not exist. `src/adapters/secrets.rs` records that the live
transport was validated by **injecting the key directly**, not through this tier.
Counting it among the exercised tiers would repeat exactly the retrospective-validation
error corrected above for the desktop and filesystem tiers. It becomes operational
when secret provisioning ships — already a Release 1 feature-map candidate.

**Not yet exercised (Proposed) — the filesystem tier.** Logs and exports were
specified as a storage tier, and no production writer exists: the crate has no
`tracing` or `log` dependency at all, `eprintln!` is the CLI warning channel (as
`src/adapters/db/backtest_run_repo.rs` records, noting the spec names `tracing::warn`
aspirationally), LLM-call records go to SQLite, and there is no export path. Counting
it as validated would misdirect storage and release planning. It arrives with the
structured-logging work ADR-0017 defers.

**Not yet exercised (Proposed) — the desktop stack.** **TypeScript (React + Vite)** for the UI in Tauri's
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
