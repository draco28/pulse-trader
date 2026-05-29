# PulseTrader — Risk Register

**Last derived from MASTER-SPEC.md @ 2026-05-28T15:12:07Z**

> A live, tracked register of project risks. Seeded from the three Phase 2.2 named risks (R1–R3) and the 13-challenge close-audit hardening pass. Owner is the solo developer throughout (no other human collaborators in v1). Hand-edit freely — this is a working document.

## Conventions

- IDs `RISK-N`, never reused. Status: `open` · `mitigating` · `mitigated` · `accepted` · `closed` · `invalidated`.
- Likelihood / Impact: `L` (low) · `M` (medium) · `H` (high).
- Categories: `tech` · `market` · `architecture` · `security` · `regulatory` · `resource`.
- `Cites` links a mitigation to the requirement that operationalizes it (FR-N / NFR-N from SRS, or S1–S5 invariants from MASTER-SPEC Phase 4.4).

## Summary table

| ID | Category | Risk | L | I | Status |
|---|---|---|---|---|---|
| RISK-1 | tech | Backtester fidelity gap | H | H | mitigating |
| RISK-2 | architecture | LLM-backend abstraction wrong (premature/under) | M | H | mitigating |
| RISK-3 | market | Strategy never crosses the profitability bar | H | M | accepted |
| RISK-4 | resource | v1 timeline vs scope (CL1) | H | M | mitigating |
| RISK-5 | tech | LLM coaching degenerates into noise-chasing hill-climber (CX1) | M | H | mitigating |
| RISK-6 | regulatory | Compliance exposure once shipped beyond author (CX3) | L | H | mitigating |
| RISK-7 | architecture | "No refactor v1→v4" implausibility (CX5) | M | M | accepted |
| RISK-8 | resource | LLM budget overrun — tracking ≠ enforcing (CL3) | M | M | mitigating |
| RISK-9 | resource | Cold-start: nothing to backtest / eval gate can't run (CL4) | M | M | mitigated |
| RISK-10 | resource | Memory budget exhausted by sweeps (CL5) | L | M | mitigating |
| RISK-11 | tech | Dependency / LLM-model drift breaks reproducibility (CX4) | M | M | mitigating |
| RISK-12 | security | Order-placing app — credential theft & bad-order risk (CX6) | M | H | mitigating |
| RISK-13 | tech | Backtest-vs-live fidelity gap surfaces only after real capital | M | H | mitigating |
| RISK-14 | tech | Liquidation mechanics unmodeled → dangerously optimistic results | M | H | mitigating |
| RISK-15 | tech | Binance API version / symbol-rule change breaks pipeline | M | M | mitigating |
| RISK-16 | tech | Cross-architecture float non-determinism breaks fingerprint | L | H | mitigating |
| RISK-17 | architecture | PulseHive churn ripples into PulseTrader core (A2) | M | M | accepted |
| ~~RISK-X~~ | ~~regulatory~~ | ~~AGPL / relicensing exposure from PulseHive/PulseDB~~ | — | — | **invalidated** |

---

## Detailed register

### RISK-1 — Backtester fidelity gap *(tech)*
- **Description:** Unrealistic modeling of fees, perp funding, slippage, and intra-bar collision produces optimistic backtest results that mislead the coaching loop and the graduation decision. This is the **load-bearing premise** of the whole product (close-audit C-FIDELITY): if the backtest lies, every downstream decision (which mutation is "better," whether to graduate to paper, whether to risk live capital) is corrupted.
- **Likelihood:** H — realistic microstructure modeling is genuinely hard and easy to get subtly wrong.
- **Impact:** H — corrupts the core value proposition and can lose real (capped) capital.
- **Mitigation:** Conservative cost defaults; explicit fee/funding/slippage/intra-bar-collision modeling (FR-5); liquidation-price modeling in v1 (RISK-14); a documented list of v1-modeled-vs-deferred microstructure effects (so deferrals are flagged gaps, not silent omissions); a calibration loop that measures the backtest-vs-paper gap and feeds it back as a slippage-model correction (FR-16); a mandatory paper-trade gate before live (FR-15, UC-9); walk-forward + Monte Carlo robustness backstop (FR-12).
- **Cites:** FR-5, FR-12, FR-15, FR-16; close-audit C-FIDELITY.
- **Trigger / early warning:** Paper expectancy diverges from backtest expectancy beyond the calibration tolerance (X% TBD in v2); regime-by-regime breakdown shows costs near-zero in conditions where they shouldn't be.
- **Owner:** Solo dev. **Status:** mitigating.

