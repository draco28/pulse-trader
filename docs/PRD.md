# PulseTrader — Product Requirements Document

**Last derived from MASTER-SPEC.md @ 2026-05-28T15:12:07Z**

---

## 1. Vision

PulseTrader is a native macOS application (with a CLI proof-of-concept stage) for AI-orchestrated crypto-futures strategy development. The user describes a trading target in natural language ("swing strategy, ~50% win rate, 1:2 R:R, BTC/ETH, no trading during high-impact news"); an AI agent composes it into a structured, deterministic strategy DSL via validated tool calls; a Rust engine backtests it on historical Binance Futures data with realistic costs (fees, funding, slippage); an AI coach reads the result and proposes exactly one mutation per loop with a stated hypothesis; the user accepts, rejects, or modifies; the loop repeats until the target is met; the same strategy then graduates to paper trading, then to live trading with capped capital — all surfaced through one native desktop UI with full trade journaling, P&L analytics, and growth projections. No leaving the app.

The product rests on three load-bearing commitments: extensibility without refactor (a hexagonal ports-and-adapters Rust architecture), a deterministic systematic core with a thin LLM layer (the Rust engine does all math; the LLM only composes, coaches, and explains), and bounded LLM cost (a pluggable backend that routes through the cheapest viable provider to keep a ~$20–40/month budget realistic).

## 2. Problem

Crypto-futures traders today juggle fragmented tools: TradingView for charting, Pine Script or Python for strategy code, a separate backtester, a separate execution bot, a separate analytics dashboard, and a spreadsheet for the trade journal. Iteration is slow because each tool sits in its own silo and the human is the integrator — manually moving a strategy idea from chart to code to backtest to execution to journal and back. PulseTrader collapses the full lifecycle — idea → strategy → backtest → paper → live → analytics → next idea — into one AI-conducted conversation, with a deliberately pluggable LLM backend so it stays affordable. The audience is solo, technically-comfortable traders for v1, expanding to non-coding discretionary traders in later versions.

## 3. Goals & 6-month success criteria

The product's central goal is to prove that an AI-orchestrated, deterministic-core strategy lifecycle is both fast to iterate and trustworthy enough to risk real (capped) capital. Concretely, at the 6-month mark:

1. **Lifecycle proven end-to-end.** The full lifecycle (NL → DSL → backtest → coach → paper → live with $10–20 risk) has run at least once for one strategy developed entirely inside the system.
2. **Extensibility proven.** The architecture has absorbed at least one major feature addition (e.g. parameter sweeps, or a new LLM backend) without a refactor, validating the ports-and-adapters hypothesis. (Per the close-audit, this is read as "minimize refactor": the v4 Account aggregate and v3+ tax-lot ledger are explicitly accepted as boundary-touching.)
3. **Cost bounded.** Per-iteration LLM cost is bounded — a typical session stays under the Tier A ceiling — by routing through subscription backends plus deterministic systematic logic wherever possible, with an enforcement control loop (not just tracking).

**Primary success proof — the 10-minute round trip:** from a cold start, write a strategy description → see backtest expectancy plus a coach suggestion → accept the suggestion → see updated expectancy, all in under 10 minutes of wall-clock time, with the LLM cost of that round trip logged. **Secondary success criterion (close-audit A3):** "is the strategy actually good" is gated separately on out-of-sample / walk-forward + Monte Carlo robustness with false-discovery control, not on workflow speed alone.

## 4. Personas

- **P1 — Solo quant-curious trader (v1 primary).** TA background, comfortable in a terminal, Rust/Python-fluent enough to debug. Wants conversational experimentation; cares deeply about realistic costs (fees/funding/slippage), MFE/MAE-driven coaching, and a versioned strategy tree.
- **P2 — Solo strategy operator (v2–v3, the same person later).** Running 1–3 finalized strategies live on small capital simultaneously, watching the analytics dashboard, doing weekly retros on the trade journal.
- **P3 — Discretionary trader leveling up (v3+ aspirational).** Doesn't code, knows candles and indicators, wants the agent to handle DSL composition entirely. Out of scope for v1 architecture decisions but noted for future UX work.

