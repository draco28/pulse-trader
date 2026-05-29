# PulseTrader — Cutover Plan

**Last derived from MASTER-SPEC.md @ 2026-05-28T15:12:07Z**

> PulseTrader is a **solo, local-first desktop app with no server**, so "cutover" is not a fleet deployment — there is no traffic to drain, no load balancer to flip, no blue/green stack. The two cutovers that actually carry risk here are (A) **graduating a strategy from paper to live capital** and (B) **transitioning the user's data from the v1 CLI to the v1.5 native app**. Both are documented below as concrete, numbered scripts with rollback paths.

## Environments

dev-only + release channels (no server). Channels: dev (local), nightly (personal, un-notarized), stable (tagged, notarized). Two build profiles (personal / distributable) orthogonal to channel. The **trading environments (backtest / paper / live) are runtime modes via the Deployment state machine, not deploys** — graduating between them is the real cutover, scripted in §Cutover A.

## Hosting

Self-hosted = the user's Mac. No cloud v1–v3. App + SQLite + Parquet + Keychain + logs live under `~/Library/Application Support/PulseTrader/` (DB), `~/Library/Logs/PulseTrader/` (logs), and macOS Keychain (secrets). Distribution hosting = GitHub Releases for the `.dmg` + Tauri update manifest. Auto-update: signed Ed25519 Tauri updater from v1.5; CLI v1 updates via `git pull`. v4+ aspirational: a shared-strategy-library API would need real hosting (out of scope).

## Rollout strategy

Staged release channels (dev → nightly → stable; dogfood nightly first) + feature flags for blast-radius-sensitive surfaces (**live-trading behind a flag, default OFF**; new LLM backends behind flags; local config, no remote flag service). The graduation gates **are** the per-strategy rollout (backtest → paper → live, manual gates). Live canary = capped capital ($10–20, a first-class deployment setting).

---

## Cutover A — Paper → Live graduation for a strategy

The highest-stakes cutover in the product: moving a strategy from simulated paper execution to real capital. v3 capability (BACKLOG-23); the script below is the operating procedure once live execution ships. It is **manual by design** — no LLM-initiated live order, ever (FR-17, NFR-7).

### Pre-cutover (gates — all must pass before flipping)

1. **Fidelity advisory passed.** Open the backtest-vs-paper comparison (FR-16). Confirm the advisory Bayesian P(paper_expectancy ≥ backtest_expectancy × tolerance) is acceptable and that the paper period is long enough to be meaningful, not within-noise. *Expected outcome:* advisory probability and trade count shown; you have explicitly judged the gap acceptable. *Abort if:* paper expectancy is materially below backtest with no calibration explanation (re-run the calibration loop, BACKLOG-18, first).
2. **Capped capital set.** Set the per-deployment capital cap to the first-live figure ($10–20). *Expected outcome:* the Deployment carries a non-null capped-capital setting; the worst-case unattended loss is bounded to that figure (NFR-4).
3. **No-withdraw key verified.** Re-confirm the active Binance Futures key has trading enabled and withdrawal disabled (FR-2, S4). *Expected outcome:* the executor refuses to start if withdrawal scope is present; with a no-withdraw key it proceeds.
4. **Kill-switch tested.** Fire `pulse kill-all` (and the SIGTERM path) against the paper deployment and confirm it disables the deployment and would close positions (FR-20, NFR-4). *Expected outcome:* the deployment moves to a disabled/`killed` state and a critical native notification fires. **Do not proceed if the kill switch did not visibly work.**
5. **Live flag review.** Confirm the live-trading feature flag is intentionally being turned ON for this deployment (default OFF). *Expected outcome:* a deliberate, logged flag flip — not an accident.

### During cutover (the flip)

1. **Advance the state machine** `paper_complete → live_pending → live_active` via the manual graduation gate; a GraduationEvent is written to the append-only log (FR-14). *Expected outcome:* the Deployment is `live_active`; one GraduationEvent recorded.
2. **Confirm the first live order manually.** When the first signal fires, the system blocks on explicit human confirmation before placing the order; the order is written to the local DB *before* the exchange call (FR-17, FR-18, S2). *Expected outcome:* a confirmation prompt; on approval, a pre-execution DB record exists and the order is sent. *If the order is rejected:* a critical notification fires; reconcile against the pre-execution record on the next tick.
3. **Watch the supervised feed.** Confirm the WebSocket actor is connected and the heartbeat watchdog is green (FR-19). *Expected outcome:* feed health surfaced as connected; a forced drop would trigger backoff reconnect + REST gap-fill, and a prolonged outage would auto-pause all deployments with a broker-feed-down GraduationEvent.

### Post-cutover (monitoring + sign-off)