### RISK-2 — LLM-backend abstraction wrong *(architecture)*
- **Description:** The AI-backend layer is either over-abstracted (complexity tax now, slows v1) or under-abstracted (forces the exact v2 refactor the architecture exists to avoid). Subscription backends (Claude Code / Codex) are stateful subprocesses, not stateless API providers — the naive `LlmProvider` shape may not fit them.
- **Likelihood:** M — the shape is reasoned but unproven until a second backend ships.
- **Impact:** H — a wrong abstraction undermines the "extensibility proven" success criterion.
- **Mitigation:** Design the agent-tool interface against an explicit ports-and-adapters contract (uniform `LlmProvider` port); validate by shipping two backends (one API + one subprocess) across the v1→v2 boundary; v1 deliberately decouples by using GLM 5.1 via the existing OpenAI-compatible provider, so subprocess complexity is a fast-follow (PulseHive work items), not a v1 blocker. Stateful-subprocess concerns solved at the PulseHive framework layer (`StatefulLlmProvider` extension trait).
- **Cites:** FR-23, NFR-9; MASTER-SPEC §5.6.
- **Trigger / early warning:** The first subprocess-backend integration requires changing domain-layer types (not just adding an adapter) — that's the abstraction failing.
- **Owner:** Solo dev. **Status:** mitigating.

### RISK-3 — Strategy never crosses the profitability bar *(market / personal)*
- **Description:** The product can be technically perfect yet only ever produce negative-expectancy strategies after realistic costs. No amount of engineering guarantees a profitable trading edge exists.
- **Likelihood:** H — most discretionary/systematic strategies do not survive realistic costs.
- **Impact:** M — disappointing, but does not invalidate the engineering deliverable.
- **Mitigation:** Reframe success: the **architecture is the product**, not any specific strategy. The 6-month success criteria explicitly include "extensibility proven" and "the lifecycle ran once end-to-end," independent of profitability. The statistical-significance guard (RISK-5) prevents false confidence in marginal strategies.
- **Cites:** PRD §3 (goals 1 & 2); FR-10.
- **Trigger / early warning:** N/A — accepted as an inherent property of trading, not a defect to fix.
- **Owner:** Solo dev. **Status:** accepted.

### RISK-4 — v1 timeline vs scope *(resource)*
- **Description:** The summed v1 scope (full DSL enum tree + compiler, ta-rs integration, Binance data pipeline, deterministic FMA-off backtester, sqlx + migration-with-backup, PulseHive + GLM wiring, 6 builder tools, coaching framework, CLI) under a 90%/100% coverage gate is realistically larger than the optimistic 6–10 week estimate at full-time-equivalent (close-audit CL1).
- **Likelihood:** H — solo developer, ambitious surface area, hard coverage gates.
- **Impact:** M — slips the timeline; does not break the product.
- **Mitigation:** De-risking decision (CL1): v1 may **hard-cut** — prove the loop with ONE hardcoded strategy template first, then build out the full DSL grammar. Treat 6–10 weeks as the optimistic hard-cut case; re-baseline if the full DSL grammar is in scope from day 1. scaffold-dev's vertical-slice decomposition keeps each work item to ~200–500 LOC.
- **Cites:** MASTER-SPEC §2.1 (CL1 caveat), PRD §7.
- **Trigger / early warning:** End of week 4 with the deterministic backtester not yet passing its 100×-identical test on one timeframe.
- **Owner:** Solo dev. **Status:** mitigating.

