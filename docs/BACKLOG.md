# PulseTrader — Product Backlog

**Last derived from MASTER-SPEC.md @ 2026-05-28T15:12:07Z**

---

Backlog items are ordered by version (v1 → v4) then priority (P0 → P2). Each item carries a Description, Priority, Version, `Traces:` (FR/UC IDs from the PRD and SRS), and Acceptance criteria. IDs (`BACKLOG-N`) are stable; slices in ROADMAP cite them.

## v1 — MVP (CLI proof-of-concept)

### BACKLOG-1 — Binance Futures data pipeline (`pulse-data`)

Implement the `BinanceDataSource` adapter behind the `MarketDataSource` port: historical backfill via `data.binance.vision` bulk dumps, REST incremental top-up, and a WebSocket live channel. Persist versioned `CandleSeries` Parquet files per `(pair, timeframe, data_version)`. First-class v1 deliverable.

- **Priority:** P0
- **Version:** v1
- **Traces:** UC-1, UC-3, FR-5, NFR-9
- **Acceptance:** A 1-year BTCUSDT M15+H4 snapshot downloads to immutable Parquet; reads are byte-identical across runs; the same pipeline feeds backtest, paper, and live.

### BACKLOG-2 — Credential setup & Keychain storage (`pulse setup-keys`)

CLI command to capture Binance Futures keys and the LLM backend key, store them in macOS Keychain via the `keyring` crate, and verify Binance key permissions (trading enabled, withdrawal disabled), refusing to proceed if withdrawal scope is on.

- **Priority:** P0
- **Version:** v1
- **Traces:** UC-1, FR-1, FR-2, NFR-5
- **Acceptance:** Keys land in Keychain (no plaintext on disk); a withdrawal-scoped key is rejected with a clear error; a no-withdraw key proceeds.

### BACKLOG-3 — DSL grammar (Rust enums + serde tagged) and compiler

Define the strategy DSL as serde-tagged Rust enums (entry signals, filters, exit rules, risk params, `SweepableValue`) with a `dsl_schema_version` semver, compile-time exhaustiveness, and a DSL→executable compiler. Per close-audit CL1, v1 may begin with one hardcoded template before the full grammar.

- **Priority:** P0
- **Version:** v1
- **Traces:** UC-2, FR-3, FR-4
- **Acceptance:** A DSL document round-trips through JSON losslessly; invalid documents fail compilation with a typed error; `dsl_original` is preserved.

### BACKLOG-4 — Deterministic backtest engine (`pulse-engine`) with realistic costs

Implement the sequential backtester: indicators via **ta-rs** behind the `Indicator` port; fees, funding, slippage, intra-bar collision, and liquidation-price modeling; regime classification; MFE/MAE; equity curve. FMA/fast-math disabled; `engine_fingerprint` embedded in every BacktestRun.

- **Priority:** P0
- **Version:** v1
- **Traces:** UC-3, FR-5, FR-6, FR-7, NFR-2, NFR-3
- **Acceptance:** A 1-year run completes < 5s and reports expectancy, regime breakdown, MFE/MAE (`mfe_r ≥ 0`, `mae_r ≤ 0`), equity curve, and trade log; backtest Trades are journaled.

### BACKLOG-5 — Shared position-sizing / money-math crate (`pulse-broker`)

Extract position-sizing, P&L, R-multiple, MFE/MAE, and funding math into one shared crate used by both backtester and (future) live executor, property-tested for byte-equal sim/live sizing.

- **Priority:** P0
- **Version:** v1
- **Traces:** FR-5, FR-6, NFR-3, NFR-8
- **Acceptance:** The sim/live byte-equality property test passes; money-math coverage is 100% with no mocking.

### BACKLOG-6 — SQLite persistence & migration protocol (`pulse-store`)

Implement entity + event-log persistence in SQLite (WAL) via sqlx compile-time-checked queries: Strategy, StrategyVersion (immutable tree), BacktestRun, Trade (append-only with TradeCorrection), CoachingSession, LLMCall, GraduationEvent. On startup, check `schema_version`; if behind, back up `pulse.db`, migrate in a transaction, verify, and restore + refuse to start on failure.

- **Priority:** P0
- **Version:** v1
- **Traces:** FR-4, FR-6, FR-21, FR-24, NFR-12
- **Acceptance:** Migrations create a timestamped backup first; a forced migration failure restores the backup and aborts startup; StrategyVersion and Trade rows are immutable.

