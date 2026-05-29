# PulseTrader — Executive Summary

**Project class:** Agent or plugin (AI-orchestrated, native desktop)
**Created:** 2026-05-28
**Full spec:** `pulse-trader-ai/docs/MASTER-SPEC.md`

---

## What it is

PulseTrader is a **native macOS application** for AI-orchestrated crypto-futures strategy development. You describe a trading target in natural language ("swing strategy, ~50% win rate, 1:2 R:R, BTC/ETH, avoid high-impact news"); an AI agent composes it into a deterministic strategy DSL via validated tool calls; a Rust engine backtests it on historical Binance Futures data with realistic costs; an AI coach proposes **exactly one** improvement per loop with a stated hypothesis; you accept/reject; the loop repeats until the target is met; the strategy then graduates to paper trading, then live trading with capped capital — all inside one app with full journaling and analytics.

The product collapses a today-fragmented lifecycle (charting + strategy code + backtester + execution bot + analytics + journal spreadsheet) into one AI-conducted conversation.

## Who it's for

- **v1 primary:** a solo, technically-comfortable trader (the author) who knows TA and wants conversational experimentation over Pine Script / hand-rolled backtesters.
- **Later:** the same person operating multiple live strategies (v2–v3); eventually non-coding discretionary traders (v3+ aspirational).

## The three load-bearing commitments

1. **Extensibility without rewrite** — a hexagonal (ports-and-adapters) Rust architecture so v1→v4 features (paper trading, live execution, analytics, multi-strategy allocation) are added behind stable ports, not bolted on via refactors. *(Close-audit refinement: "minimize refactor," with the v4 Account aggregate + tax-lot ledger explicitly accepted as boundary-touching.)*
2. **Deterministic core, thin LLM layer** — the Rust engine does all math; the DSL is the contract; the LLM only composes, coaches, and explains. It never does arithmetic, iterates, or holds state.
3. **Bounded LLM cost (~$20–40/mo)** — a pluggable `LlmProvider` port routes through the cheapest viable backend (v1: GLM 5.1; fast-follow: Claude Code & Codex subscription subprocesses; plus DeepSeek/Gemini), with a budget-enforcement control loop (not just tracking).

## Architecture at a glance

- **Single Tauri desktop app**, Rust backend + TypeScript/React UI in a WKWebView. No Python (agent orchestration runs in-Rust via **PulseHive**, the author's own multi-agent SDK).
- **Storage:** SQLite (entities + event logs) + versioned Parquet (candle data) + macOS Keychain (secrets) + local files (logs).
- **Determinism:** every backtest carries an `engine_fingerprint` (crate versions + toolchain + DSL schema + target arch); FMA/fast-math disabled for cross-chip-reproducible floats; a CI test asserts 100×-identical results on both Mac architectures.
- **Data:** Binance Futures via bulk dumps (`data.binance.vision`) + REST top-up + WebSocket live — all three pipelines (backtest/paper/live) use Binance, the trade venue, so the backtest can predict live behavior.

## Surfaces & staging

- **v1 (MVP):** a CLI proof-of-concept that validates the full compose → backtest → coach loop end-to-end (single pair, GLM 5.1 backend). The CLI is a proving ground, not the product.
- **v1.5+:** the native macOS app — chat-first Strategy Designer, version-tree Strategy Library, Backtest Lab, Deployment Dashboard, Trade Journal, Analytics. Code-signed, notarized, auto-updating.

## Success proof

The **10-minute round trip** — from cold start, describe a strategy → see backtest expectancy + a coach suggestion → accept → see improved expectancy, in under 10 minutes wall-clock, with LLM cost logged. *(Close-audit addition: a secondary criterion ties "is this strategy actually good" to out-of-sample / walk-forward robustness, not just workflow speed.)*

## Top risks (and how the architecture answers them)

| Risk | Mitigation |
|---|---|
| **Backtester fidelity gap** (backtest lies about live) | Conservative cost modeling, liquidation modeling, a calibration loop (measure backtest-vs-paper gap, correct the slippage model), explicit v1-modeled-vs-deferred microstructure list, paper-trade gate before live. |
| **AI-backend abstraction wrong** (forces a v2 refactor) | Ports-and-adapters validated by shipping two backends (API + subprocess) across the v1→v2 boundary. |
| **Strategy never crosses the profitability bar** | The architecture is the deliverable; success includes "extensibility proven." Statistical-significance guard so the coach doesn't chase noise. |
| **Live-trading blast radius** | Capped capital ($10–20 first live), automatic kill-switch + auto-pause on feed loss, mandatory human confirmation for order-affecting actions, withdrawal-disabled API keys. |

## Process note

This spec was authored through scaffold-onboard's 10-phase conversation and hardened by **3 interactive grill rounds** (domain model, UX, implementation) and **4 architect-critic audits** (Phase 3, 5, 7 premise-audits + a Phase 10 close-depth audit with codex fresh-frame) — 34 challenges surfaced, all resolved. It deliberately re-architected, rather than copied, the May-2026 initial vision spec.

## Next step

Run `/plan-roadmap` to decompose this into a Phase → Sprint → Vertical Slice hierarchy for the scaffold-dev build cycle.
