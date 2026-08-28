# 22. Toolchain bump: Rust 1.92.0 → 1.98.0

Date: 2026-08-25T00:00:00Z

## Status

Accepted

(Accepted 2026-08-28 at spine `r1.s5`'s close: the bump landed on `r1.s5.w1` and has been
exercised — the whole test suite, `clippy --all-targets -D warnings` and `cargo deny` run
green on 1.98.0, and the specta trio it unblocked (`w2`) deleted the
`post_process_bindings` workaround rather than re-justifying it. Authored `Proposed` on
purpose by `r1.s5.w1`: ossify's bones protocol mints a decision `Accepted` only once a
release has exercised it. This ADR answers `rust-toolchain.toml`'s own header comment —
*"Bump deliberately (records in an ADR) when the engine moves"* — and is the ADR
`Cargo.toml`'s specta-pin comment names: *"the toolchain bump needs its own ADR, because it
moves the engine fingerprint (NFR-2 byte-reproducibility)."*)

## Context

`rust-toolchain.toml` has pinned an **exact** channel since ADR-0019 (audit C4): not
a floating `stable`, because the resolved toolchain hash is one of the four inputs to
`build.rs`'s `engine_fingerprint` (the others are `Cargo.lock`'s bytes, the DSL
schema-version string, and the target triple), and byte-reproducibility across
architectures (NFR-2) requires that hash to be pinned, not ambient. The pin sat at
`1.92.0`, six stable releases behind, and the gap had already forced one workaround:
`r1.s1.w1` needed `tauri-specta`-generated bindings, but the newest compatible pair
(specta `2.0.0-rc.22` / `tauri-specta 2.0.0-rc.21`) is not the newest available —
specta `2.0.0-rc.24` and `rc.25` both use `core::fmt::from_fn`, gated behind the
`debug_closure_helpers` feature (rust-lang#117729), which is unstable on `1.92.0` and
fails `E0658` on a stable compiler. `r1.s1.w1` shipped against the older, compatible
pair and repaired the generator's known-defective output in code
(`post_process_bindings`) rather than block the shell on a toolchain move it was not
scoped to make. `r1.s5.w2` retires that workaround by bumping specta itself — but
only once the toolchain underneath it can compile the newer pair, which is this
item's entire job.

**Checked, not inferred, in both directions** — a minimal `core::fmt::from_fn` probe
(a standalone `.rs` file, not this crate) was compiled under both toolchains outside
the worktree:

- `rustc +1.92.0`: `error[E0658]: use of unstable library feature
  'debug_closure_helpers'` (rust-lang#117729) — reproduces the failure specta rc.24+
  hits.
- `rustc +1.98.0`: compiles and runs. The feature stabilized somewhere in the six
  releases between `1.92.0` and `1.98.0`, so a normal stable build now reaches it.

`1.98.0` was independently confirmed to be the **latest stable release** as of this
ADR's date (`rustup toolchain install stable` resolves to `rustc 1.98.0 (88d9e12ae
2026-08-18)` on `2026-08-25`) — not assumed from the pinned version number alone.

## Decision

**Pin `1.98.0`.** `rust-toolchain.toml`'s `channel` moves from `"1.92.0"` to
`"1.98.0"`; the `components = ["rustfmt", "clippy"]` line and the exact-pin
rationale in its header comment are unchanged — only the version moves, not the
policy (see *Alternatives considered* below for why a floating channel is still
rejected).

**Why now.** This unblocks `r1.s5.w2`'s specta rc.24+ bump, verified in both
directions above, not merely inferred from the release-notes gap. `r1.s5.w2` itself —
bumping specta/`tauri-specta`, deleting `post_process_bindings`, regenerating
`ui/src/bindings.ts` — is out of this item's scope; this ADR decides only the
toolchain move that makes it possible.

## Consequences

**The fingerprint moves, and that is provenance, not an invalidation.**
`build.rs` folds the *resolved* `rustc -vV` (`release:` + `commit-hash:` lines) into
`engine_fingerprint` alongside `Cargo.lock`'s bytes, so every `BacktestRun`'s stored
fingerprint changes value the moment this pin changes. Read literally that sounds
like a breaking change to every persisted run; it is not, for a reason worth stating
precisely because an earlier draft of this ADR got it backwards:
[`EngineFingerprint::compare`](../../src/domain/fingerprint.rs) (line 120) returns
[`Option<String>`](../../src/domain/fingerprint.rs) — `None` when two fingerprints
match, `Some(<warning text>)` when they differ — **never an `Err`**. Its one
production caller, `persist_and_compare` in
[`src/cli/backtest.rs`](../../src/cli/backtest.rs) (around line 314), only
`eprintln!`s that warning to stderr; it never fails the run, never blocks
`save_run`, and no test or fixture asserts a literal fingerprint hex anywhere in the
crate. Nothing in the codebase treats "fingerprint changed" as more than a WARNING
that two runs came from different engine builds — and the fingerprint already moves
on **every** dependency change, since `Cargo.lock`'s bytes are input #1, so a
version-controlled toolchain bump is not a new category of change, only a larger
instance of one that already happens routinely.

**The cost that IS real: six releases of new `clippy::pedantic` lints under
`deny(warnings)`.** Two categories fired, both fixed rather than allowed away:

- **`clippy::map_unwrap_or`** (1 site) — `src/adapters/binance/client.rs`'s jitter
  seed used `.duration_since(UNIX_EPOCH).map(|d| …).unwrap_or(…)`; rewritten to the
  single `.map_or(default, |d| …)` clippy itself suggests. Behaviour-identical, one
  fewer intermediate `Option`.
- **`clippy::unused_async_trait_impl`** (34 sites) — new since the `1.96.0` probe
  this spec's own table was built from, and clippy hard-stops linting at the first
  compile error, so the probe never reached most of these: every test-only mock
  trait impl (`FakeSource`, `FakeRepo`, `FakeRunRepo`, `FakeProvider`,
  `FakeLlmCallRepo`, and their siblings across `src/domain/port.rs`,
  `src/adapters/binance/source.rs`, `src/adapters/llm/redacting_logging.rs`, and five
  `tests/*.rs` integration files) implemented an `async fn` with no `.await`
  anywhere in its body — dead weight from the state-machine transform for a value
  that was always ready synchronously. Each was rewritten to a plain `fn` returning
  `impl Future<Output = …>` via `std::future::ready(..)`, matching the pattern the
  port traits already use for non-async adapters (`PageSource::get` in
  `src/adapters/binance/source.rs`). Four sites used the `?` operator inside the
  original body in a way clippy's own auto-suggested diff does not actually
  type-check against a plain-`fn`-returning-`impl Future` signature; those four use
  an immediately-invoked closure to keep the early-return semantics correct rather
  than following clippy's literal (and here, wrong) suggested edit.

**Also checked and found NOT to reproduce:** the spec's own probe table (built on
`1.96.0`, before this item's clippy run ever reached the composer module) named
`clippy::duration_suboptimal_units` at `src/agent/composer.rs:154`
(`Duration::from_secs(120)`) as a known site. On `1.98.0` that lint did not fire —
`src/agent/composer.rs` was left untouched. This is recorded as an observed result,
not a silently dropped line item: the six-release gap the spec's table exercised
does not fully overlap the six-release gap this item bumped through.

**No crate-root blanket `#![allow]`.** `src/lib.rs` carries none, before or after —
clearing the tail meant fixing sites, not suppressing the lints that found them.

## Alternatives considered

**Stay on `1.92.0`, carry `post_process_bindings` indefinitely.** Rejected. The
workaround this ADR retires is not free: it repairs specta's known-defective output
in code every time bindings regenerate, and the pin blocks unrelated future
dependency work that happens to want a newer `rustc` for reasons that have nothing
to do with specta. Compounding a workaround is a worse trade than fixing the root
cause once.

**Pin `1.96.0`, already installed locally at the start of this item.** Rejected.
`1.96.0` was already two releases behind `1.98.0` (the actual latest stable) at
authoring time, and pinning it would only repeat this exact problem on a shorter
fuse — a second toolchain-bump ADR due almost immediately, for no benefit over
moving straight to latest stable now that the move is being made at all.

**Float `stable` instead of an exact pin.** Rejected outright, and not re-litigated
by this item. ADR-0019's exact-pin requirement (audit C4) exists so
`engine_fingerprint` is reproducible build-to-build; a floating channel would make
the fingerprint drift on every `rustup update` a developer happens to run locally,
which defeats NFR-2's byte-reproducibility premise. This ADR moves the pinned
*version*, never the *policy* of pinning one.