## 5. Use Cases

The use-case set is derived from the core closed loop, the F1–F8 feature backlog, and the version staging. Steps (a)–(f) of the core loop are must-have v1; (g)–(j) are version-staged but architecturally pre-supported from day 1. Use-case IDs are stable (UC-N) and are cited by SRS functional requirements and by backlog items.

**UC-1 — Set up credentials and data**
- **Actor:** P1
- **Goal:** Get the system trading-ready from a cold start.
- **Preconditions:** Fresh install; Binance Futures account exists; an LLM backend (GLM 5.1) is reachable.
- **Main flow:** (1) Run the onboarding wizard / `pulse setup-keys`. (2) Enter Binance Futures API keys; system verifies trading is enabled and withdrawal is disabled, refusing to proceed if withdrawal scope is on. (3) Enter / auto-detect LLM backend credentials. (4) Keys are stored in macOS Keychain. (5) Select pair(s); background historical data download begins.
- **Success outcome:** Credentials live in Keychain; candle data is available; the user can compose a strategy.
- **Version:** v1 (CLI) / v1.5+ (wizard).

**UC-2 — Compose a strategy from natural language**
- **Actor:** P1 (v3+: P3)
- **Goal:** Turn a plain-language trading target into a validated DSL strategy.
- **Preconditions:** Credentials set; candle data available.
- **Main flow:** (1) User describes the target in NL (style, R:R, win rate, pairs, constraints). (2) The composer agent emits a sequence of granular, server-validated builder tool calls (`create_strategy` → `add_entry_signal` → `add_filter` → `set_exit_rules` → `set_risk_params` → `finalize_strategy`). (3) Each tool call validates against the DSL schema and returns correctable errors; the LLM never emits raw JSON. (4) Each call surfaces as a visible step (CLI stream / Designer DSL preview). (5) A finalized, immutable StrategyVersion is written.
- **Success outcome:** A schema-valid StrategyVersion exists, with `dsl_original` preserved and provenance recorded.
- **Version:** v1.

**UC-3 — Run a backtest with realistic costs**
- **Actor:** P1
- **Goal:** Evaluate a StrategyVersion against historical data with realistic costs.
- **Preconditions:** A StrategyVersion exists; a CandleSeries data snapshot is available.
- **Main flow:** (1) User triggers a backtest (`pulse backtest` / "Run Backtest"). (2) The Rust engine compiles the DSL and runs the strategy over the snapshot, applying fees, funding, slippage, intra-bar collision, and liquidation-price modeling. (3) A BacktestRun is persisted with an `engine_fingerprint`. (4) Results are surfaced: expectancy, regime breakdown, MFE/MAE, equity curve, full trade log.
- **Success outcome:** A reproducible BacktestRun with summary stats and a trade log; backtest Trades are journaled.
- **Version:** v1.

**UC-4 — Coach-driven iteration (one mutation per loop)**
- **Actor:** P1
- **Goal:** Improve a strategy via single, hypothesis-backed mutations.
- **Preconditions:** A BacktestRun exists for the StrategyVersion.
- **Main flow:** (1) User opts into coach analysis (cost estimate shown). (2) The coach agent reads the structured backtest summary and proposes exactly one mutation with a stated hypothesis, referencing confidence intervals (not point estimates). (3) User accepts, rejects, or modifies. (4) On accept, a child StrategyVersion is cloned with the mutation; the CoachingSession is recorded. (5) The new version is re-backtested (UC-3) and results are shown side-by-side. (6) Loop until the target is met.
- **Success outcome:** A child version with measurably-different expectancy in a versioned tree; an improvement is only "accepted as better" if it exceeds the noise band for the trade count.
- **Version:** v1.

**UC-5 — Browse and manage the strategy version tree**
- **Actor:** P1
- **Goal:** Navigate, clone, tag, pin, and archive strategies and their version subtrees.
- **Preconditions:** At least one Strategy exists.
- **Main flow:** (1) View the Strategy Library (`pulse list-strategies` / card grid). (2) Drill into a strategy's default-collapsed version subtree. (3) Perform trivial UI affordances — clone, tag, archive, pin, view stats, compare — without invoking the agent. (4) Compare two versions side-by-side.
- **Success outcome:** The user can navigate and curate the strategy forest at scale (thousands of versions).
- **Version:** v1 (CLI list) / v1.5+ (tree UI).

