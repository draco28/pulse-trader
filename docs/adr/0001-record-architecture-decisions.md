# 1. Record architecture decisions

Date: 2026-05-28T15:12:07Z

## Status

Accepted

## Context

PulseTrader entered implementation carrying an unusually high density of load-bearing architectural choices. The MASTER-SPEC was authored through scaffold-onboard's 10-phase conversation and hardened by three interactive grill rounds (domain model, UX, implementation) plus four architect-critic audits (Phase 3, 5, 7 premise-audits and a Phase 10 close-depth audit with a Codex fresh-frame adversary) — 34 challenges surfaced, all resolved. Several of those resolutions are binding and irreversible-in-practice: the hexagonal port boundaries that the "minimize-refactor" promise (close-audit CX5) rests on, the FMA-off determinism contract that `engine_fingerprint` and NFR-2 depend on, and the elimination of the originally-planned Python layer in favour of the in-Rust PulseHive SDK.

Decisions of this weight need a durable, dated, reviewable record. Otherwise the *why* behind a choice — especially a choice made to settle an audit challenge — evaporates, and a future session (human or agent) is liable to "simplify" away a constraint that exists for a reason it can no longer see. This is acute for a solo developer pairing with rotating AI agents (Claude Code, Codex): there is no colleague whose memory backstops the rationale.

The Nygard ADR format addresses this directly: each significant decision gets its own short, dated, immutable document with a fixed structure (Status / Context / Decision / Consequences), so the decision history can be read chronologically and a superseded decision is marked rather than silently overwritten.

A naming distinction matters for this dual-repo workspace. **Product / architecture decisions** that constrain the shipped system (the canonical repo) belong here in `docs/adr/`. **Process decisions** that govern how the AI workspace operates (memory-bank conventions, scaffold-dev cadence, agent-routing policy) belong in the paired AI workspace's own ADR space, so that product-history readers are not forced to wade through tooling meta-decisions and vice versa.

## Decision

We will use lightweight, MADR-style Architecture Decision Records, as described by Michael Nygard, under `docs/adr/` in the **canonical** repository:

- `NNNN-<slug>.md` — one file per decision; numbers are zero-padded to four digits and **never reused** (a withdrawn decision keeps its number).
- Each ADR has the sections **Status** / **Context** / **Decision** / **Consequences**. Longer ADRs may add **Alternatives considered**.
- Status values: `Proposed` · `Accepted` · `Deprecated` · `Superseded by NNNN`. A superseding ADR links back to the one it replaces; the superseded ADR is never deleted.
- A decision warrants an ADR when it **constrains future work** — a port boundary, a storage contract, a determinism guarantee, a backend-coupling stance. Routine implementation detail does not.
- **Product / architecture ADRs live here** (canonical `docs/adr/`). **Process ADRs** (how the AI workspace and scaffold-dev cadence operate) live in the paired AI workspace, not in this directory.
- New ADRs are authored via the scaffold-dev `/adr-new` slash command, which seeds the dated MADR-lite skeleton.

## Consequences

**Positive**

- Every load-bearing decision gets a written record at the moment it is made, with the rationale (often an audit concession) captured while it is still fresh.
- A future reader — human or agent — can reconstruct the project's architectural evolution by reading ADRs in order, reducing the risk of "simplifying" away a constraint whose purpose was lost.
- Superseded decisions remain visible, so a reversal is itself documented rather than hidden in a diff.
- Separating product ADRs (canonical) from process ADRs (AI workspace) keeps each history legible to its audience.

**Negative / costs**

- ADR authoring is a small recurring discipline tax; skipping it on a "small" decision that later proves load-bearing reintroduces the exact gap this process exists to close.
- ADRs capture decisions, not implementation detail — they are not a substitute for the MASTER-SPEC, the SRS, or code comments, and readers must not mistake them for one.
- The product/process split requires judgement at the boundary; a misfiled ADR is a minor annoyance, not a correctness problem.

## Decision queue / decisions already made

