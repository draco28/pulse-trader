# PulseTrader — Project Plan

**Last derived from MASTER-SPEC.md @ 2026-05-28T15:12:07Z**

## 1. Timeline

- **Target: 6–10 weeks** to the v1 MVP — the NL → DSL → backtest → coach loop on a CLI surface. The Rust backtester is the long-tail effort; the PulseHive agent shell scaffolds fast; TDD throughout per scaffold-dev. The estimate holds **only because v1 is decoupled from any new PulseHive feature** (v1 uses GLM 5.1 through the existing OpenAI-compatible provider — see §7).
- **Close-audit caveat (CL1).** The summed v1 scope (full DSL enum tree + compiler, ta-rs integration, Binance data pipeline, deterministic FMA-off backtester, sqlx + migration-with-backup, PulseHive + GLM wiring, six builder tools, coaching framework, CLI) under the 90% domain / 100% money-math coverage gate is realistically larger than 6–10 weeks at full-time-equivalent. **De-risking rule:** if the schedule slips, **hard-cut to one hardcoded strategy template** (prove the loop end-to-end first), then build out the full DSL grammar. Treat 6–10 weeks as the optimistic case; re-baseline if the full grammar is in scope from Sprint 1.
- **Sprint cadence:** 1-week sprints, Sprint 0 (bootstrap, done) through ~Sprint 6. Sprint 6 is a flex/hardening buffer that absorbs the realistic overrun the close-audit flagged.

## 2. Risks

- **R1 (tech) — backtester fidelity gap.** Unrealistic fee / funding / slippage / intra-bar-collision / liquidation modeling produces optimistic results that mislead the coaching loop. *Mitigation:* conservative cost defaults and explicit liquidation-price modeling in v1 (BACKLOG-4); walk-forward + Monte Carlo and the calibration loop arrive v2 (BACKLOG-18/19); a paper-trade gate sits before any live capital.
- **R2 (architecture) — wrong abstraction in the LLM-backend layer.** Over-abstraction is a complexity tax; under-abstraction forces the v2 refactor we exist to avoid. *Mitigation:* the `LlmProvider` port is designed in v1 against an explicit contract and **validated by shipping two backends (one API + one subprocess) across the v1→v2 boundary** (BACKLOG-7 then BACKLOG-16).
- **R3 (market / personal) — the strategy never crosses the profitability bar.** The system can be technically perfect yet yield negative-expectancy strategies after costs. *Mitigation:* the architecture is the deliverable, not any single strategy; the 6-month success criteria explicitly include "extensibility proven," and the coach's significance guard (BACKLOG-8, FR-10) stops noise-chasing.

## 3. Success metric

The **10-minute round trip.** From a cold start: write a strategy description → see backtest expectancy + a coach suggestion → accept → see updated expectancy, all under 10 minutes wall-clock, with the LLM cost of that round trip logged (UC-2 → UC-3 → UC-4; NFR-1). The whole loop is encoded as the scripted E2E test in BACKLOG-11. Secondary criterion (close-audit A3): "is the strategy actually *good*" is gated separately on out-of-sample / walk-forward robustness, deferred to v2.

## 4. Budget

- Monthly cap: **Tier A, ~$20–40/mo.** v1 spends on the GLM 5.1 API (cheap, OpenAI-compatible); Claude Code subscription becomes the primary backend as a **fast-follow** once the PulseHive subprocess-provider work lands (see §8 — not a v1 blocker). Hosting cost ~$0 (local Rust binary + local Parquet + local SQLite; no servers v1–v3). The budget-enforcement control loop (notify 80% / reroute 100% / hard ceiling) is a v2 deliverable (BACKLOG-21); v1 only tracks and displays per-session cost (BACKLOG-13).

## 5. Rollout plan

- **Strategy:** staged release channels (dev → nightly → stable; dogfood nightly first) + feature flags for blast-radius-sensitive surfaces (**live-trading behind a flag, default OFF**; new LLM backends behind flags; local config, no remote flag service). The graduation gates **are** the per-strategy rollout (backtest → paper → live, manual gates). Live canary = capped capital ($10–20, a first-class deployment setting).
- **Observability:** all local. Logs → tracing JSON at `~/Library/Logs/PulseTrader/pulse.log` (rotated). Metrics = domain data in SQLite (LLMCall cost, BacktestRun durations, Trade records are the metrics). Traces = tracing spans per tool-call / LLM-call / backtest with correlation IDs. The app is its own observability stack; nothing leaves the machine (NFR-11).
- **Alerting:** native macOS notifications only. Critical (always): kill-switch fired, max-drawdown breach, broker feed down (auto-pause), order rejected, DB-migration failure. Informational (configurable): graduation, position open/close, daily P&L, monthly budget threshold. No email / Slack / pager.

