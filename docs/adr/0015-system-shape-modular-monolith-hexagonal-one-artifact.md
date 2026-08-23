# 15. System shape: modular monolith, hexagonal, one shippable artifact, zero sidecars

Date: 2026-08-23T00:00:00Z

## Status

Accepted

(Accepted at adoption. This decision was made and exercised under the scaffold-dev
stack across sprints 1.1-1.3; `/ossify:adopt` recorded it as a bone on 2026-08-23
against baseline `49f229a`. Per ossify's bones protocol, decisions the adopted
baseline already exercises are minted `Accepted`, not `Proposed` — the baseline
*is* the release that exercised them. Retrospective record: it documents a
standing decision, it does not introduce one.)

## Context

PulseTrader must ship as an installable macOS app while running an LLM agent loop, a
deterministic backtest engine, and market-data ingestion. The obvious alternative was a
Rust core plus a Python agent sidecar, which the PulseHive decision eliminated.

## Decision

A **modular monolith** with **hexagonal (ports-and-adapters)** discipline: ports are
traits in the domain layer, adapters implement them, and dependency direction is
always inward.

**The domain invariant is "zero I/O", not "zero dependencies"** — `src/domain/mod.rs`
states that policy explicitly. Domain types freely use `serde`, `rust_decimal`,
`chrono` and `thiserror`; `rust_decimal::Decimal` appears directly in port
signatures. What the domain must not do is perform I/O or reach an external system.
Recording the stronger dependency-free claim would make compliant changes look like
architecture violations. **One shippable artifact**, **zero sidecar
processes**.

**Agent orchestration is first-party and in-process; PulseHive is the transport.**
`src/agent/composer.rs` is the orchestrator — it drives the model through the builder
tools and finalizes the `StrategyVersion` itself. PulseHive is reached only through
the thin `LlmProvider` port (ADR-0012, ADR-0013); `src/adapters/llm/openai_compat.rs`
states the boundary explicitly: *"Thin transport ONLY: no `HiveMind`/agent/lens
substrate."* The zero-sidecar property comes from orchestrating in-process at all,
not from adopting PulseHive's agent substrate — which this baseline deliberately does
not use.

## Relationship to the ADR-0001 decision queue

`0001-record-architecture-decisions.md` carries a decision index listing
**ADR-0002** (hexagonal), **ADR-0003** (single Tauri shell, zero sidecars) and
**ADR-0004** (PulseHive as the in-Rust agent framework) as *Proposed* placeholders
never written up. This ADR is where two of those land, and leaving the index
unreconciled would give the same architecture two live statuses:

- **ADR-0002 (hexagonal)** — written up here. Superseded.
- **ADR-0003 (zero sidecars)** — the zero-sidecar half is written up here and
  superseded. The **Tauri desktop shell** half is *not*: it is unbuilt at this
  baseline. Its Proposed placeholder is **ADR-0003** in ADR-0001's queue, and it needs
  its own dedicated ADR when a slice builds it — **not** ADR-0019, which scopes itself
  to the exercised stack and expressly refuses ownership of the shell.
- **ADR-0004 (PulseHive as the agent framework)** — superseded by **ADR-0012** and
  **ADR-0013**, which chose the opposite of what the queue entry proposed: a thin
  PulseTrader-owned port over PulseHive rather than its agent substrate. The queue
  entry itself even records that alternative ("A2") as the one to revisit; it won.

The index in ADR-0001 is annotated accordingly.

## Consequences

Crate splits stay mechanical because ports live in the domain. No IPC, no process
supervision, no version skew between components. The cost is that everything shares one
address space and one release cadence: a component that genuinely needs independent
deployment would force a revisit, which is this bone's trigger.
