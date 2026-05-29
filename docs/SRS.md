# PulseTrader — Software Requirements Specification

**Last derived from MASTER-SPEC.md @ 2026-05-28T15:12:07Z**

---

## 1. Introduction

This document specifies the functional and non-functional requirements for PulseTrader. Functional requirements (FR-N) describe what the system shall do and trace to the PRD use cases (UC-N). Non-functional requirements (NFR-N) describe quality attributes and cite their source spec area and a verifiable acceptance check. Backlog items (BACKLOG-N) trace back to these FR/UC IDs.

## 2. Functional Requirements

**FR-1** — The system shall store all sensitive credentials (Binance Futures API keys, LLM provider API keys) in the macOS Keychain via the Rust `keyring` crate, never in plaintext files.
- Traces: UC-1
- Acceptance: After setup, no credential appears on disk outside Keychain; a Keychain read returns the stored key.

**FR-2** — The system shall verify, at credential setup, that the Binance Futures key has trading enabled and withdrawal disabled, and shall refuse to start the executor if withdrawal scope is present.
- Traces: UC-1, UC-10
- Acceptance: Given a key with withdrawal scope, setup/executor start aborts with a clear error; given a no-withdraw key, it proceeds.

**FR-3** — The system shall accept a natural-language strategy target and, via a composer agent, emit a sequence of granular server-validated builder tool calls (`create_strategy`, `add_entry_signal`, `add_filter`, `set_exit_rules`, `set_risk_params`, `finalize_strategy`) that produce a schema-valid DSL strategy; the LLM shall never emit raw DSL JSON directly.
- Traces: UC-2
- Acceptance: A free-text prompt yields a finalized StrategyVersion whose DSL passes schema validation; each builder call rejects invalid input with a correctable error.

**FR-4** — The system shall persist each finalized StrategyVersion as an immutable DSL snapshot carrying `dsl_schema_version` (semver), `dsl`, `dsl_original`, provenance (`created_by`, `creating_llm_call_ids`), and an optional `parent_version_id` forming a version tree.
- Traces: UC-2, UC-4, UC-5
- Acceptance: A written StrategyVersion cannot be mutated; a cloned child references its parent; `dsl_original` is preserved verbatim.

**FR-5** — The system shall compile a StrategyVersion's DSL and backtest it against a versioned CandleSeries data snapshot, applying realistic costs: trading fees, perp funding, slippage, intra-bar collision, and liquidation-price modeling.
- Traces: UC-3
- Acceptance: A BacktestRun completes against a 1-year BTCUSDT snapshot and reports cost-adjusted results that include funding and liquidation effects.

**FR-6** — The system shall produce, for each BacktestRun, expectancy, regime breakdown (TrendingUp / TrendingDown / Ranging), MFE/MAE (with `mfe_r ≥ 0`, `mae_r ≤ 0`), an equity curve, and a full trade log, and shall journal the resulting backtest Trades.
- Traces: UC-3, UC-12
- Acceptance: BacktestRun output contains all listed fields; backtest Trades appear in the journal with `source=backtest`.

**FR-7** — The system shall record, on every BacktestRun, an `engine_fingerprint` = hash(crate versions + rust toolchain + DSL schema version + target architecture), such that the same fingerprint plus the same `data_snapshot_id` yields byte-identical results and a cross-fingerprint comparison emits a warning.
- Traces: UC-3
- Acceptance: Two runs with identical fingerprint + snapshot produce an identical result hash; a mismatched fingerprint comparison surfaces a warning.

**FR-8** — The system's coach agent shall, given a BacktestRun, propose exactly one mutation per turn with a stated hypothesis, referencing confidence intervals rather than point estimates; the user may accept, reject, or modify.
- Traces: UC-4
- Acceptance: A CoachingSession turn yields exactly one mutation proposal carrying a hypothesis and a CI-referenced rationale; multi-mutation output is rejected at the tool signature.

