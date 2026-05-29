# PulseTrader — Model Card

**Last derived from MASTER-SPEC.md @ 2026-05-28T15:12:07Z**

> Model card describing what model is used, for what, and its limitations.

## 1. Overview

PulseTrader **does not train, fine-tune, or host its own model.** It depends on
third-party LLM backends accessed behind a single uniform `LlmProvider` port
(FR-23), and on the deterministic Rust engine for *all* computation. This card
documents the models PulseTrader *consumes*, their roles, intended use,
limitations, risks, and how user data flows through them.

**The hard boundary (MASTER-SPEC Phase 1 commitment #2):** the LLM only (a)
translates NL → DSL via validated builder tool calls, (b) reads structured
backtest summaries and proposes exactly one mutation with a hypothesis, and (c)
explains results. **The LLM never does arithmetic, never iterates a loop, and
never holds state.** Every number a user sees comes from the engine; the DSL is
the contract between the two layers. This boundary is the single most important
thing this card asserts — most of the risks below are mitigated by it.

## 2. Backends

| Backend | Version / ID | Access mode | Status | Roles |
|---|---|---|---|---|
| **GLM 5.1** | `glm-5.1` (pinned, NFR-12) | API — OpenAI-compatible, via PulseHive provider | **v1 primary** | Composer, Coach, Explainer |
| **Claude Code** | `claude` CLI (subscription) | Subscription subprocess — no API key held by us | Fast-follow (v2) | Composer, Coach, Explainer |
| **Codex** | `codex` CLI (subscription) | Subscription subprocess — no API key held by us | Fast-follow (v2) | Composer, Coach, Explainer (fresh-frame) |
| **DeepSeek** | pinned model ID | API | Fast-follow (v2) | Composer, Coach, Explainer |
| **Gemini** | pinned model ID | API | Fast-follow (v2) | Composer, Coach, Explainer |
| **Anthropic / OpenAI API** | optional | API | Optional fallback | Any role |

All backends are swappable by a configuration flag with **zero code refactor**
(FR-23 / UC-14) because they implement the same port. The same eval set runs
across all of them for the quality-per-dollar comparison (EVALS_PLAN §8). Model
versions are **pinned** for eval reproducibility (NFR-12 / close-audit CX4):
GLM 5.1 specifically, since model behaviour affects eval reproducibility.

**Build-profile gating (NFR-7):** the subprocess providers (Claude Code, Codex)
ship only in the **personal/dev build** with least-privilege subprocess
isolation. The **distributable/notarized build registers API providers only**
(clean hardened runtime + Keychain entitlements). The profile selects which
adapters register at startup — a build-feature flag, not a code fork.

## 3. Roles in the system

A single model instance plays three agent roles, each with a distinct system
prompt and a distinct PulseHive `Lens` scope (governed in
[PROMPT_GOVERNANCE](../../pulse-trader-ai/docs/PROMPT_GOVERNANCE.md)):

- **Composer** — NL strategy target → DSL via the six granular server-validated
  builder tools (`create_strategy → add_entry_signal → add_filter →
  set_exit_rules → set_risk_params → finalize_strategy`, FR-3 / UC-2). Never
  emits raw DSL JSON.
- **Coach** — reads a structured `BacktestRun` summary and proposes **exactly
  one** mutation per turn with a stated hypothesis, referencing confidence
  intervals not point estimates (FR-8 / FR-10 / UC-4).
- **Explainer** — renders engine results in plain language. No new claims beyond
  the engine's fields.
- **(v4) Auto-optimizer** — autonomous mutation/sweep search within budget and
  robustness guards (FR-26 / UC-15); same no-math/no-state boundary, bounded by
  the budget hard ceiling.

## 4. Intended use

- **Primary use:** the closed loop — describe a target (NL) → composer builds a
  DSL strategy via tools → Rust engine backtests with realistic costs → coach
  proposes one hypothesis-backed mutation → user accepts/rejects/modifies → loop
  until target met → (v2+) graduate to paper, then (v3+) live with capped
  capital → all trades journaled (UC-2 → UC-4, version-staged UC-8 → UC-12).
- **Users:** P1 solo quant-curious trader (v1 primary); P2 solo strategy
  operator (v2–v3); P3 non-coding discretionary trader (v3+ aspirational).
- **Surfaces:** CLI proof-of-concept (v1), then the native macOS app (v1.5+).

## 5. Out-of-scope use

- **Any arithmetic, P&L, sizing, expectancy, or statistic** — the engine
  computes these; the model must never produce a number that did not come from
  an engine field (this is the groundedness contract, FR-10 / EVALS_PLAN Dim 2).
- **Autonomous order placement** — no LLM-initiated live order without explicit
  human confirmation; every order-affecting action is human-gated (FR-17 /
  NFR-7).
- **Financial advice or jurisdictional/suitability guidance** — PulseTrader is a
  personal-use tool; trading is the user's responsibility under a "not financial
  advice" framing (Phase 4.1 / close-audit CX3 / PRD §9).
- **Holding state across turns or iterating internally** — the engine and the
  event-sourced data own all state.
- **Processing of unredacted secrets, balances, or account IDs** — redaction
  strips these before dispatch (NFR-6).
- Untrusted-content execution: imported strategy text and news-feed content are
  treated as data, never instructions (FR-13 / NFR-7).

## 6. Limitations

- **Hallucinated metrics** — an LLM can invent plausible-sounding numbers.
  *Mitigation:* the no-math boundary + the groundedness eval (Dim 2, 100%
  trace-to-field, 0 fabrications). The engine is the sole source of every
  statistic; the explainer/coach only reference fields.
- **Overfitting / noise-chasing suggestions** — a coach may propose a mutation
  that fits noise rather than signal. *Mitigation:* one-mutation discipline
  (FR-8), the statistical-significance guard (FR-10 / CX1 — only above-noise
  improvements count as "better"), confidence-interval references, and the
  walk-forward / Monte Carlo robustness backstop (UC-6).
- **Prompt-injection susceptibility** — untrusted strategy text and (v2+)
  news-feed content could carry injected directives. *Mitigation:* untrusted
  content is framed as data not instructions, the LLM cannot emit raw DSL or
  place orders, and adversarial prompts are part of the eval set (FR-13 / NFR-7 /
  EVALS_PLAN §2A).
- **Latency** — LLM calls dominate turn latency (1–10s typical) against the
  NFR-1 budget; a turn is hard-capped at **120s** with graceful degradation
  ("here's what I have so far") and cancellation at safe checkpoints via
  `CancellationToken`. The engine hot paths (backtest <5s, sweep <30s) are
  measured separately and are not LLM-bound.
- **Cost variability** — per-call cost varies by backend and prompt size; bounded
  by the budget control loop (FR-25 / NFR-10): notify at 80%, route to
  cheapest/subscription-only at 100%, disable autonomous/optimizer runs at the
  hard ceiling while keeping interactive use available. Subscription backends
  (Claude Code / Codex) are $0 marginal. Tier A target ~$20–40/mo; hosting ~$0
  (local binary + Parquet + SQLite, no servers).
- **Non-determinism** — model outputs are not bit-reproducible across calls; this
  is why all *computation* lives in the deterministic engine and why LLM tests
  use recorded fixtures by default (NFR-9.4), pinning model versions for eval
  stability (NFR-12).

## 7. Risks & mitigations (summary)

| Risk | Mitigation |
|---|---|
| Hallucinated metrics | No-math boundary + groundedness eval (Dim 2) + engine-as-sole-source |
| Overfitting / noise-chasing | One-mutation rule (FR-8) + noise-band guard (FR-10) + CI references + walk-forward/MC |
| Prompt injection | Untrusted-as-data framing, no raw-DSL/no-order capability, adversarial eval prompts (FR-13/NFR-7) |
| Unauthorized order placement | Mandatory human confirmation for all order-affecting actions (FR-17/NFR-7) |
| Secret/PII leakage to a vendor | Dispatch-path + write-path redaction (NFR-6) |
| Budget blow-out (esp. v4 optimizer) | Budget control loop with hard ceiling (FR-25/NFR-10) |

## 8. Data handling

**What the LLM sees (context).** Each agent receives only its `Lens`-scoped,
**redacted** context. Before any prompt reaches any backend, the redaction layer
(NFR-6 / Phase 4 S3) strips API keys and account IDs and normalizes raw balance
numbers to **relative** values — applied on *both* the LLMCall write path and
the LLM dispatch path. The Composer sees the strategy library + DSL templates;
the Coach sees `BacktestRun` + `Trade` summaries + prior `CoachingSession`
suggestions for that version. No raw balances, no keys, no account IDs ever
enter a prompt.

**What is logged.** Every model interaction is recorded as an **LLMCall** event
(Phase 3 invariant #13 / FR-24): backend, model version, prompt + completion
tokens, cost, **verbatim prompt + completion**, redaction flags, and timestamp.
Verbatim capture is intentional (debugging + eval reproducibility) and is safe
*because* redaction runs before the prompt is built — the stored verbatim text
already contains no secrets. Optional gzipped JSONL LLMCall archives on the
filesystem follow a configurable retention. All of this is local (SQLite +
filesystem under `~/Library/Application Support/PulseTrader/`); **no telemetry,
no cloud crash reporting** (NFR-11).

**Sensitive data classification (Phase 4.1).** The user's own financial data:
Binance Futures API keys (highest blast radius — Keychain only, never in a
prompt or plaintext file), account balance, trade history, P&L, position state,
funding payments. LLM API keys (lower blast radius, Keychain). No PII, no health
data, no payment-card data (account funding is out-of-band via Binance's own
deposit flow).

**Embeddings (conditional).** PulseTrader uses **no embeddings in v1**. If
cross-session coach memory ships (v2+), it would use a local **PulseDB MiniLM**
embedding model over redacted, locally-stored coaching history — same redaction
and same no-egress posture. Until that ships, this row is N/A.

## 9. Eval reference

- See [EVALS_PLAN](./EVALS_PLAN.md) for the eval set, the six dimensions,
  acceptance thresholds, CI gating, and the backend quality-per-dollar
  comparison.
- See [PROMPT_GOVERNANCE](../../pulse-trader-ai/docs/PROMPT_GOVERNANCE.md) for
  the system prompts, Lens scoping, redaction policy, and prompt-injection
  defences.