**UC-6 — Parameter sweep and robustness analysis**
- **Actor:** P1
- **Goal:** Find robust parameter ranges and guard against overfitting.
- **Preconditions:** A StrategyVersion with sweepable parameters.
- **Main flow:** (1) User defines parameter ranges (a sweep grid). (2) The engine runs combinations in parallel (Rayon across combos, never within a single backtest). (3) Walk-forward (rolling train/test windows) and Monte Carlo robustness run as the overfitting backstop. (4) Results surface as a sweep heatmap plus train/validation split metrics.
- **Success outcome:** Robust parameter selection with false-discovery control; results feed the secondary success criterion.
- **Version:** v2 (F7).

**UC-7 — News / macro calendar filter**
- **Actor:** P1
- **Goal:** Avoid trading during high-impact news.
- **Preconditions:** A strategy with a news-avoidance constraint; calendar source configured.
- **Main flow:** (1) User adds a news/macro filter to the strategy. (2) v2 surfaces the calendar as a filter; (3) v3 auto-blocks entries during flagged windows. (4) Imported strategy text and news-feed content are treated as untrusted (prompt-injection-aware) input to the LLM.
- **Success outcome:** Entries are suppressed during high-impact windows.
- **Version:** v2 filter / v3 auto-block (F4).

**UC-8 — Graduate a strategy to paper trading**
- **Actor:** P1
- **Goal:** Promote a backtest-passing strategy to paper execution.
- **Preconditions:** A StrategyVersion has passed its backtest target; a Deployment can be created.
- **Main flow:** (1) User triggers graduation (`pulse graduate` / coach tool / auto-check on paper close — all emit a GraduationEvent to one log). (2) A Deployment is created and advances through the guarded state machine (`created → backtested_passed → paper_pending → paper_active`). (3) Paper trades run on the live Binance feed using the same engine. (4) Paper Trades are journaled (source=paper) with full timestamps (signal vs fill latency captured).
- **Success outcome:** A paper-active Deployment producing journaled paper Trades.
- **Version:** v2 (F5).

**UC-9 — Calibrate backtest-vs-paper fidelity**
- **Actor:** P1
- **Goal:** Measure and close the gap between backtest and paper results.
- **Preconditions:** A strategy has both a BacktestRun and a comparable paper period.
- **Main flow:** (1) System surfaces backtest stats, paper stats, and an advisory Bayesian P(paper_expectancy ≥ backtest_expectancy × tolerance). (2) The calibration loop measures the gap and feeds it back as a slippage-model correction. (3) The set of v1-modeled vs deferred microstructure effects is documented as known fidelity gaps.
- **Success outcome:** A measured fidelity gap and a corrected cost model; an informed manual graduation decision.
- **Version:** v2 (F5 / close-audit C-FIDELITY).

**UC-10 — Graduate a strategy to live trading with capped capital**
- **Actor:** P1 / P2
- **Goal:** Promote a paper-validated strategy to live execution with bounded blast radius.
- **Preconditions:** Paper results match backtest within tolerance; live-trading feature flag is ON; capped-capital deployment setting configured.
- **Main flow:** (1) User reviews advisory gate stats and manually approves graduation (`paper_complete → live_pending → live_active`). (2) Capital is capped ($10–20 first live). (3) Every order is logged to the local DB before the exchange call; every order-affecting action requires explicit human confirmation (no LLM-initiated live orders). (4) A supervised WebSocket actor drives execution; feed loss triggers auto-pause across deployments. (5) Live Trades are journaled (source=live).
- **Success outcome:** A live-active Deployment trading capped capital with a complete pre-execution audit trail and fail-safe defaults (kill-switch, auto-pause, max-drawdown auto-kill).
- **Version:** v3 (F5).