**FR-9** — On acceptance of a coach mutation, the system shall clone a child StrategyVersion with the mutation applied, record the CoachingSession, and support re-backtest and side-by-side comparison against the parent.
- Traces: UC-4, UC-5
- Acceptance: Accepting a mutation creates a child version, a CoachingSession row, and a comparable BacktestRun; the two versions are viewable side-by-side.

**FR-10** — The system shall treat a coach mutation as "accepted as better" only when the expectancy improvement exceeds the statistical noise band given the trade count.
- Traces: UC-4
- Acceptance: A within-noise improvement is not flagged as a genuine improvement; an above-noise improvement is.

**FR-11** — The system shall let the user browse, clone, tag, pin, archive, and compare strategies and their default-collapsed version subtrees through trivial UI/CLI affordances that do not invoke the LLM agent.
- Traces: UC-5
- Acceptance: Clone/tag/pin/archive/compare operations complete without an LLM call; archiving a live strategy requires closing positions first.

**FR-12** — The system shall execute parameter sweeps across a defined parameter grid, parallelizing across combinations (never within a single backtest), and shall run walk-forward and Monte Carlo robustness analyses.
- Traces: UC-6
- Acceptance: A 24-combo sweep returns a heatmap and per-combo results; walk-forward produces train/validation split metrics.

**FR-13** — The system shall support a news / macro calendar filter that surfaces high-impact windows (v2) and auto-blocks entries during them (v3), and shall treat imported strategy text and news-feed content as untrusted, prompt-injection-aware input.
- Traces: UC-7
- Acceptance: Entries within a flagged window are suppressed (v3); injected directives in feed content do not alter agent behavior.

**FR-14** — The system shall create and advance a Deployment through a guarded state machine, emitting a GraduationEvent on transition, and shall support paper graduation from any of three trigger paths (CLI `pulse graduate`, coach tool surface, automatic check on paper-trade close) into one append-only log.
- Traces: UC-8, UC-10
- Acceptance: Illegal state transitions are rejected; each of the three trigger paths writes a GraduationEvent to the same log; `killed` is terminal.

**FR-15** — The system shall run paper trading on the live Binance feed using the same engine as backtest and live, journaling paper Trades (`source=paper`) with all four timestamps so signal-to-fill latency is captured.
- Traces: UC-8
- Acceptance: A paper-active Deployment produces journaled paper Trades with non-null fill timestamps and computed `latency_ms`.

**FR-16** — The system shall surface a fidelity comparison — backtest stats, paper stats, and an advisory Bayesian P(paper_expectancy ≥ backtest_expectancy × tolerance) — and shall run a calibration loop that measures the backtest-vs-paper gap and feeds it back as a slippage-model correction.
- Traces: UC-9
- Acceptance: The advisory probability is displayed before manual graduation; the calibration run updates the slippage model and records the measured gap.

**FR-17** — The system shall require explicit manual user approval for live graduation and for every order-affecting action, shall enforce a per-deployment capped-capital setting, and shall never place an LLM-initiated live order without human confirmation.
- Traces: UC-10
- Acceptance: No live order is placed absent an explicit user confirmation; the first live deployment is capped at the configured amount ($10–20).

**FR-18** — The system shall log every order placement to the local database before the exchange call, so that a crash between log and ACK leaves a reconciliation record.
- Traces: UC-10
- Acceptance: A simulated crash after DB-log/before-ACK leaves a pending order record discoverable on restart.

**FR-19** — The system shall drive live execution via a supervised WebSocket actor with exponential-backoff reconnect, REST gap-fill on reconnect, and a heartbeat watchdog; a prolonged feed outage shall auto-pause all Deployments and emit a broker-feed-down GraduationEvent.
- Traces: UC-10
- Acceptance: A forced feed drop triggers reconnect attempts; a prolonged outage auto-pauses deployments and emits the feed-down event.

**FR-20** — The system shall provide a kill switch (`pulse kill-all`, SIGTERM handler, and native "Kill All" menu) that closes live positions and disables all deployments.
- Traces: UC-11
- Acceptance: Firing any kill-switch trigger closes open positions and leaves no active deployment.