1. **Monitor the first session.** Watch the first few live trades against the journal (source=live) with the four timestamps and computed `latency_ms` (FR-15/FR-21). *Expected outcome:* live Trades are journaled; signal-to-fill latency looks sane (NFR-1: signal-fire → order sent < 100ms).
2. **Reconcile vs paper.** Compare early live results to the paper baseline and to the calibrated fidelity model. *Expected outcome:* live behavior tracks paper within the documented tolerance; the paper-vs-live reconciliation test (BACKLOG-24, NFR-3) confirms the engine is identical across modes.
3. **Sign-off criteria.** Graduation is considered successful when, over the agreed initial window: no order placed without confirmation, no kill-switch/auto-pause anomalies, P&L within bounded expectations, and zero unreconciled pre-execution records. Only then consider raising the capital cap.

### Rollback ("we will roll back if X")

Roll back **if** live expectancy diverges materially from paper, any order executes without confirmation, the feed becomes unreliable, or max-drawdown is breached.
1. **Pause the deployment** (`live_active → paused`) — or fire `pulse kill-all` if positions must close immediately. *Expected outcome:* no new orders; positions closed on kill-all (FR-20).
2. **Close open positions** and confirm flat via the journal. *Expected outcome:* zero open positions.
3. **Disable the live flag** for this deployment and return it to paper for further calibration. *Expected outcome:* deployment off live; data preserved (Trade rows are immutable — nothing is lost on rollback).
4. **Root-cause** via the local logs (`~/Library/Logs/PulseTrader/`) and the LLMCall/Trade records before any re-attempt.

---

## Cutover B — v1 CLI → v1.5 native-app transition

The v1 CLI and the v1.5 native app **share the same Rust core, the same DSL, and the same SQLite DB**. The transition is therefore primarily a data-continuity and parity exercise, not a rewrite cutover (BACKLOG-15). The CLI remains the fallback.

### Pre-cutover

1. **Back up state.** Copy `~/Library/Application Support/PulseTrader/pulse.db` and the Parquet `CandleSeries` directory. *Expected outcome:* a restorable snapshot exists (Time Machine also covers the single-directory app state). *Verify rollback path is tested:* confirm the backup restores and the CLI still runs against it.
2. **Record a baseline.** Run a known strategy through the CLI backtest and note its result hash + expectancy. *Expected outcome:* a golden baseline to diff parity against.
3. **Install the v1.5 app** (signed, notarized `.dmg` from GitHub Releases) pointed at the *same* DB path. *Expected outcome:* the app launches; cold startup < 2s (NFR-1).

### During cutover

1. **Run the startup migration.** On first app launch, the migration protocol checks `schema_version`; if behind, it backs up `pulse.db` → `pulse.db.bak-<version>-<timestamp>`, migrates in a transaction, verifies, and proceeds — or restores the backup and refuses to start on failure (FR-6, NFR-12). *Expected outcome:* a timestamped backup exists and the app opens on the migrated DB; on failure the app refuses to start and the original DB is intact.
2. **Confirm data visibility.** Open the Strategy Library and verify existing strategies, version subtrees, and BacktestRuns authored via the CLI are present (FR-11). *Expected outcome:* the full version forest the CLI built is visible in the GUI.

### Post-cutover (parity verification)

1. **Parity backtest.** Re-run the baseline strategy in the app's Backtest Lab and compare the result hash + expectancy to the CLI baseline. *Expected outcome:* identical `engine_fingerprint`-keyed result (same core, same engine — FR-7, NFR-2). A divergence is a parity defect, not a "new" result.
2. **IPC sanity.** Confirm the Tauri command + AgentEvent surfaces behave (the IPC round-trip test having passed in CI for the build). *Expected outcome:* chat ↔ DSL panes stay in sync as projections over one event stream; no drift.
3. **Sign-off.** The transition is complete when the app reproduces the CLI's baseline byte-for-byte and all CLI-authored data is visible and operable in the GUI.

### Rollback

The CLI is the fallback and shares the DB, so rollback is trivial.
1. **Keep using the CLI** against the same `pulse.db` — it is unaffected by the app install. *Expected outcome:* no work lost.
2. **If a migration was applied** and the app misbehaves, restore the pre-migration `pulse.db.bak-<version>-<timestamp>` and continue on the CLI on the older schema. *Expected outcome:* prior-schema DB restored; CLI operational.
3. **Roll back if:** parity fails (app result ≠ CLI baseline) or CLI-authored data is missing in the GUI — investigate before re-attempting.

---

## Communication

PulseTrader is a **solo project**: there are no external stakeholders, no Slack/email/status-page comms, and no on-call rotation. Cutover communication is therefore self-directed:
- **CHANGELOG** entry for each release-channel promotion and each notable cutover (the deprecation/migration record).
- **Self-notes** in the trade journal / retrospective for graduation decisions (why a strategy went live, what the fidelity gap was, sign-off rationale) so the decision is auditable to future-you.
- **Native macOS notifications** are the only runtime alerting channel; there is nobody else to page.

## See also

- [MASTER-SPEC](../MASTER-SPEC.md)
- [BACKLOG](./BACKLOG.md)
- [PROJECT_PLAN](./PROJECT_PLAN.md)