### BACKLOG-7 — PulseHive agent loop + GLM 5.1 backend + builder tools

Wire PulseHive in-Rust with the composer agent, GLM 5.1 via the OpenAI-compatible `LlmProvider`, and the six granular builder tools (`create_strategy` → `finalize_strategy`), each server-validating against the DSL schema and returning correctable errors. Persist every LLMCall (verbatim prompt + completion) with redaction on both the write and dispatch paths.

- **Priority:** P0
- **Version:** v1
- **Traces:** UC-2, FR-3, FR-23, FR-24, NFR-6
- **Acceptance:** An NL prompt yields a schema-valid StrategyVersion via visible tool-call steps; no secret or absolute balance appears in any captured prompt or LLMCall row.

### BACKLOG-8 — Coach framework (one mutation per turn) with significance guard

Implement the coach agent that reads a BacktestRun summary and proposes exactly one mutation per turn with a stated hypothesis referencing confidence intervals; enforce single-mutation at the tool signature, system prompt, and framework; on accept, clone a child StrategyVersion and re-backtest. An improvement counts as "better" only if it exceeds the noise band for the trade count.

- **Priority:** P0
- **Version:** v1
- **Traces:** UC-4, FR-8, FR-9, FR-10
- **Acceptance:** Each turn yields exactly one hypothesis-backed mutation; accepting it creates a child version + CoachingSession + comparable BacktestRun; within-noise gains are not flagged as improvements.

### BACKLOG-9 — CLI surface & strategy listing (`pulse`)

Implement the v1 CLI: `compose`, `backtest`, `coach`, `list-strategies`, plus `--json` / `NO_COLOR` support, persisting to the shared SQLite DB. `list-strategies` shows the version tree.

- **Priority:** P0
- **Version:** v1
- **Traces:** UC-2, UC-3, UC-4, UC-5, FR-11
- **Acceptance:** The full compose → backtest → coach → accept → re-backtest loop runs from the CLI; `list-strategies` renders strategies and version subtrees.

### BACKLOG-10 — Kill switch (`pulse kill-all` + SIGTERM)

Implement the CLI kill switch and SIGTERM handler that disables all deployments (and, once live exists, closes positions). v1 has no live positions but the surface and disable path exist from day 1.

- **Priority:** P0
- **Version:** v1
- **Traces:** UC-11, FR-20, NFR-4
- **Acceptance:** `pulse kill-all` and SIGTERM both disable all deployments and (when live) close open positions; a critical notification fires.

### BACKLOG-11 — 10-minute-round-trip E2E test + LLM eval fixture set

Author the scripted E2E test (compose → backtest → coach → accept → re-backtest with mocked LLM) and the LLM eval fixture set (~20 prompts + ~10 backtest fixtures) that gates CI; assert format validity, groundedness, one-mutation discipline, cost, and latency. Per close-audit CL4 this is an explicit early v1 deliverable.

- **Priority:** P0
- **Version:** v1
- **Traces:** UC-2, UC-3, UC-4, FR-8, NFR-1, NFR-8
- **Acceptance:** The E2E test runs the round trip with a mocked LLM and asserts updated expectancy; the eval harness auto-asserts dimensions 1–5 and gates prompt/tool PRs.

### BACKLOG-12 — Cross-architecture determinism CI gate

Implement the determinism test: run the same backtest 100× (single and parallel) on both aarch64 and x86_64, asserting identical result hashes; golden-file backtest results so any output change is a reviewed diff.

- **Priority:** P0
- **Version:** v1
- **Traces:** FR-7, NFR-2, NFR-9
- **Acceptance:** The 100× determinism test passes on both architectures in CI on every PR; a changed output surfaces as a golden-file diff.

### BACKLOG-13 — Seed starter strategies & per-session cost display

Ship 3–5 seed strategies so a first-run user can backtest immediately and learn by example, and display running per-session LLM cost (CLI line / status pill). Per close-audit CL4 / CL3 tracking baseline.

- **Priority:** P1
- **Version:** v1
- **Traces:** UC-1, UC-14, FR-24
- **Acceptance:** A fresh install can backtest a seed strategy with no authoring; session cost is shown after each LLM turn.