**FR-21** — The system shall maintain a permanent, append-only trade journal with a unified Trade row shape across all sources; corrections shall be appended as TradeCorrection events and the current view computed as a deterministic projection over base + corrections; v3 shall add free-text annotations and journal querying.
- Traces: UC-12
- Acceptance: A Trade is never updated in place; applying corrections changes the projected view deterministically; v3 supports annotation and filtered queries.

**FR-22** — The system shall provide an analytics dashboard surfacing P&L, equity curve, regime breakdown, per-strategy metrics, a growth-rate forward projection, and a tax-lot / FIFO ledger.
- Traces: UC-13
- Acceptance: The dashboard renders P&L and projections from real journal data; the FIFO ledger reconciles realized lots.

**FR-23** — The system shall route all LLM calls through a uniform `LlmProvider` port so that backend selection (GLM 5.1, DeepSeek, Gemini, Claude Code / Codex subprocess) is a configuration flag requiring no code refactor, and shall support a backend-comparison mode.
- Traces: UC-14
- Acceptance: Switching the configured backend changes the provider without recompiling domain logic; backend-comparison runs the same prompt across two backends.

**FR-24** — The system shall record every LLM interaction as an LLMCall event (backend, tokens, cost, verbatim prompt + completion, redaction flags, timestamp) and shall track per-backend token and cost, surfacing session cost to the user.
- Traces: UC-14, UC-12
- Acceptance: Each LLM call writes one LLMCall row with cost and redaction flags; the UI/CLI shows the running session cost.

**FR-25** — The system shall enforce a monthly LLM budget via a control loop: notify at 80%, route new calls to the cheapest backend / subscription-only mode at 100%, and disable autonomous and optimizer runs at the hard ceiling while keeping interactive use available.
- Traces: UC-14, UC-15
- Acceptance: Crossing 80% emits a notification; crossing 100% changes routing; the hard ceiling blocks an optimizer run but not an interactive compose.

**FR-26** — The system shall provide an auto-optimizer agent (autonomous mutation/sweep search within budget and robustness guards) and a multi-strategy capital allocator (Account aggregate: total margin, cross-deployment leverage cap, account-level kill switch).
- Traces: UC-15
- Acceptance: The optimizer runs a bounded autonomous search that halts at the budget ceiling; the Account aggregate enforces the cross-deployment leverage cap.

## 3. Non-Functional Requirements

**NFR-1 (Performance)** — Hot-path latencies shall meet: UI interaction < 100ms perceived; single backtest (1y M15+H4, 1 pair) < 5s; 24-combo parameter sweep < 30s; live signal-fire → exchange order sent < 100ms; WebSocket tick → handler < 10ms; cold startup < 2s. Per-turn agent budget is 120s wall-clock with graceful degradation and cancellation at safe checkpoints.
- Source: Phase 5.3 latency table.
- Acceptance: Benchmarks on the reference Mac meet each target; a turn exceeding 120s degrades gracefully ("here's what I have so far") rather than hanging.

**NFR-2 (Reliability — determinism)** — Backtests shall be byte-reproducible: FMA/fast-math disabled in the backtester for cross-architecture-identical floats, target architecture folded into `engine_fingerprint`, and a CI test asserting 100×-identical result hashes (single and parallel) on both aarch64 and x86_64.
- Source: Phase 3 invariant #10, Phase 7.4, Phase 9.4.
- Acceptance: The determinism test passes on both architectures every PR; any output change appears as a reviewed golden-file diff.

**NFR-3 (Reliability — sim/live fidelity)** — Position-sizing and money-math shall be a single shared crate (`pulse-broker`) producing byte-equal sizes in sim and live, property-tested; backtest, paper, and live shall use the same engine and the same Binance venue.
- Source: Phase 3 invariant #3, Phase 7.4, Phase 9.4.
- Acceptance: The sim/live byte-equality property test passes; a paper-vs-live reconciliation test (v3) confirms identical engine behavior.

