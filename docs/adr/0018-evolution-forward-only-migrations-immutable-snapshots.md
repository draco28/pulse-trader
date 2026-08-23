# 18. Evolution: forward-only migrations on the startup path, immutable content-addressed snapshots, immutable StrategyVersion

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

Backtest reproducibility is the product's core claim: a result must be re-derivable
from the same inputs. That is incompatible with mutable history.

## Decision

Schema evolves by **forward-only sqlx migrations on the normal startup path**. Candle
snapshots are **immutable and content-addressed** by `data_version` (ADR-0009).
`StrategyVersion` is **immutable** with provenance (ADR-0010). Correction is a new
version, never an edit.

**There is one destructive exception and it is currently uncontrolled.** The tree ships
four `*.down.sql` migrations and a publicly exported `undo_to(pool, target_version)`
(`src/adapters/db/migrate.rs`). Calling it applies those down migrations, dropping the
strategy, backtest, trade and LLM-call tables — no backup, no confirmation. No shipped
CLI verb reaches it, so the exception is library-surface only; but "forward-only"
describes the startup path, **not the crate's public API**. Restricting or gating
`undo_to` is open work, not a decided part of this bone.

## Consequences

Reproducibility holds by construction **on the normal startup path**, and there the
determinism fingerprint means something. It does **not** survive the `undo_to`
exception above: a library caller that runs the down migrations destroys the persisted
inputs a result would be re-derived from, so the guarantee is scoped to a database that
has only ever migrated forward. The cost of the immutability discipline is storage
growth and no cheap way to fix a bad row — a destructive or non-forward migration on
the startup path would break the claim outright, which is why it is the revisit trigger
rather than a routine option, and why gating `undo_to` is open work.