### BACKLOG-14 — Dependency & model pinning, license/advisory gates

Pin major dependency versions and the GLM 5.1 model version (for eval reproducibility), add cargo-deny (licenses + advisories) on top of cargo-audit, and commit all lockfiles for reproducible builds. Per close-audit CX4.

- **Priority:** P1
- **Version:** v1
- **Traces:** NFR-2, NFR-12
- **Acceptance:** cargo-deny and cargo-audit pass in CI; lockfiles are committed; the GLM model version is pinned in config.

## v1.5 — Native app shell

### BACKLOG-15 — Tauri app shell with chat-first Strategy Designer

Re-surface the v1 workflow in `PulseTrader.app` (Tauri + React/Vite in WKWebView) with the onboarding wizard, Strategy Library (default-collapsed version tree), Strategy Designer (chat + live DSL preview as projections over one event stream), and Backtest Lab. Tauri command + AgentEvent surfaces type-generated via tauri-specta with an IPC round-trip test. WCAG 2.1 AA.

- **Priority:** P0
- **Version:** v1.5
- **Traces:** UC-2, UC-3, UC-4, UC-5, FR-11, FR-23, NFR-1, NFR-9
- **Acceptance:** Cold-start to first backtested strategy in < 10 minutes through the GUI; the DSL and chat panes never drift; the IPC round-trip test passes; the app is code-signed, notarized, and auto-updating (signed Ed25519 bundles).

## v2 — Multi-backend, paper trading, robustness

### BACKLOG-16 — Multi-LLM backend abstraction (subprocess + APIs) & per-backend cost tracking

Register Claude Code and Codex subprocess providers plus DeepSeek/Gemini behind the existing `LlmProvider` port (zero refactor), add backend-comparison mode, and track per-backend tokens + cost via the LLMCall log (F8).

- **Priority:** P0
- **Version:** v2
- **Traces:** UC-14, FR-23, FR-24, NFR-10
- **Acceptance:** Backend swap is a config flag with no domain-code change; backend-comparison runs the same prompt across backends; per-backend cost is reported.

### BACKLOG-17 — Paper trading & graduation gate (`pulse graduate`)

Implement the Deployment guarded state machine, paper execution on the live Binance feed (same engine), GraduationEvent logging from all three trigger paths, and the manual advisory paper graduation gate (F5).

- **Priority:** P0
- **Version:** v2
- **Traces:** UC-8, FR-14, FR-15
- **Acceptance:** Illegal transitions are rejected; paper Trades are journaled with latency timestamps; all three trigger paths write to one GraduationEvent log.

### BACKLOG-18 — Backtest-vs-paper fidelity & calibration loop

Surface backtest stats, paper stats, and the advisory Bayesian P(paper ≥ backtest × tolerance); implement the calibration loop that measures the gap and feeds a slippage-model correction; document v1-modeled vs deferred microstructure effects (F5 / C-FIDELITY).

- **Priority:** P0
- **Version:** v2
- **Traces:** UC-9, FR-16
- **Acceptance:** The advisory probability displays before manual graduation; a calibration run records the measured gap and updates the slippage model; the modeled-vs-deferred list ships.

### BACKLOG-19 — Parameter sweeps + walk-forward + Monte Carlo robustness

Implement sweep grids (Rayon across combos, RAM-capped concurrency, Parquet streaming for large shared data), walk-forward rolling windows, and Monte Carlo robustness with false-discovery control; sweep heatmap visualization (F7).

- **Priority:** P1
- **Version:** v2
- **Traces:** UC-6, FR-12, NFR-1
- **Acceptance:** A 24-combo sweep completes < 30s and renders a heatmap; walk-forward produces train/validation split metrics; concurrency is capped by available RAM.

### BACKLOG-20 — News / macro calendar filter (surface)

Add a news/macro calendar source (TradingEconomics / Forex Factory, minimal) and a strategy filter that surfaces high-impact windows; treat feed content as untrusted, prompt-injection-aware input (F4 v2).

- **Priority:** P2
- **Version:** v2
- **Traces:** UC-7, FR-13, NFR-7
- **Acceptance:** High-impact windows are surfaced on the strategy timeline; injected directives in feed content do not change agent behavior.

### BACKLOG-21 — Budget enforcement control loop