**NFR-4 (Reliability — fail-safe defaults)** — The system shall fail safe: broker feed down → auto-pause; DB migration failure → restore backup and refuse to start; max-drawdown breach → automatic kill-switch. Live trading shall be the only real-time-stakes surface and shall be capped so worst-case unattended failure is bounded.
- Source: Phase 10.3, Phase 4 (S1).
- Acceptance: Each failure scenario triggers its safe default in test; the live capital cap bounds maximum loss.

**NFR-5 (Security — credential isolation)** — Sensitive keys shall live only in macOS Keychain; the Binance key shall be no-withdraw-enforced; the app shall present no inbound network listener (air-gapped inbound, egress-only outbound).
- Source: Phase 4.2, 4.3, 4.4 (S4).
- Acceptance: A port scan finds no inbound listener; no plaintext key on disk; withdrawal-scoped keys are refused.

**NFR-6 (Security — LLM context redaction)** — Before any prompt is sent to any LLM, the system shall strip API keys and account IDs and normalize raw balance numbers to relative values, applying the redaction layer on both the LLMCall write path and the LLM dispatch path.
- Source: Phase 4.4 (S3).
- Acceptance: Captured prompts and stored LLMCall rows contain no secrets or absolute balances; redaction flags are set.

**NFR-7 (Security — order confirmation & untrusted input)** — No order-affecting action shall execute without explicit human confirmation; imported strategy text and news-feed content shall be treated as untrusted, prompt-injection-aware input; subprocess LLM providers (Claude Code / Codex) shall run with least privilege in personal builds only.
- Source: Phase 4, close-audit CX6.
- Acceptance: Order actions block on confirmation; injected directives in untrusted content do not alter agent behavior; subprocess providers are absent from distributable builds.

**NFR-8 (Maintainability — coverage floors)** — CI shall enforce tiered test coverage: workspace ≥ 80%, `mod domain` ≥ 90%, and money-math (position-sizing, P&L, R-multiple, MFE/MAE, funding, intra-bar collision, DSL compilation) = 100%, with no mocking of money-math.
- Source: Phase 9.1, 9.2, 9.4.
- Acceptance: A PR below any floor fails the coverage gate; money-math tests use real computation.

**NFR-9 (Portability)** — The system shall build and pass all gates on both aarch64 and x86_64 apple-darwin; the architecture shall remain hexagonal so external concerns (exchange, LLM, storage, market data) are swappable adapters behind domain ports.
- Source: Phase 5.1, Phase 8.2.
- Acceptance: The aarch64 + x86_64 build matrix is green; an adapter can be swapped behind its port without touching domain logic.

**NFR-10 (Cost)** — LLM spend shall be bounded to the Tier A budget (~$20–40/month) via subscription/systematic routing and the NFR/FR budget control loop; a single runaway autonomous/optimizer run shall be unable to exceed the month's ceiling.
- Source: Phase 2.2, close-audit CL3.
- Acceptance: A simulated optimizer overrun is halted at the hard ceiling; monthly spend stays within Tier A in normal use.

**NFR-11 (Observability — local only)** — All logs, metrics, and traces shall remain local: tracing JSON logs at `~/Library/Logs/PulseTrader/`, domain data in SQLite as metrics, correlated tracing spans per tool-call/LLM-call/backtest; no telemetry or cloud crash reporting by default; alerting via native macOS notifications only.
- Source: Phase 10.2, Phase 7.4.
- Acceptance: No network egress occurs for observability; critical events (kill-switch, drawdown breach, feed down, order rejected, migration failure) raise native notifications.

**NFR-12 (Dependency & model pinning)** — Major dependency versions shall be pinned, the GLM 5.1 model version shall be pinned for eval reproducibility, licenses and advisories shall be gated via cargo-deny + cargo-audit, and the DB-before-migration backup protocol shall be honored.
- Source: Phase 8.4, close-audit CX4.
- Acceptance: cargo-deny and cargo-audit pass in CI; a migration creates `pulse.db.bak-<version>-<timestamp>` before applying.

## 4. Security & Access (requirements view)