The MASTER-SPEC has already locked the following architecture decisions. Each warrants its own ADR so the rationale is captured in the canonical history; they are listed here as a decision index and are marked **Proposed** until each is written up in full.

- **ADR-0002 — Hexagonal (ports-and-adapters) architecture** — *Proposed.* Domain logic depends only on port traits (`ExchangeAdapter`, `LlmProvider`, `StrategyRepository`, `CandleSeriesRepository`, `MarketDataSource`, `EventBus`, `Clock`, `Indicator`) living in a pure-domain module with zero external deps; every external concern is a swappable adapter, dependency direction always inward. Rationale: it is the mechanism behind the "minimize-refactor" promise (close-audit CX5) and the v1→v4 feature staging — and it makes the eventual 2-crate→10-crate split mechanical because ports already sit in the domain.

- **ADR-0003 — Single Tauri desktop shell, zero sidecars** — *Proposed.* Ship one artifact (`PulseTrader.app`): a Rust backend plus a TypeScript/React UI in a WKWebView, glued by a tauri-specta-generated command bus, with no sidecar processes. Rationale: a single code-signed, notarized, auto-updating native artifact with no Python runtime to bundle or secure; the CLI proof-of-concept and the GUI share one Rust core.

- **ADR-0004 — PulseHive as the in-Rust agent framework** — *Proposed.* Use the author-owned PulseHive Rust multi-agent SDK for agent orchestration in-process, eliminating the originally-planned Python layer. Rationale: removes the sidecar and its packaging/security burden, unblocks distribution licensing (the author owns PulseHive), and uses PulseHive `Lens` scoping for Composer/Coach context; the A2 alternative (wrap PulseHive behind PulseTrader's own thin agent port) is recorded for revisit if PulseHive churn becomes disruptive.

- **ADR-0005 — Deterministic FMA-off backtester with `engine_fingerprint`** — *Proposed.* Disable FMA/fast-math in the backtester for byte-identical floats across aarch64 and x86_64, fold the target-architecture triple into `engine_fingerprint` = hash(crate versions + rust toolchain + DSL schema version + target arch), parallelize across sweep combinations only (never within a single backtest), and gate every PR on a 100×-identical determinism test on both architectures. Rationale: byte-reproducibility (NFR-2) is the load-bearing premise that lets a backtest predict live behaviour and makes golden-file results a reviewable diff.

- **ADR-0006 — Enums + serde-tagged DSL representation** — *Proposed.* Represent the strategy DSL as Rust enums with `#[serde(tag="type")]` variants plus a `SweepableValue` enum, kept hand-rolled in v1. Rationale: compile-time exhaustiveness, free JSON round-trip, and sweep-friendliness, chosen over trait objects and embedded scripting; the A1 alternative (a schema→codegen single source of truth across Rust/tool-signatures/TS) is recorded to revisit if hand-syncing the three representations drifts painfully.

- **ADR-0007 — sqlx + versioned Parquet storage** — *Proposed.* Persist all entities and event logs in single-file embedded SQLite (WAL) via sqlx with compile-time-checked queries and `sqlx migrate`; store CandleSeries as immutable Parquet files per `(pair, timeframe, data_version)` read directly via Polars/arrow-rs (not via SQL); secrets in macOS Keychain. Rationale: type-safe, server-free, reproducible local storage with the DB as system-of-record for real-money trades, backed by the back-up-then-migrate-or-refuse-to-start protocol (NFR-12).

- **ADR-0008 — GLM-5.1-first LLM backend, decoupled behind `LlmProvider`** — *Proposed.* Route all LLM calls through a uniform `LlmProvider` port; ship v1 on GLM 5.1 (OpenAI-compatible API, version-pinned for eval reproducibility), with Claude Code / Codex subprocess providers and DeepSeek/Gemini as fast-follow registrations behind the same port. Rationale: decouples v1 delivery from new PulseHive subprocess-provider work (a config-flag swap, no refactor — FR-23), keeps spend inside the Tier A budget control loop (NFR-10), and validates the abstraction by shipping two backends across the v1→v2 boundary.