## 6. Sprint structure

> Sprint length: 1 week. Use the scaffold-dev slice workflow per sprint (`/orchestrate VS-N.M` → per-slice work items → `/impl-check` → `/close`). Every sprint ships behind the standard pre-merge gates: fmt, clippy `-D warnings`, nextest, tiered coverage (domain ≥90% / money-math 100% / workspace ≥80%), cross-arch determinism, cargo-audit, and the relevant `auto:` ACs.

### Sprint 0 — bootstrap (done)
- Onboarding artifact authored — 2026-05-28T15:12:07Z.
- Memory-bank + governance docs derived (PRD / SRS / BACKLOG / this plan).
- Workspace scaffolding, `rust-toolchain.toml` pin, `just` command runner, CI skeleton.
- **Exit:** repo builds, `just dev` runs, first slice ready to start.

### Sprint 1 — Data foundation + credentials
- **Goal:** the system can ingest real BTCUSDT data and hold credentials safely — the substrate everything else reads from.
- **Key deliverables:** `BinanceDataSource` behind the `MarketDataSource` port (bulk dumps from `data.binance.vision` → REST incremental top-up → WS live channel); versioned `CandleSeries` Parquet per `(pair, timeframe, data_version)`; `pulse setup-keys` with Keychain storage via the `keyring` crate and Binance no-withdraw scope verification.
- **BACKLOG addressed:** BACKLOG-1 (data pipeline), BACKLOG-2 (credentials + Keychain).
- **Exit criteria:** a 1-year BTCUSDT M15+H4 snapshot downloads to immutable Parquet and reads byte-identically across runs; a withdrawal-scoped key is refused, a no-withdraw key proceeds; no plaintext credential on disk (FR-1, FR-2, NFR-5, NFR-9).

### Sprint 2 — DSL grammar + compiler
- **Goal:** a strategy is a typed, immutable, round-trippable artifact.
- **Key deliverables:** the strategy DSL as serde-tagged Rust enums (entry signals, filters, exit rules, risk params, `SweepableValue`) with `dsl_schema_version` semver and compile-time exhaustiveness; the DSL → executable compiler with typed (`thiserror`) errors; `dsl_original` preservation and version-tree (`parent_version_id`) shape. **Per CL1, start with one hardcoded "RSI oversold + H4 uptrend" template** wired through the compiler before broadening the grammar, so downstream sprints can integrate immediately.
- **BACKLOG addressed:** BACKLOG-3 (DSL + compiler).
- **Exit criteria:** a DSL document round-trips through JSON losslessly; an invalid document fails compilation with a typed error; the demo template compiles (FR-3, FR-4).

### Sprint 3 — Indicator + backtest engine + money-math
- **Goal:** a compiled strategy produces a reproducible, cost-realistic backtest. This is the long-tail sprint — budget conservatively.
- **Key deliverables:** the `Indicator` port with **ta-rs** wrapped behind it (version-pinned for determinism); the sequential backtester applying fees, funding, slippage, intra-bar collision, and **liquidation-price modeling** (close-audit C-FIDELITY); regime classification (TrendingUp / TrendingDown / Ranging), MFE/MAE (`mfe_r ≥ 0`, `mae_r ≤ 0`), equity curve; the shared `pulse-broker` money-math crate (position sizing, P&L, R-multiple) used by backtester and the future live executor; FMA/fast-math disabled; `engine_fingerprint` embedded in every BacktestRun.
- **BACKLOG addressed:** BACKLOG-4 (engine), BACKLOG-5 (money-math crate).
- **Exit criteria:** a 1-year run completes < 5s and reports expectancy, regime breakdown, MFE/MAE, equity curve, and a full trade log; money-math coverage is 100% with no mocking; the sim/live byte-equality property test passes (FR-5, FR-6, FR-7, NFR-1, NFR-2, NFR-3, NFR-8).

### Sprint 4 — SQLite persistence + migration protocol
- **Goal:** every entity and event is durably, immutably persisted — the system-of-record for real-money trades later.
- **Key deliverables:** `pulse-store` over SQLite (WAL) via sqlx compile-time-checked queries — Strategy, immutable StrategyVersion tree, BacktestRun, append-only Trade + TradeCorrection, CoachingSession, LLMCall, GraduationEvent; the startup migration protocol (check `schema_version` → back up `pulse.db` → migrate in a transaction → verify → restore-and-refuse-to-start on failure); the IPC/type round-trip discipline for shared types.
- **BACKLOG addressed:** BACKLOG-6 (persistence + migration).
- **Exit criteria:** migrations create a timestamped `pulse.db.bak-<version>-<timestamp>` first; a forced migration failure restores the backup and aborts startup; StrategyVersion and Trade rows cannot be mutated in place (FR-4, FR-6, FR-21, FR-24, NFR-12).

