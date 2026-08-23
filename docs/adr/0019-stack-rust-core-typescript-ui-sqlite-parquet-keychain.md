# 19. Stack: Rust core + agent + Tauri backend, TypeScript/React UI, SQLite WAL + Parquet + Keychain

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

Two languages, chosen so the deterministic engine and the agent orchestration share one
runtime. Python was eliminated when PulseHive made in-Rust agent orchestration viable.

## Decision

**Rust** for the core engine, agent orchestration and Tauri backend; **TypeScript
(React + Vite)** for the UI in Tauri's WebView, glued by the Tauri command bus with
types generated via tauri-specta. Storage is four tiers: **SQLite** (WAL, single file)
for entities and event logs with sqlx migrations; **Parquet** for immutable
`CandleSeries`; the **macOS Keychain** for secrets; the **filesystem** for logs and
exports. No Postgres, no Redis, no separate vector database.

## Consequences

One toolchain for all deterministic work, and no cross-language serialization boundary
in the hot path. Embedded storage keeps the app installable with no service to run,
at the cost of ruling out multi-machine deployment without revisiting this bone. A
second backend language or any server-side component is the revisit trigger.