Implement the budget control loop: monthly budget setting, 80% notify, 100% route to cheapest/subscription-only, hard ceiling disables autonomous/optimizer runs while keeping interactive use available (F8 / close-audit CL3).

- **Priority:** P1
- **Version:** v2
- **Traces:** UC-14, UC-15, FR-25, NFR-10
- **Acceptance:** Crossing 80% notifies; crossing 100% changes routing; a simulated optimizer overrun halts at the hard ceiling.

## v3 — Live execution, analytics, journal

### BACKLOG-22 — Live execution engine (supervised WebSocket actor)

Implement the live executor: Tokio-supervised WebSocket actor with exponential-backoff reconnect, REST gap-fill, heartbeat watchdog, and auto-pause-all + broker-feed-down GraduationEvent on prolonged outage; pre-execution DB logging of every order.

- **Priority:** P0
- **Version:** v3
- **Traces:** UC-10, FR-18, FR-19, NFR-4
- **Acceptance:** A forced feed drop triggers reconnect; a prolonged outage auto-pauses all deployments; every order is DB-logged before the exchange call (reconciliation record survives a crash).

### BACKLOG-23 — Live graduation gate, capped capital & mandatory order confirmation

Implement the manual live graduation gate behind a feature flag (default OFF), per-deployment capped-capital setting ($10–20 first live), and mandatory human confirmation for every order-affecting action (no LLM-initiated live orders).

- **Priority:** P0
- **Version:** v3
- **Traces:** UC-10, FR-17, NFR-4, NFR-7
- **Acceptance:** Live trading is flag-gated off by default; no live order executes without explicit confirmation; the first live deployment is capital-capped.

### BACKLOG-24 — Trade journal annotations & querying

Add free-text annotations on Trades, journal filtering/querying, and the paper-vs-live reconciliation test proving the engine is identical across modes (F1).

- **Priority:** P1
- **Version:** v3
- **Traces:** UC-12, FR-21, NFR-3
- **Acceptance:** Trades carry user annotations; the journal supports filtered queries; the paper-vs-live reconciliation test passes.

### BACKLOG-25 — Analytics dashboard, growth projection & tax-lot ledger

Build the Analytics Dashboard (P&L, equity curve, regime breakdown, per-strategy metrics), the growth-rate forward projection, and the tax-lot / FIFO ledger (F1/F2/F3).

- **Priority:** P1
- **Version:** v3
- **Traces:** UC-13, FR-22
- **Acceptance:** The dashboard renders from real journal data; the growth projection computes from trade history; the FIFO ledger reconciles realized lots.

### BACKLOG-26 — News auto-block (v3) & live alerting

Upgrade the news filter to auto-block entries during flagged windows, and wire critical native notifications (kill-switch fired, max-drawdown breach, feed down, order rejected, migration failure) plus configurable informational alerts (F4 v3).

- **Priority:** P2
- **Version:** v3
- **Traces:** UC-7, UC-11, FR-13, NFR-11
- **Acceptance:** Entries within a flagged window are auto-suppressed; each critical event raises a native notification with no external egress.

## v4+ — Automation & multi-strategy

### BACKLOG-27 — Auto-optimizer agent

Implement the autonomous mutation/sweep search agent operating within the budget control loop and robustness guards (walk-forward + Monte Carlo + false-discovery control); disabled at the budget hard ceiling (F6 / A3).

- **Priority:** P1
- **Version:** v4
- **Traces:** UC-15, FR-26, FR-25, NFR-10
- **Acceptance:** The optimizer runs a bounded autonomous search that halts at the hard ceiling; proposed strategies must pass robustness gates before promotion.

### BACKLOG-28 — Multi-strategy capital allocator (Account aggregate)

Introduce the v4 Account aggregate: total margin tracking, cross-deployment leverage cap, account-level kill switch, and the real `account_id` FK on Deployment (replacing the v1–v3 'default' singleton). Explicitly accepted as boundary-touching (close-audit CX5).

- **Priority:** P2
- **Version:** v4
- **Traces:** UC-15, FR-26, NFR-4
- **Acceptance:** The Account aggregate enforces the cross-deployment leverage cap and an account-level kill switch; Deployment carries a real `account_id` FK.

## See also

- [MASTER-SPEC §Phase 1](../MASTER-SPEC.md#phase-1-foundation)
- [PRD](./PRD.md)
- [SRS](./SRS.md)