The application has no internal auth: a single user on a single machine, with OS login plus macOS Gatekeeper as the perimeter (covered by NFR-5). Outbound credentials are Binance Futures keys (trading-enabled, withdrawal-disabled — FR-2, S4), LLM provider keys, and Claude Code / Codex subscriptions (auth held by those CLIs). The system is single-tenant for v1–v3; the v4 Account aggregate is the natural home for a future `tenant_id`. The threat surface is egress-only with no inbound listener; data at rest lives under `~/Library/Application Support/PulseTrader/` and secrets in Keychain. The five security invariants (S1 kill switch, S2 pre-execution logging, S3 LLM redaction, S4 no-withdraw enforcement, S5 IP-whitelist mandate) are realized by FR-20, FR-18, NFR-6, FR-2, and operator documentation respectively. PulseTrader is a personal-use tool outside GDPR/HIPAA/SOC2/PCI scope; jurisdictional trading rules are the user's responsibility, with a "not financial advice" framing and a distribution-triggers-compliance-review checkpoint.

## 5. Surfaces (requirements view)

Two version-staged surfaces. The **CLI (`pulse`)** is the v1 proof-of-concept only — commands `setup-keys`, `compose`, `backtest`, `coach`, `list-strategies`, `graduate`, `kill-all` — persisting to the same SQLite DB the app uses; it respects `NO_COLOR` and a `--json` mode. The **native macOS app (`PulseTrader.app`)** is the v1.5+ end product — Tauri + TypeScript/React in a WKWebView, with Strategy Library, Strategy Designer (chat-first), Backtest Lab, Deployment Dashboard, Trade Journal, Analytics (v3+), and Settings sections; native menu bar, notifications, and dock; code-signed and notarized. The native app shall meet WCAG 2.1 AA (accessible primitives, full keyboard navigation, screen-reader labels, 3:1 UI / 4.5:1 text contrast, colorblind-safe chart palettes). Web UI, mobile, and TUI are out of scope. The primary flow targets cold-start to first backtested strategy in under 10 minutes (the 10-minute round trip), realized by UC-1 through UC-4.

## 6. Design constraints (architecture / implementation decisions)

These are binding design decisions, not functional requirements; they constrain how the FR/NFR set is satisfied. The system is a modular monolith with hexagonal (ports-and-adapters) architecture shipped as one Tauri artifact with zero sidecar processes (agent orchestration runs in-Rust via the PulseHive SDK). v1 starts as two crates (`pulse` with hexagonal modules + `ui`), growing into a 10-crate workspace via mechanical splits because ports live in the domain crate. The DSL is represented as Rust enums with serde tagged variants (compile-time exhaustiveness, free JSON round-trip, sweep-friendly), kept hand-rolled in v1 (a schema→codegen single source of truth is recorded as alternative A1 to revisit if drift becomes painful). Indicators use **ta-rs** wrapped behind an `Indicator` port, version-pinned for determinism. Persistence uses **sqlx** with compile-time-checked queries against SQLite (WAL), with Parquet candle data read directly via Polars/arrow-rs rather than SQL; there is no public HTTP/GraphQL API in v1–v3 (the internal API is the domain port traits, semver-versioned, with the DSL schema as the externally-stable contract and Tauri command + AgentEvent surfaces formally typed). Build profiles (distributable = API providers only with hardened runtime; personal = adds subprocess providers) are a build-feature flag selecting which adapters register, not a code fork. Local tooling is container-free (pinned Rust toolchain, Node + pnpm, Tauri CLI, cargo-nextest, cargo-audit, sqlx-cli, `just`), targeting clone-to-running in under 15 minutes; CI is GitHub Actions on an aarch64 + x86_64 matrix; hosting is the user's Mac with GitHub Releases for distribution and a signed Ed25519 Tauri auto-updater from v1.5. Further detail and the recorded alternatives (A1 DSL codegen, A2 PulseHive port, A3 robustness success criterion) live in MASTER-SPEC Phases 5, 7, 8 and the close-audit hardening section.

## See also

- [MASTER-SPEC](../MASTER-SPEC.md)
- [PRD](./PRD.md)
- [BACKLOG](./BACKLOG.md)
