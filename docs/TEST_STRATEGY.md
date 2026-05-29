# PulseTrader — Test Strategy

**Last derived from MASTER-SPEC.md @ 2026-05-28T15:12:07Z**

> Derived from Phase 9 (Quality, Testing & Eval). The governing principle: **the deterministic Rust core is where correctness is proven, so the test strategy is heaviest there and lightest at the LLM/UI edges.** Two non-negotiables anchor everything below — the backtester determinism test is *sacred*, and the money-math is *never mocked*.

---

## 1. Test pyramid & tiered coverage floors (NFR-8)

Coverage is tiered by criticality, not uniform. CI enforces three hard floors; below any of them, the PR fails the coverage gate.

| Layer | Floor | Rationale | How tested |
|---|---|---|---|
| **money-math** (position-sizing, P&L, R-multiple, MFE/MAE, funding, intra-bar collision, DSL compilation) | **100%** | Real money depends on it; any gap is a capital-risk gap. **Never mocked.** | Unit (hand-computed) + property tests, real computation |
| **`mod domain`** (pure types, ports, logic; zero I/O) | **90%** | The deterministic core; the contract everything else depends on | Unit + property + cross-validation |
| **workspace** (whole repo) | **80%** | Baseline floor across all crates | Aggregate |
| `mod adapters` (SQLite repos, Binance data/broker, Tokio clock) | ~70% (advisory) | Integration-shaped; real DB + fixtures | Integration |
| `mod agent` (PulseHive tools, agent loop, lens scopes) | ~60% (advisory) | Tools fully tested; loop via recorded LLM | Tool unit + recorded-LLM integration |
| `ui` (TypeScript/React) | lighter (verified by use) | Edge layer; verified through E2E + manual | tsc + eslint + E2E |

The shape is a wide base of fast unit + property tests over the domain, a thinner band of integration tests against real SQLite + a small real Parquet fixture, a slim contract layer, and a single automated E2E (the 10-minute round trip). **Pyramid runtime target: the fast suite runs on every commit; E2E stays under ~30s with a mocked LLM.**

---

## 2. The two anchors

### 2.1 Sacred: backtester determinism (NFR-2, invariant #10)
The same backtest, run **100×**, single-threaded **and** Rayon-parallel, on **both** aarch64 and x86_64 apple-darwin, must produce a **byte-identical result hash**. This runs on **every PR** and is non-negotiable. It is enforceable because FMA/fast-math is disabled in the backtester, Rayon parallelizes only *across* sweep combos (never within a single backtest, so no parallel FP reductions), and the target architecture is folded into `engine_fingerprint`. Any intentional change to backtest output surfaces as a reviewed **golden-file diff** (invariant: golden-file backtest results).

### 2.2 No mocking the money-math (NFR-8, invariant: §9.4)
Position-sizing, P&L, R-multiple, MFE/MAE, funding application, intra-bar collision, and DSL compilation are tested with **real computation** against hand-verifiable expected values. Mocks are permitted *only* for external I/O (exchange, LLM, clock). A money-math test that mocks the calculation under test is rejected in review.

---

## 3. The five test types — concrete for PulseTrader

### 3.1 Unit (hand-verifiable synthetic fixtures)
Hand-crafted synthetic candle series with **hand-computed expected values**. Examples:
- A 5-candle synthetic series where the RSI(14) value and the resulting entry signal are computed by hand; assert the engine matches.
- A single long trade on a hand-built series: entry 100, stop 95, 2% account risk on $1000 → assert position size, R-multiple, realized P&L after a hand-computed fee + funding charge.
- Liquidation-price computation for a known leverage + margin against a hand-derived expected price (FR-5, RISK-14).
- DSL compilation of a minimal `create_strategy → … → finalize_strategy` sequence into the expected compiled form (FR-3).