### RISK-5 — Coaching degenerates into a noise-chasing hill-climber *(tech)*
- **Description:** "One mutation per turn" iteration, if it accepts any point-estimate improvement, will chase statistical noise and overfit — manufacturing strategies that look better but aren't (close-audit CX1).
- **Likelihood:** M — a natural failure mode of greedy single-step optimization on noisy data.
- **Impact:** H — directly corrupts the coaching loop's value and produces false-discovery strategies.
- **Mitigation:** A statistical-significance guard — a mutation is "accepted as better" only when the expectancy improvement exceeds the noise band given the trade count (FR-10); the coach references confidence intervals, not point estimates (FR-8), reinforcing the Phase 9 groundedness eval dimension. Walk-forward train/validation split + Monte Carlo as the overfitting backstop (FR-12, UC-6). Secondary success criterion (A3) gates "is the strategy good" on out-of-sample robustness, not workflow speed.
- **Cites:** FR-8, FR-10, FR-12; close-audit CX1, A3.
- **Trigger / early warning:** Accepted mutations stop generalizing — in-sample expectancy rises while walk-forward validation expectancy falls.
- **Owner:** Solo dev. **Status:** mitigating.

### RISK-6 — Compliance exposure once shipped beyond the author *(regulatory)*
- **Description:** "Personal-use" does not erase regulatory exposure once the app generates strategy logic + growth projections + graduate-to-live advice. Distributing beyond the author could implicate financial-advice / suitability / audit-trail rules (close-audit CX3). (Note: this is the *regulatory* dimension only — the *licensing* dimension is handled; see the invalidated AGPL risk below.)
- **Likelihood:** L — v1–v3 are single-user, single-machine, author-only; the risk only materializes on distribution.
- **Impact:** H — regulatory action against a distributed financial tool is severe.
- **Mitigation:** A v1 "not financial advice" disclaimer framing; an explicit **distribution-triggers-compliance-review checkpoint** — any build shipped beyond the author requires a regulatory-framing review (jurisdiction gating, suitability language, audit trail) before release. PulseTrader does not advise on or constrain jurisdictional choice — documented as the user's responsibility.
- **Cites:** SRS §4, PRD §9; MASTER-SPEC §4.1, close-audit CX3.
- **Trigger / early warning:** Any decision to distribute a build (DMG/notarized app) to a second user.
- **Owner:** Solo dev. **Status:** mitigating.

### RISK-7 — "No refactor v1→v4" claim is implausible *(architecture)*
- **Description:** The strong claim "v1→v4 without refactor" is unrealistic — the v4 Account aggregate, the v3+ tax-lot/FIFO ledger, and v4 optimizer data requirements **will** touch domain boundaries (close-audit CX5).
- **Likelihood:** M — these features inherently reshape the domain.
- **Impact:** M — bounded reshaping, not a rewrite.
- **Mitigation:** Soften the claim to **"minimize refactor via ports + event-sourced data."** Explicitly accept that the named features will touch domain boundaries; ports minimize but do not eliminate that reshaping. Event-sourced trade data (append-only Trade + TradeCorrection) keeps history reshape-safe.
- **Cites:** NFR-9; FR-26 (Account aggregate); close-audit CX5; MASTER-SPEC §1.4.
- **Trigger / early warning:** N/A — accepted, with the boundary-touching features pre-identified.
- **Owner:** Solo dev. **Status:** accepted.