**UC-11 — Emergency kill switch**
- **Actor:** P1 / P2
- **Goal:** Immediately halt all live activity.
- **Preconditions:** At least one live Deployment.
- **Main flow:** (1) User fires the kill switch (`pulse kill-all` / SIGTERM / native "Kill All" menu). (2) Open live positions are closed. (3) All deployments are disabled. (4) A critical native notification fires.
- **Success outcome:** No open positions; all deployments halted.
- **Version:** v1 CLI / v1.5+ native (security invariant S1).

**UC-12 — Journal and annotate trades**
- **Actor:** P1 / P2
- **Goal:** Maintain a permanent, append-only trade journal and annotate it.
- **Preconditions:** Trades exist (any source).
- **Main flow:** (1) All Trades (backtest/paper/live/manual) flow into a permanent journal with a unified row shape. (2) Corrections are appended as TradeCorrection events; the current view is a deterministic projection over base + corrections. (3) User adds free-text annotations to trades (v3). (4) User queries / filters the journal.
- **Success outcome:** A queryable, immutable journal with append-only corrections and annotations.
- **Version:** v1 (backtest journaling) / v3 (annotations F1, querying).

**UC-13 — View analytics and growth projection**
- **Actor:** P2
- **Goal:** Understand performance and forward-project account growth.
- **Preconditions:** A trade history exists.
- **Main flow:** (1) Open the Analytics Dashboard. (2) View P&L, equity curve, regime breakdown, per-strategy metrics. (3) View growth-rate forward projection. (4) View the tax-lot / FIFO ledger.
- **Success outcome:** Actionable analytics and a growth projection from real trade data.
- **Version:** v3 (F1/F2/F3, analytics dashboard).

**UC-14 — Route across multiple LLM backends and track cost**
- **Actor:** P1
- **Goal:** Use the cheapest viable LLM backend and stay within budget.
- **Preconditions:** More than one backend configured.
- **Main flow:** (1) Calls route through the `LlmProvider` port (v1: GLM 5.1; v2+: Claude Code / Codex subprocess, DeepSeek, Gemini). (2) Per-backend token + cost is tracked via the LLMCall log and surfaced (status-bar cost pill). (3) The budget control loop notifies at 80%, switches to cheapest/subscription-only at 100%, and disables autonomous/optimizer runs at the hard ceiling. (4) Backend-comparison mode runs the same prompt across backends for quality-per-dollar.
- **Success outcome:** Backend swap is a config flag (zero refactor); spend stays within Tier A.
- **Version:** v1 (single backend + tracking) / v2 (multi-backend, F8).

**UC-15 — Auto-optimize and allocate capital across strategies**
- **Actor:** P2 (aspirational)
- **Goal:** Automate mutation search and allocate capital across multiple live strategies.
- **Preconditions:** Multiple validated strategies; Account aggregate exists.
- **Main flow:** (1) The auto-optimizer agent runs an autonomous mutation/sweep search within budget and robustness guards. (2) The multi-strategy capital allocator (Account aggregate) manages total margin, a cross-deployment leverage cap, and an account-level kill switch. (3) Autonomous runs are disabled when the budget hard ceiling is hit.
- **Success outcome:** Automated optimization and multi-strategy allocation without blowing the budget.
- **Version:** v4 (F6, auto-optimizer).

## 6. Project class

Agent or plugin — an LLM agent orchestrating structured tools over a deterministic Rust core, with pluggable LLM backends (Claude Code / Codex subprocess plus cheap APIs).

## 7. MVP cut