### Sprint 5 — Agent loop + GLM backend + builder tools + coach
- **Goal:** the AI half of the loop works end-to-end against persisted state.
- **Key deliverables:** PulseHive wired in-Rust with the composer agent and GLM 5.1 via the OpenAI-compatible `LlmProvider`; the six granular server-validated builder tools (`create_strategy` → `add_entry_signal` → `add_filter` → `set_exit_rules` → `set_risk_params` → `finalize_strategy`), each returning correctable errors; LLMCall persistence with redaction on both the write and dispatch paths; the coach framework that proposes **exactly one** mutation per turn with a stated hypothesis referencing confidence intervals, enforced at tool signature + system prompt + framework, with the statistical-significance guard.
- **BACKLOG addressed:** BACKLOG-7 (agent + GLM + builder tools), BACKLOG-8 (coach framework + significance guard).
- **Exit criteria:** an NL prompt yields a schema-valid StrategyVersion via visible tool-call steps with no secret/absolute-balance in any captured prompt or LLMCall row; each coach turn yields one hypothesis-backed mutation; accepting it clones a child version + CoachingSession + comparable BacktestRun; within-noise gains are not flagged as improvements (FR-3, FR-8, FR-9, FR-10, FR-23, FR-24, NFR-6).

### Sprint 6 — CLI surface, kill-switch, E2E + eval gate, hardening (flex buffer)
- **Goal:** assemble the full CLI proof-of-concept and lock the success metric behind a test. This sprint also absorbs the CL1 overrun risk.
- **Key deliverables:** the `pulse` CLI (`compose`, `backtest`, `coach`, `list-strategies`, plus `--json` / `NO_COLOR`) over the shared SQLite DB; `list-strategies` rendering the version tree; the `pulse kill-all` + SIGTERM disable path (surface exists from day 1 even with no live positions); the scripted **10-minute-round-trip E2E test** (mocked LLM) and the **LLM eval fixture set** (~20 prompts + ~10 backtest fixtures) that gates CI; the cross-architecture determinism CI gate (100×, single + parallel, both archs, golden-file diffs); 3–5 **seed starter strategies** + per-session cost display; dependency + GLM-model-version pinning with cargo-deny.
- **BACKLOG addressed:** BACKLOG-9 (CLI), BACKLOG-10 (kill-switch), BACKLOG-11 (E2E + eval set), BACKLOG-12 (determinism gate), BACKLOG-13 (seeds + cost display), BACKLOG-14 (dependency/model pinning).
- **Exit criteria:** the full compose → backtest → coach → accept → re-backtest loop runs from the CLI in under 10 minutes wall-clock with logged LLM cost; the E2E test asserts updated expectancy; the eval harness auto-asserts dimensions 1–5 and gates prompt/tool PRs; the determinism test passes on both archs; cargo-deny + cargo-audit pass and lockfiles are committed (FR-11, FR-20, NFR-1, NFR-2, NFR-9, NFR-12). **This exit = v1 MVP shipped.**

> **Post-v1 (out of this plan's window):** BACKLOG-15 (v1.5 Tauri app shell), then v2 (BACKLOG-16–21: multi-backend, paper trading, fidelity/calibration, sweeps + robustness, news filter, budget control loop), v3 (BACKLOG-22–26: live execution, capped-capital live graduation, journal querying, analytics, news auto-block), v4+ (BACKLOG-27–28: auto-optimizer, Account aggregate).

## 7. Fast-follow dependency note (PulseHive subprocess providers)

PulseTrader v1 is **deliberately decoupled** from new PulseHive features: it uses GLM 5.1 through PulseHive's existing OpenAI-compatible `LlmProvider`, so no upstream PulseHive change blocks the v1 timeline. The subprocess-provider work (`SubprocessProvider` primitive, `StatefulLlmProvider` extension, `Usage::SubscriptionBilled`, tool-adapter for non-tool-native backends, `ClaudeCodeProvider` + `CodexProvider`) and the streaming-tools/agent-events work are tracked as **fast-follow** PulseHive work items, pursued at the v1→v2 boundary. When they land, the new providers register **behind the existing `LlmProvider` port with zero PulseTrader refactor** — which is precisely the R2 mitigation this plan relies on (BACKLOG-16). PulseDB coach-memory is v2+ and likely needs no PulseDB API change.

## See also

- [MASTER-SPEC](../MASTER-SPEC.md)
- [BACKLOG](./BACKLOG.md)
- [WORKFLOW](../.claude/memory-bank/WORKFLOW.md)