### RISK-8 — LLM budget overrun *(resource)*
- **Description:** Tracking spend is not the same as enforcing it. A single runaway autonomous/optimizer run (v4) could blow a month's ~$20–40 Tier A budget (close-audit CL3).
- **Likelihood:** M — autonomous loops are the obvious overrun vector once v4 ships.
- **Impact:** M — financial overrun, capped at a month's surprise.
- **Mitigation:** A budget **control loop** (not just tracking): monthly budget setting; 80% → notify; 100% → auto-route new LLM calls to the cheapest backend / subscription-only mode; hard ceiling → disable autonomous/optimizer runs while keeping interactive use available (FR-25, NFR-10). Coach is opt-in with a per-call cost estimate; aggressive subscription/systematic routing.
- **Cites:** FR-24, FR-25, NFR-10; close-audit CL3.
- **Trigger / early warning:** Crossing the 80% monthly notification threshold; an optimizer run's projected token cost exceeding remaining budget.
- **Owner:** Solo dev. **Status:** mitigating.

### RISK-9 — Cold-start gap *(resource)*
- **Description:** (a) A first-run user has nothing to backtest and no example to learn from. (b) The LLM eval gate cannot run in CI until the eval fixture set exists — but it's specced as a gate, creating a chicken-and-egg ordering hazard (close-audit CL4).
- **Likelihood:** M — a real ordering trap if not scheduled deliberately.
- **Impact:** M — blocks first-use value and blocks the CI eval gate.
- **Mitigation:** (a) Seed 3–5 starter strategies so a first-run user can backtest immediately and learn by example. (b) Name the LLM eval fixture set (~20 prompts + ~10 backtest fixtures) as an **explicit early v1 deliverable**, sequenced before the eval gate is switched on.
- **Cites:** FR-3, PRD §7 (CL4); MASTER-SPEC close-audit CL4.
- **Trigger / early warning:** N/A — addressed by sequencing; deliverables are scheduled early in v1.
- **Owner:** Solo dev. **Status:** mitigated (resolution baked into the v1 plan).

### RISK-10 — Memory budget exhausted by sweeps *(resource)*
- **Description:** A parameter sweep that loads all shared candle data into RAM can exhaust memory on a base Mac (16GB), especially for multi-pair v2 sweeps (close-audit CL5).
- **Likelihood:** L — only bites at v2 multi-pair sweep scale.
- **Impact:** M — OOM/crash during a sweep; no data loss (append-only DB), but lost run.
- **Mitigation:** State the 16GB+ memory assumption; cap sweep concurrency by available RAM; stream candle data from Parquet rather than load-all when a sweep's shared data exceeds budget. Rayon parallelizes across combos but the engine reads candles lazily.
- **Cites:** NFR-1 (sweep <30s for 24 combos), PRD §10; close-audit CL5; MASTER-SPEC §5.3.
- **Trigger / early warning:** A sweep's projected shared-candle footprint exceeds a fraction of available RAM at sweep-plan time.
- **Owner:** Solo dev. **Status:** mitigating.

### RISK-11 — Dependency / LLM-model drift breaks reproducibility *(tech)*
- **Description:** Unpinned dependencies or a silently-updated LLM model break the reproducibility this spec is built on — particularly GLM 5.1 model-version drift, which changes eval results and breaks eval reproducibility (close-audit CX4). "Pin later" is insufficient for a reproducibility-obsessed system.
- **Likelihood:** M — providers update models; transitive deps drift.
- **Impact:** M — invalidates `engine_fingerprint` reproducibility and eval baselines.
- **Mitigation:** Pin major dependency versions; commit all lockfiles; pin the GLM 5.1 model version explicitly; license + advisory gating via **cargo-deny** (licenses + advisories, extending cargo-audit); a documented upgrade cadence. The `engine_fingerprint` folds crate versions + toolchain + DSL schema + target arch into every BacktestRun.
- **Cites:** NFR-2, NFR-12, FR-7; close-audit CX4; MASTER-SPEC §8.4.
- **Trigger / early warning:** cargo-audit/cargo-deny flags a new advisory; an eval baseline shifts without a code change (model drift signature).
- **Owner:** Solo dev. **Status:** mitigating.

