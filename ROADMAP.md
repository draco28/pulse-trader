# PulseTrader — Roadmap

> Derived from MASTER-SPEC.md by `/plan-roadmap` on 2026-05-28.
> Co-edited by user + scaffold-dev orchestrator over time.

## Roadmap overview

PulseTrader's roadmap is organized as four **Phases** (the visionary horizon — the 5-year shape), each broken into **Sprints** (value-building windows that compound) and, where build is imminent, into **Vertical Slices** (visibility cycles — what ships demoably). The four phases trace the product's version staging: **Foundation** (v1) proves the closed compose → backtest → coach loop on a CLI; **Native Workbench** (v1.5 + v2) moves the workflow into a native macOS Tauri app and adds the research toolkit (sweeps, walk-forward, paper trading, multi-LLM backends); **Live Trading & Analytics** (v3) completes the lifecycle with capped-capital live execution, trade journaling, and the analytics dashboard; **Autonomy & Scale** (v4+) adds the auto-optimizer and a multi-strategy capital allocator.

Phase 1 is decomposed to full vertical-slice depth (12 slices across 3 sprints), each carrying requirement traceability (FR/NFR/BACKLOG) and 1–2 demo criteria — this is the build-ready surface for scaffold-dev's orchestrator. Phases 2–4 are intentionally held at sprint-level granularity; their slices will be authored via `/plan-roadmap --add-slice <sprint_id>` as each sprint approaches, so demo criteria stay grounded in real implementation context rather than speculation.

Traceability flows end-to-end: PRD use cases (UC-N) → SRS requirements (FR-N / NFR-N) → BACKLOG items (BACKLOG-N) → the vertical slices below. Each Phase-1 slice cites the requirements it satisfies, so requirement coverage is reportable and scaffold-dev work items inherit the trace links.

## Phase 1: Foundation — ~2-3 months / Q3 2026

The v1 MVP: the closed compose -> backtest -> coach loop proven end-to-end on a CLI surface. Describe a strategy in natural language; the agent composes it into the DSL via validated builder tools; a deterministic FMA-off Rust backtester runs it on historical BTCUSDT data with realistic costs (fees, funding, slippage); the AI coach proposes exactly one mutation that measurably moves expectancy. The hexagonal Rust core (domain + adapters + agent + GLM 5.1 backend), the Binance data pipeline (bulk + REST + WS), the deterministic backtester with engine_fingerprint, and SQLite persistence all exist. At phase end the '10-minute round trip' success metric is realized.

### Sprint 1.1: Data & Domain Foundation

Binance data pipeline (pulse-data: bulk+REST+WS), DSL grammar (enums+serde) + compiler, indicator engine (ta-rs wrapped behind Indicator port), SQLite persistence (sqlx + migrations). Demoable: indicators compute on real BTCUSDT data from a strategy JSON.

#### VS-1.1.1: Binance historical data pipeline

pulse-data adapter (BinanceDataSource impl of MarketDataSource port) fetches + normalizes + version-snapshots BTCUSDT M15+H4 into immutable Parquet; bulk via data.binance.vision + REST incremental top-up.

##### Traceability

- FR: FR-5
- NFR: NFR-2, NFR-9
- Backlog: BACKLOG-1

##### Demo criteria

- [ ] auto: `pulse fetch-data BTCUSDT --tf M15,H4 --years 2` produces a versioned Parquet snapshot; a second run fetches only new candles (incremental).
- [ ] user: open the Parquet snapshot and confirm OHLCV + funding columns are correct and gap-free.

#### VS-1.1.2: DSL grammar + compiler

Rust enums+serde DSL (ValueSource, Condition, ExitRule, RiskParams, SweepableValue) + compiler to evaluator tree + semver schema_version with auto-migrate.

##### Traceability

- FR: FR-3, FR-4
- NFR: None
- Backlog: BACKLOG-3

##### Demo criteria

- [ ] auto: a sample strategy JSON round-trips through serde unchanged; an invalid DSL is rejected with a correctable, field-level error.
- [ ] user: author an RSI-oversold strategy and inspect the compiled evaluator tree.

#### VS-1.1.3: Indicator engine (ta-rs wrapped)

Indicator port + ta-rs adapter (RSI/EMA/ADX/MACD/etc.) with Next<T> streaming + lookback history buffers; pinned version for determinism.

##### Traceability

- FR: FR-5
- NFR: NFR-2, NFR-12
- Backlog: BACKLOG-4

##### Demo criteria

- [ ] auto: RSI/EMA/ADX values match a pandas-ta reference within epsilon (cross-validation); indicators advance one candle at a time deterministically.
- [ ] user: view indicator series computed over the BTCUSDT snapshot.

#### VS-1.1.4: SQLite persistence + migrations

pulse-store repositories for Strategy/StrategyVersion (immutable) + sqlx compile-time-checked queries + migration protocol with backup-before-migrate.

##### Traceability

- FR: FR-4, FR-11
- NFR: NFR-2
- Backlog: BACKLOG-6

