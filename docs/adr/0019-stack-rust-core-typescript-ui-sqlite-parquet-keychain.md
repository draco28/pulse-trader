# 19. Stack: Rust core with embedded SQLite (WAL) and Parquet storage

Date: 2026-08-23T00:00:00Z

## Status

Accepted

(Recorded at adoption on 2026-08-23 against baseline `49f229a`. **This ADR is
deliberately narrow**: it decides only the stack the adopted baseline actually
exercises, so it can carry one authoritative status per ADR-0001's vocabulary. The
Keychain provisioning path, the filesystem logging/export tier and the
Tauri/TypeScript desktop shell were all named in the original stack plan and are
**not decided here** — see *Deliberately out of scope* below. Ossify's bones protocol
mints a decision `Accepted` when the baseline exercises it; that is true of
everything this ADR decides and of nothing it excludes.)

## Context

Two languages, chosen so the deterministic engine and the agent orchestration share one
runtime. Python was eliminated when PulseHive made in-Rust agent orchestration viable.

## Decision

**Exercised at this baseline (Accepted).** **Rust** for the core engine and agent
orchestration. Storage in **two** operational tiers: **SQLite** in WAL mode for
entities and event logs with sqlx migrations, and **Parquet** for immutable
`CandleSeries`.

**"Embedded", not "single-file".** `Db::with_path` unconditionally selects
`SqliteJournalMode::Wal`, so the database is `pulse.db` **plus** its `-wal` and `-shm`
companions (the repo's `.gitignore` handles both). While the database is open or holds
uncheckpointed frames, copying `pulse.db` alone can omit committed transactions — so
any backup, export or packaging path must take all three, or checkpoint first. No Postgres, no Redis, no separate vector database.

## Deliberately out of scope

Three components of the original stack plan are **not decided by this ADR**, because
the baseline does not exercise any of them. Each needs its own ADR when something
does. They are listed so the omission reads as deliberate rather than forgotten.

- **Keychain provisioning.** The Keychain is the intended secret store (ADR-0016) and
  the read path is implemented and tested, but no credential can be *provisioned*
  through the shipped application: `keyring` binds the code-identity-scoped
  data-protection keychain, so only the `pulse` binary can seed a key it can later
  read back, and `pulse setup-keys` does not exist. `src/adapters/secrets.rs` records
  that the live transport was validated by **injecting the key directly**, not through
  this tier. Secret provisioning is already a Release 1 candidate.
- **A filesystem logging / export tier.** No production writer exists: the crate has
  no `tracing` or `log` dependency, `eprintln!` is the CLI warning channel (as
  `src/adapters/db/backtest_run_repo.rs` records, noting the spec names
  `tracing::warn` aspirationally), LLM-call records go to SQLite, and there is no
  export path. It arrives with the structured-logging work ADR-0017 defers.
- **The Tauri/TypeScript desktop shell** — React + Vite in Tauri's WebView, a Tauri
  backend, tauri-specta-generated bindings. None of it exists at `49f229a`;
  `src/tauri/mod.rs` is an empty stub pinned at WI-01 to reserve the layout. It is a
  direction, not a settled contract, and the first slice that builds it may revisit
  the choice without violating any bone.

`ARCHITECTURE.md` and `EXECUTIVE-SUMMARY.md` still describe some of these as present;
that drift is tracked in **#111**.

## Consequences

One toolchain for all deterministic work, and no cross-language serialization boundary
in the hot path. Embedded storage keeps the app installable with no service to run,
at the cost of ruling out multi-machine deployment without revisiting this bone. A
second backend language or any server-side component is the revisit trigger.