### RISK-12 — Order-placing-app security surface *(security)*
- **Description:** This app holds Binance Futures keys and can place real orders. Threats: API-key theft, prompt-injection via imported strategy text / news-feed content, supply-chain compromise, and malicious/erroneous LLM output causing bad orders (close-audit CX6). See THREAT_MODEL.md for the full STRIDE treatment.
- **Likelihood:** M — local malware, dependency compromise, and prompt injection are realistic for an internet-egress app handling money.
- **Impact:** H — worst case is loss of capital and key compromise.
- **Mitigation:** **Mandatory human confirmation for all order-affecting actions** — no LLM-initiated live order without explicit user approval (FR-17, S-invariants); keys in macOS Keychain, no-withdraw scope enforced (FR-1, FR-2, S4), IP whitelist mandated (S5); LLM context redaction before dispatch (NFR-6, S3); treat imported strategy text + news-feed content as untrusted, prompt-injection-aware input (FR-13, NFR-7); supply-chain control via cargo-deny + cargo-audit (NFR-12); least-privilege subprocess isolation for Claude Code / Codex (personal-build only); the LLM-does-no-math boundary keeps order sizing in deterministic Rust.
- **Cites:** FR-1, FR-2, FR-13, FR-17, NFR-5, NFR-6, NFR-7, NFR-12; S1–S5; close-audit CX6.
- **Trigger / early warning:** A redaction-flag audit shows a secret leaked into an LLMCall; an unexpected order appears without a corresponding human-confirmation record.
- **Owner:** Solo dev. **Status:** mitigating.