### 3.2 Property-based (proptest)
Invariants that must hold across generated inputs:
- **Sizing identity:** `position_size × stop_distance == risk_amount` (within float tolerance) for any valid risk/stop (invariant #3, NFR-3).
- **MFE/MAE signs:** `mfe_r ≥ 0` and `mae_r ≤ 0` for every trade, always (invariant #4, FR-6).
- **No-entry → zero-trades:** any strategy whose entry condition never fires produces exactly zero trades and a flat equity curve.
- **DSL serde round-trip:** `deserialize(serialize(dsl)) == dsl` for any schema-valid DSL (tagged-enum representation, FR-4).
- **100×-identical backtest:** the determinism anchor expressed as a property over seeds/orderings (§2.1, NFR-2).
- **Sim/live byte-equality:** `pulse-broker` produces byte-equal position sizes in sim and live for any input (invariant #3, NFR-3).

### 3.3 Integration (real SQLite + real Parquet fixture)
- Full pipeline on a **1-month BTCUSDT fixture** (real Binance data, M15+H4): ingest → compile → backtest → persist BacktestRun → journal Trades → assert summary stats + trade count.
- **sqlx migration up/down**: apply all migrations to a fresh DB, assert schema; roll down where supported; verify the DB-before-migration backup protocol writes `pulse.db.bak-<version>-<timestamp>` and that a failed migration restores the backup and refuses to start (NFR-12, invariant: §7.4 migration protocol).
- Real SQLite (WAL) repository round-trips for each entity; append-only Trade + TradeCorrection projection correctness (invariant #9, FR-21).

### 3.4 Contract
- **DSL versioning:** a minor `schema_version` bump auto-migrates an old DSL at read; a major bump requires an explicit migration function; `dsl_original` preserved (FR-4, invariant #7).
- **Tauri ↔ TS IPC round-trip:** for each shared type, Rust serializes → TS deserializes → assert shape, so tauri-specta codegen drift surfaces as a **test failure** (Phase 7.4, IPC round-trip gate).
- **Recorded Binance response shapes:** contract tests against recorded REST/WS response fixtures so a Binance API/symbol-rule change breaks CI rather than production (RISK-15, FR-5/FR-19).
- **AgentEvent stream shape:** the formally-typed PulseHive `AgentEvent` variants relayed to the UI are asserted stable.

### 3.5 End-to-end — the 10-minute round trip (automated)
The single E2E test scripts the v1 success proof with a **mocked LLM**: compose (NL → builder tool calls → finalized StrategyVersion) → backtest → coach (one mutation) → accept → re-backtest → assert the child version exists with a comparable BacktestRun and a recorded CoachingSession (UC-2→UC-4, FR-3/FR-8/FR-9). Runs deterministically and fast (<30s) because the LLM is mocked; the live-LLM variant is gated/nightly (§5).

### 3.6 Cross-validation (indicators vs pandas-ta)
Each indicator wrapped behind the `Indicator` port (ta-rs in v1) is cross-validated against **pandas-ta within an epsilon** on shared fixtures, so a divergent indicator implementation is caught (Phase 7.4 test-data strategy layer 2).

---

## 4. Fidelity & reconciliation tests

- **Paper-vs-live reconciliation (v3):** once live exists, assert the same engine produces matching behavior across paper and live on identical inputs (invariant: §9.4; NFR-3). Underpins the calibration loop (FR-16).
- **Calibration-loop test (v2+):** a known strategy run through backtest AND paper; assert the measured gap is recorded and fed back as a slippage-model correction (FR-16, close-audit C-FIDELITY).
- **Four-timestamp / latency discipline:** assert backtest Trades have `signal_time == fill_time` (`latency_ms == 0`) and paper/live Trades carry non-null fill timestamps with computed `latency_ms` (invariant #11, FR-15).

---

## 5. LLM testing & eval gate (Phase 9.3)

- **Recorded fixtures by default.** LLM-dependent tests use recorded prompt/completion fixtures; **live-LLM tests are gated and run nightly**, not on every PR (they cost tokens). The eval gate runs in CI **only for prompt/tool changes**.
- **Eval fixture set is an explicit early v1 deliverable** (~20 prompts + ~10 backtest fixtures) — it gates CI, so it must exist before the eval gate is switched on (close-audit CL4, RISK-9).
- **Six eval dimensions** against the fixed set: (1) format/structural validity (schema-valid DSL, well-formed tool calls; target 100%) — auto-asserted; (2) groundedness (coach claims trace to real result fields, references CIs) — auto-asserted (FR-8); (3) one-mutation discipline (binary; invariant #8, FR-8) — auto-asserted; (4) cost (tokens + $ per turn/round-trip, per backend, budget alerts) — auto-asserted (FR-24/FR-25); (5) latency (per turn, 120s cap) — auto-asserted (NFR-1); (6) actionability — human-rated on a sample.
- **Backend-comparison** is built into the harness (same eval set across GLM 5.1 / DeepSeek / Claude Code) for quality-per-dollar (FR-23).

---

## 6. Pre-merge gates (Phase 9.2)

A PR merges only when **ALL** pass:
1. `fmt` (rustfmt)
2. `clippy -D warnings` (all + pedantic)
3. `nextest` (full unit + property + integration suite)
4. **Coverage gate:** `mod domain` ≥ 90% + money-math = 100% + workspace ≥ 80% (NFR-8)
5. **Determinism test** — 100×-identical, single + parallel, on **both** aarch64 and x86_64 (NFR-2)
6. **Money-math property tests** (sizing identity, MFE/MAE signs, sim/live byte-equality)
7. `cargo audit` + `cargo deny` (advisories + licenses; NFR-12)
8. Frontend `tsc` + `eslint` — *UI PRs*
9. **IPC round-trip test** — *type-changing PRs* (catches tauri-specta drift)
10. **LLM eval gate** — *prompt/tool PRs* (gated; costs tokens)
11. scaffold-dev `auto:` acceptance criteria for the slice

---

## 7. Test data strategy (layered — Phase 7.4)

1. **Hand-crafted synthetic series** with hand-computed expected values (unit layer).
2. **Cross-validation vs pandas-ta** within an epsilon (indicator correctness).
3. **Property tests** for the invariants in §3.2.
4. **Small real Binance fixture** — 1-month BTCUSDT (M15+H4) for integration.
5. **Recorded LLM fixtures** + the ~20-prompt / ~10-backtest eval set (§5).
6. **Recorded Binance REST/WS responses** for contract tests (§3.4).

---

## 8. Frameworks & tooling

Two languages (Python eliminated by PulseHive):
- **Rust core/agent/Tauri backend:** `cargo-nextest` (runner), `proptest` (property tests), `cargo-audit` + `cargo-deny` (supply chain), `sqlx` compile-time-checked queries (schema validated at build). Test commands live in `just` / `cargo`.
- **TypeScript UI (React + Vite):** `tsc --strict` (no `any`) + `eslint`; IPC types generated via tauri-specta and round-trip-tested.
- Commands are invoked through the build tool (`cargo nextest`, `just test`) and by scaffold-dev's slice-verify; CI is GitHub Actions on an aarch64 + x86_64 apple-darwin matrix.

---

## 9. What we deliberately don't test

- **Generated code** — tauri-specta IPC bindings are validated by the round-trip *contract* test (§3.4), not by re-testing the generator itself.
- **Third-party dependencies** — `ta-rs`, `sqlx`, Polars, PulseHive internals are trusted (version-pinned + audited); we test our *use* of them (cross-validation, integration), not their internals.
- **The LLM provider's internals** — we test our prompts/tools and our parsing of responses against recorded fixtures, not GLM/Claude/Codex model quality beyond the eval dimensions.
- **Binance's API correctness** — we contract-test against *recorded* response shapes; we do not test Binance itself.
- **`ui` exhaustively** — the UI is verified by use + E2E + tsc/eslint, not by high-coverage unit tests (lighter floor by design).

## See also

- [MASTER-SPEC](../MASTER-SPEC.md) §9 (Quality, Testing & Eval), §7.4 (test-data strategy), §9.4 (testing invariants)
- [SRS](./SRS.md) — NFR-2, NFR-3, NFR-8; FR-3, FR-4, FR-5, FR-6, FR-7, FR-8, FR-9, FR-15, FR-16
- [RISK_REGISTER](./RISK_REGISTER.md) — RISK-1, RISK-9, RISK-13, RISK-14, RISK-16
