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
traits in the domain layer with no external dependencies; adapters implement them;
dependency direction is always inward. **One shippable artifact**, **zero sidecar
processes** — agent orchestration runs in-process via PulseHive.

## Consequences

Crate splits stay mechanical because ports live in the domain. No IPC, no process
supervision, no version skew between components. The cost is that everything shares one
address space and one release cadence: a component that genuinely needs independent
deployment would force a revisit, which is this bone's trigger.