##### Demo criteria

- [ ] auto: a Strategy + immutable StrategyVersion is created and reloaded byte-identically; sqlx migration up/down runs with backup-before-migrate.
- [ ] user: inspect pulse.db schema and confirm the StrategyVersion row is immutable.

### Sprint 1.2: Deterministic Backtester

Backtest loop, realistic costs (fees/funding/slippage), MFE/MAE, regime detection, position-sizing (pulse-broker), engine_fingerprint, FMA-off cross-arch determinism. Demoable: backtest a hand-written strategy -> expectancy + trade log; 100x determinism test green.

#### VS-1.2.1: Core backtest loop + costs

pulse-backtest engine iterating primary-TF candles with HTF alignment (no look-ahead), applying taker fees + 8-hourly funding + slippage + conservative intra-bar SL/TP collision.

##### Traceability

- FR: FR-5, FR-6
- NFR: None
- Backlog: BACKLOG-6, BACKLOG-7

##### Demo criteria

- [ ] auto: backtesting a known strategy on the 1-month BTCUSDT fixture yields the expected trade count and P&L; funding is applied every 8h.
- [ ] user: read the resulting trade log and confirm fees/funding/slippage are deducted.

#### VS-1.2.2: Position sizing + MFE/MAE + regime

Shared pulse-broker compute_position_size + BinanceAdapter exchange constraints (lot step/min notional/leverage); per-trade MFE/MAE tracking; regime detection (ADX+EMA50/200).

##### Traceability

- FR: FR-5, FR-6
- NFR: NFR-3
- Backlog: BACKLOG-8

##### Demo criteria

- [ ] auto: property test — position_size × |entry-stop| == risk_amount; mfe_r >= 0 and mae_r <= 0 on every completed trade.
- [ ] user: view the regime breakdown (trending-up/down/ranging) on a backtest result.

#### VS-1.2.3: Engine fingerprint + determinism

Build-time engine_fingerprint (Cargo.lock+toolchain+dsl_schema+target arch); FMA/fast-math disabled in backtester; 100x single+parallel determinism CI test on both archs.

##### Traceability

- FR: FR-7
- NFR: NFR-2, NFR-8
- Backlog: BACKLOG-9

##### Demo criteria

- [ ] auto: the same backtest run 100x (single-threaded AND Rayon-parallel) produces an identical output hash on both aarch64 and x86_64.
- [ ] auto: every BacktestRun row carries a non-empty engine_fingerprint including the target arch.

#### VS-1.2.4: Backtest results + summary stats

SummaryStats (expectancy, win rate, profit factor, Sharpe/Sortino, max drawdown, streaks, funding/commission totals) + equity curve + BacktestRun persistence.

##### Traceability

- FR: FR-6
- NFR: None
- Backlog: BACKLOG-10

##### Demo criteria

- [ ] auto: a backtest emits all SummaryStats fields (expectancy, win rate, profit factor, Sharpe, max drawdown, streaks, funding/commission totals) + a persisted equity curve.
- [ ] user: read the expectancy and headline stats for a finished backtest.

### Sprint 1.3: AI Loop & CLI

PulseHive agent integration + GLM 5.1 provider + granular builder tools (composer) + coach framework (one mutation/turn) + CLI surface. Demoable: the full 10-minute round trip via CLI.

#### VS-1.3.1: PulseHive + GLM provider wiring

pulse-agent integrates PulseHive HiveMind + GLM 5.1 via OpenAI-compatible LlmProvider; LLMCall event logging (verbatim prompt+completion+cost+tokens) with redaction on the dispatch path.

##### Traceability

- FR: FR-3
- NFR: NFR-6
- Backlog: BACKLOG-11

##### Demo criteria

- [ ] auto: a GLM 5.1 call round-trips through PulseHive and logs an LLMCall with backend, tokens, and cost; the dispatch-path redaction strips API keys, account IDs, and raw balances from the prompt.
- [ ] user: open an LLMCall record and confirm the verbatim prompt has secrets redacted.

#### VS-1.3.2: Composer agent + builder tools

Granular server-validated builder tools (create_strategy, add_entry_signal, add_filter, set_exit_rules, set_risk_params, finalize_strategy) + composer AgentDefinition with Lens scope; LLM never emits raw DSL JSON.

##### Traceability

- FR: FR-3
- NFR: None
- Backlog: BACKLOG-12

##### Demo criteria

- [ ] auto: the prompt 'RSI oversold bounce on BTC with an H4 uptrend filter' yields a finalized, schema-valid StrategyVersion built only via builder tools (no raw DSL JSON emitted); an invalid builder-tool input returns a correctable error.
- [ ] user: watch the streamed tool calls build the DSL step by step in the CLI.

#### VS-1.3.3: Coach agent + one-mutation framework

Coach AgentDefinition reads the BacktestRun summary, proposes exactly one mutation per turn with a stated hypothesis + significance/noise-band guard; accept clones a child StrategyVersion linked to parent.

##### Traceability