- **v1 (MVP, proof-of-concept):** UC-1 through UC-5 plus UC-11 and the backtest slice of UC-12 — the NL → DSL → backtest → coach loop on a CLI surface, single pair (BTCUSDT M15+H4), single LLM backend (GLM 5.1 via OpenAI-compatible API). Architecture is designed-for but not yet implementing: multi-pair, parameter sweeps, paper, live, in-app analytics, multi-backend routing. Concrete demo: author "RSI oversold + H4 uptrend" as text → ~10 minutes later see expectancy on 1 year of BTC data → coach suggests an ADX filter → accept → re-test → expectancy improves. **The CLI is the proof-of-concept surface, not the end product.** Per close-audit CL1, v1 may hard-cut by proving the loop with one hardcoded strategy template before building the full DSL grammar; per CL4, the ~20-prompt + ~10-backtest LLM eval fixture set and 3–5 seed strategies are explicit early v1 deliverables.
- **v1.5:** the workflow re-surfaced through a native macOS app shell (same Rust core, same DSL).
- **v2:** native app + multi-LLM backend abstraction + paper trading + parameter sweep + walk-forward / Monte Carlo + news/macro filter + per-backend cost tracking (UC-6 through UC-9, UC-14).
- **v3:** live execution + in-app analytics + journal querying + growth projection + graduation gates + tax-lot ledger (UC-10, UC-12 annotations, UC-13).
- **v4+:** auto-optimizer + multi-strategy capital allocator + Account aggregate (UC-15).
- **End product:** an installable, code-signed, notarized native `.app` on macOS — open from the dock, full UI, auto-update.

## 8. Domain entities

Seven first-class entities for v1–v3, plus one v4-deferred, plus first-class sub-objects / event logs (not counted in the seven).

1. **Strategy** — named idea; mutable name/tags/owner; `latest_version` and `pinned_version` are derived views, not stored fields.
2. **StrategyVersion** — immutable DSL snapshot; tree via `parent_version_id`; carries `dsl_schema_version` (semver), `dsl`, `dsl_original`, and provenance (`created_by`, `creating_llm_call_ids`).
3. **BacktestRun** — one execution of a StrategyVersion against a data snapshot; carries `engine_fingerprint` = hash(crate versions + rust toolchain + DSL schema + target architecture); summary stats + regime breakdown + equity curve.
4. **Deployment** — paper or live execution context; explicit guarded state machine (`created → backtested_passed → paper_pending → paper_active ↔ paused → paper_complete → live_pending → live_active ↔ paused → killed`); nullable `account_id`.
5. **Trade** — immutable single-trade record; `source ∈ {backtest, paper, live, manual}`; embedded `fills`; corrections via append-only event log; four timestamps (entry/exit × signal/fill) so latency is captured.
6. **CandleSeries** — versioned per `(pair, timeframe, data_version)` for byte-identical reproducibility.
7. **CoachingSession** — 1:N per BacktestRun; tracks backend used and LLM cost via the LLMCall log.
8. **(v4-deferred) Account** — multi-strategy capital allocator aggregate (total margin, cross-deployment leverage cap, account-level kill switch).

Sub-objects / event logs: **Fill** (1:N on Trade), **TradeCorrection** (append-only per Trade), **LLMCall** (cross-cutting append-only — captures backend, tokens, cost, verbatim prompt + completion with redaction flags), **GraduationEvent** (append-only per Deployment, three trigger paths into one log).

## 9. Out of scope

Web UI, mobile, and TUI surfaces; multi-tenant / shared-strategy-library hosting (v4+ aspirational only); trading on venues other than Binance Futures in v1 (the port keeps multi-venue open); TradingView integration; Slack/Discord/Telegram/email/SMS alerting (native macOS notifications only); tax-API services; financial advice or jurisdictional suitability gating (documented as the user's responsibility, with a "not financial advice" framing). Distribution beyond the author triggers a regulatory-framing review checkpoint (close-audit CX3).

## 10. Assumptions

Single user, single machine, single Binance account (single-tenant v1–v3). The user runs macOS on a machine with 16GB+ RAM (sweep concurrency is RAM-capped; large shared candle data is streamed from Parquet). Hosting cost is ~$0 (local Rust binary + local Parquet + local SQLite; no servers in v1–v3). The GLM 5.1 model version is pinned for eval reproducibility. Backtester fidelity is the load-bearing premise — conservative cost defaults, liquidation modeling, a calibration loop, and a paper-trade gate before live are mandatory mitigations. The architecture is the product: success criteria explicitly include "extensibility proven," independent of any single strategy's profitability.

## See also

- [MASTER-SPEC](../MASTER-SPEC.md)
- [SRS](./SRS.md)
- [BACKLOG](./BACKLOG.md)
- [PROJECT_PLAN](./PROJECT_PLAN.md)
