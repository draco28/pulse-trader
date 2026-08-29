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

One language for everything this baseline ships, so the deterministic engine and the
agent loop share a runtime. Python was eliminated when in-Rust agent orchestration
became viable. A second language for the UI is anticipated but **not decided here** —
see *Deliberately out of scope*.

## Decision

**Exercised at this baseline (Accepted).** **Rust** for the core engine and agent
orchestration. Storage in **two** operational tiers: **SQLite** in WAL mode for
entities and event logs with sqlx migrations, and **Parquet** for immutable
`CandleSeries`.

**"Embedded", not "single-file".** `Db::with_path` unconditionally selects
`SqliteJournalMode::Wal`, so on disk the database is `pulse.db` **plus** its `-wal` and
`-shm` companions (the repo's `.gitignore` handles both), and copying `pulse.db` alone
can omit committed transactions.

**Copying the three files is not the remedy.** Read as ordinary separate files they are
not guaranteed to represent the same instant, and a manual checkpoint races any
subsequent write unless the database is quiesced. `src/adapters/db/migrate.rs` already
names the right tool: **`VACUUM INTO`**, SQLite's first-class consistent-backup
primitive — *"WAL-safe by construction (no manual `wal_checkpoint`, no `-wal`/`-shm`
sidecar copy, no torn snapshot)"*. Any backup, export or packaging path must use
`VACUUM INTO`, an atomic filesystem snapshot, or an exclusive shutdown — never a
file-by-file copy of a live database. No Postgres, no Redis, no separate vector database.

## Deliberately out of scope

Three components of the original stack plan are **not decided by this ADR**, because
the baseline does not exercise any of them. Each needs its own ADR when something
does. They are listed so the omission reads as deliberate rather than forgotten.

- **Keychain provisioning.** The Keychain is the intended secret store (ADR-0016) and
  a read path exists, but no credential can be *provisioned*
  through the shipped application: `keyring` binds the code-identity-scoped
  data-protection keychain, so only the `pulse` binary can seed a key it can later
  read back, and `pulse setup-keys` does not exist. `src/adapters/secrets.rs` records
  that the live transport was validated by **injecting the key directly**, not through
  this tier. Test coverage is narrower than "implemented and tested" suggests: the only
  test queries a deliberately absent account and asserts the error mapping, so
  **successful retrieval is unverified through the shipped application**. Secret
  provisioning is already a Release 1 candidate.
- **A filesystem logging / export tier.** No production writer exists: the crate has
  no `tracing` or `log` dependency, `eprintln!` is the CLI warning channel (as
  `src/adapters/db/backtest_run_repo.rs` records, noting the spec names
  `tracing::warn` aspirationally), LLM-call records go to SQLite, and there is no
  export path. It arrives with the structured-logging work ADR-0017 defers.
- **The Tauri/TypeScript desktop shell** — React + Vite in Tauri's WebView, a Tauri
  backend, tauri-specta-generated bindings. **Decided, as of 2026-08-25, by
  [ADR-0020](0020-desktop-shell-tauri-react.md)** — the dedicated ADR this bullet asked
  for. It is no longer undecided: `r1.s1.w1` built the shell, replacing the
  `src/tauri/mod.rs` stub with a real Tauri v2 backend, and ADR-0020 records the stack,
  the command-bus contract, the least-privilege capability set, and the single-binary
  argv-dispatch topology that keeps ADR-0015's one-artifact rule literally true. This
  ADR still does not own that decision — it scopes itself to the stack the `49f229a`
  baseline exercised, which the shell was not part of. The pointer is here so the
  omission reads as *closed elsewhere* rather than as still open.

`ARCHITECTURE.md` and `EXECUTIVE-SUMMARY.md` still describe some of these as present;
that drift is tracked in **#111**.

## Consequences

One toolchain for all deterministic work, and no cross-language serialization boundary
in the hot path. Embedded storage keeps the app installable with no service to run,
at the cost of ruling out multi-machine deployment without revisiting this bone. A
second backend language or any server-side component is the revisit trigger.

**The exact toolchain *version* `rust-toolchain.toml` pins is not decided here.**
This ADR decides Rust as the language; the specific channel string is a separate,
narrower decision `rust-toolchain.toml`'s own header comment requires an ADR to
move. [ADR-0022](0022-toolchain-bump-1-98.md) is where that move — and any future
one — is recorded.