- FR: FR-8, FR-9, FR-10
- NFR: None
- Backlog: BACKLOG-13

##### Demo criteria

- [ ] auto: the coach proposes exactly one mutation per turn (binary check); accepting it clones a child StrategyVersion linked to its parent.
- [ ] user: read the coach's one-line hypothesis and accept/reject/modify the proposed mutation.

#### VS-1.3.4: CLI surface + 10-min round trip

pulse CLI (setup-keys, compose, backtest, coach, list-strategies, kill-all) wiring the full loop + the automated e2e 10-minute round-trip test (mocked LLM).

##### Traceability

- FR: FR-3
- NFR: NFR-1
- Backlog: BACKLOG-14

##### Demo criteria

- [ ] auto: an automated e2e test runs compose -> backtest -> coach -> accept -> re-backtest with a mocked LLM, completes in under 10 minutes, and produces an improved child version with logged LLM cost.
- [ ] user: from a cold start, run the real 10-minute round trip end-to-end via the `pulse` CLI.

## Phase 2: Native Workbench — ~6-9 months

The product becomes the native macOS app (v1.5) and gains the research toolkit (v2). The user works entirely through the Tauri UI -- chat-first Strategy Designer, version-tree Library, Backtest Lab -- and can run parameter sweeps, walk-forward + Monte Carlo robustness checks, paper-trade a graduated strategy, route across multiple LLM backends (Claude Code/Codex subprocess + DeepSeek/Gemini) with per-backend cost tracking, and apply a news/macro calendar filter. The CLI becomes a power-user fallback.

### Sprint 2.1: Native App Shell (v1.5)

Tauri shell, command bus (tauri-specta + round-trip test), AgentEvent streaming to UI, Strategy Library (version tree), Strategy Designer (chat + DSL preview unified event stream), Backtest Lab. Demoable: the v1 loop entirely in the native UI.

### Sprint 2.2: Research Toolkit (v2)

Parameter sweeps (Rayon, across-combo parallelism), walk-forward + Monte Carlo robustness, sweep heatmap. Demoable: a 24-combo sweep + walk-forward rendered as a heatmap in the UI.

### Sprint 2.3: Multi-Backend & Paper Trading (v2)

Multi-LLM backend (Claude Code/Codex subprocess via PulseHive work items) + per-backend cost tracking + news/macro calendar filter + paper trading + manual graduation gate (advisory stats). Demoable: paper-trade a strategy, compare backends, view cost dashboard.

## Phase 3: Live Trading & Analytics — ~12-18 months

The strategy lifecycle completes -- strategies graduate paper -> live with capped real capital (v3). Supervised Tokio execution engine (kill switch, auto-pause on broker-feed loss, mandatory human order confirmation), Deployment Dashboard, first-class trade journaling + annotations across backtest/paper/live/manual, and the Analytics Dashboard with growth-rate forward projection (Monte Carlo) and a tax-lot/FIFO ledger. Graduation gates (backtest->paper->live) are first-class with a calibration loop measuring backtest-vs-live fidelity.

### Sprint 3.1: Live Execution Engine

Supervised Tokio WebSocket actor (backoff reconnect, gap-fill, heartbeat, health surface), order placement, position management, kill switch, auto-pause, mandatory human order confirmation, pre-execution logging, capped capital. Demoable: live-deploy with $10-20, place + close a real order safely.

### Sprint 3.2: Trade Journal & Calibration

Unified journal + annotations (backtest/paper/live/manual), paper-vs-live reconciliation test, backtest-vs-live fidelity calibration loop. Demoable: journal a live trade, see a backtest-vs-live fidelity report.

### Sprint 3.3: Analytics Dashboard

Growth-rate forward projection (Monte Carlo), strategy comparison, LLM cost analytics, regime-conditional performance, tax-lot/FIFO ledger. Demoable: the analytics dashboard on real live-trade data.

## Phase 4: Autonomy & Scale — ~year 2+

The system runs multiple strategies and improves them autonomously (v4+). The auto-optimizer iterates strategies toward objectives within constraints (one mutation per iteration, overfitting guard); a multi-strategy capital allocator coordinates 1-3 live deployments under an Account-level risk aggregate (cross-deployment margin + leverage cap + account kill switch); optional distribution beyond the author and cross-platform (Linux/Windows) if demand exists.

### Sprint 4.1: Auto-Optimizer

Goal-directed autonomous strategy improvement: objectives, constraints, budget, one-mutation-per-iteration loop, overfitting guard (train/val split + walk-forward). Demoable: auto_optimize -> improved strategy with version tree + mutation log.

### Sprint 4.2: Multi-Strategy Portfolio

Account aggregate (cross-deployment margin + leverage cap + account kill switch), multi-strategy capital allocator. Demoable: 2-3 strategies live under one account with coordinated risk.

### Sprint 4.3: Distribution & Cross-Platform (optional)

Code-signing/notarization hardening for distribution, compliance review (disclaimer/jurisdiction framing), optional Linux/Windows port. Demoable: a distributable signed build; optional Linux run.