### RISK-13 — Backtest-vs-live fidelity gap surfaces only on real capital *(tech)*
- **Description:** Distinct from RISK-1's modeling concern: even a well-modeled backtester can diverge from live behavior (real fills, real latency, real slippage) in ways that only appear once capital is at risk. The four-timestamp discipline exists to measure this.
- **Likelihood:** M — live always differs from sim at the margins.
- **Impact:** H — divergence under real capital can lose money beyond expectations.
- **Mitigation:** Same engine across backtest/paper/live on the same Binance venue (NFR-3); `pulse-broker` shared position-sizing math, property-tested byte-equal sim/live (NFR-3, invariant #3); mandatory paper-trade gate (FR-15); four-timestamp discipline captures signal-to-fill latency (`latency_ms`, invariant #11); calibration loop corrects the slippage model from the measured gap (FR-16); paper-vs-live reconciliation test (v3); capped live capital ($10–20) bounds the blast radius (FR-17, NFR-4).
- **Cites:** FR-15, FR-16, FR-17, NFR-3, NFR-4; invariants #3, #11; close-audit C-FIDELITY.
- **Trigger / early warning:** Live `latency_ms` or realized slippage materially exceeds the paper-period distribution.
- **Owner:** Solo dev. **Status:** mitigating.

### RISK-14 — Liquidation mechanics unmodeled *(tech)*
- **Description:** A leveraged futures strategy that ignores liquidation is dangerously optimistic — a backtest can show survival through a drawdown that would have liquidated the position in reality (close-audit C-FIDELITY item 4).
- **Likelihood:** M — easy to omit; common backtester defect.
- **Impact:** H — masks ruin risk; the strategy graduates to live and gets liquidated.
- **Mitigation:** **Liquidation-price modeling specifically included in v1's backtester** (FR-5). The deferred-microstructure list (margin mode, maintenance-margin tiers, ADL) flags what is *not* yet modeled as a known fidelity gap rather than a silent omission.
- **Cites:** FR-5; close-audit C-FIDELITY; UC-3.
- **Trigger / early warning:** A backtested strategy shows max drawdown approaching the leverage-implied liquidation threshold without a liquidation event recorded.
- **Owner:** Solo dev. **Status:** mitigating.

### RISK-15 — Binance API version / symbol-rule change *(tech)*
- **Description:** Binance API version changes, endpoint deprecations, or symbol rule changes (tick size, lot size, leverage tiers) break the data pipeline or the executor — the primary external dependency risk (MASTER-SPEC §10.3).
- **Likelihood:** M — exchanges change APIs and listing rules regularly.
- **Impact:** M — breaks data ingest or execution until patched.
- **Mitigation:** Pin the Binance API version; contract tests against recorded Binance response shapes surface breakage in CI (FR-7-adjacent contract tests, TEST_STRATEGY contract tier); a runbook covers deprecated endpoints and reconnect/gap-fill; the `MarketDataSource` / `ExchangeAdapter` ports keep a multi-venue future open if Binance becomes untenable.
- **Cites:** FR-5, FR-19; MASTER-SPEC §10.3, §5.2; TEST_STRATEGY contract tier.
- **Trigger / early warning:** A contract test fails against a freshly recorded Binance response; a symbol's rule fields change shape.
- **Owner:** Solo dev. **Status:** mitigating.

### RISK-16 — Cross-architecture float non-determinism *(tech)*
- **Description:** Floating-point math can differ between aarch64 and x86_64 (FMA, reordering), breaking the byte-identical-result guarantee and the `engine_fingerprint` contract that the whole reproducibility story depends on.
- **Likelihood:** L — well-understood and directly mitigated, but high-consequence if it slips.
- **Impact:** H — silently invalidates determinism and golden-file comparisons.
- **Mitigation:** FMA/fast-math **disabled** in the backtester; target architecture folded into `engine_fingerprint`; Rayon parallelizes only across sweep combos, never within a single backtest (no parallel FP reductions); a CI test asserts the same backtest produces an identical result hash 100× (single + parallel) on **both** aarch64 and x86_64 — the "sacred" determinism test on every PR (NFR-2).
- **Cites:** FR-7, NFR-2, NFR-9; invariant #10; MASTER-SPEC §7.4, §9.4.
- **Trigger / early warning:** The 100×-identical determinism test fails on one architecture but passes on the other.
- **Owner:** Solo dev. **Status:** mitigating.

### RISK-17 — PulseHive churn ripples into PulseTrader core *(architecture)*
- **Description:** PulseHive is the author's own evolving SDK; leaning on its types directly couples two moving targets, so PulseHive churn could ripple into PulseTrader's core (close-audit alternative A2).
- **Likelihood:** M — both projects evolve in parallel under the same author.
- **Impact:** M — rework, not a rewrite.
- **Mitigation:** The hexagonal architecture already leans toward isolating PulseHive behind an agent port. **Make it explicit** — adapt PulseHive behind PulseTrader's OWN thin agent port if churn becomes disruptive, so PulseHive becomes swappable and its churn doesn't reach the domain. Recorded as alternative A2; revisit if pain emerges.
- **Cites:** NFR-9; MASTER-SPEC §5.6, close-audit A2.
- **Trigger / early warning:** A PulseHive version bump forces changes outside the `mod agent` adapter layer.
- **Owner:** Solo dev. **Status:** accepted (watch; promote to mitigating if triggered).

---

## Invalidated risks (retained for audit trail)

### ~~RISK-X — AGPL / relicensing exposure~~ *(regulatory)* — **INVALIDATED**
- **Original concern:** A copyleft (AGPL) obligation from the agent framework or DB could block distribution / relicensing of PulseTrader.
- **Why invalidated:** The author **owns PulseHive and PulseDB**. Distribution and relicensing are unblocked; there is no third-party copyleft obligation. (MASTER-SPEC §5.5: "AGPL concern invalidated — author owns PulseHive + PulseDB.") The *regulatory* dimension of distribution is tracked separately as RISK-6; this entry covers only the licensing dimension, which is closed.
- **Status:** invalidated — retained so the audit trail shows it was considered and resolved, not overlooked.
