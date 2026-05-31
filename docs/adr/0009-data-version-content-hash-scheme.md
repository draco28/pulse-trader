# 9. `data_version` content-hash scheme for candle snapshots

Date: 2026-05-31T00:00:00Z

## Status

Accepted

(Implements the storage contract of ADR-0007 — versioned immutable Parquet snapshots per `(pair, timeframe, data_version)`. ADR-0007 fixed *that* snapshots are versioned; this ADR fixes *how* the version id is computed.)

## Context

VS-1.1.1 (the Binance historical data pipeline) is the first producer of the immutable `CandleSeries` Parquet snapshots that every later slice (backtest, paper, live) consumes. The store path is `<base>/candles/<PAIR>/<TF>/<data_version>.parquet`, so the `data_version` is both the file name and the identity of the data. The MASTER-SPEC left the scheme open (slice plan: *"deterministic snapshot id — recommend content-hash of (range + source + schema_version) or YYYYMMDD-seq"* — flagged ADR-worthy). The slice-close audit (C7) required the chosen scheme to be recorded.

Three forces shaped the choice:

1. **Idempotency.** A re-fetch of an unchanged window must be a no-op, not a duplicate snapshot. That requires the id to be a function of the *content*, so identical candles yield the same id and the existing file is recognised.
2. **Immutability + change-detection.** Any change to the candle set (a corrected bar, an extended window, an incremental top-up) must produce a *new* version automatically, never overwrite an existing one. A content hash gives this for free: different content ⇒ different id.
3. **Cross-run / cross-architecture reproducibility (NFR-2).** The same logical data must hash to the same id on aarch64 and x86_64, across process runs, so two machines agree on snapshot identity.

A purely sequential or date-based id (`YYYYMMDD-seq`) satisfies none of these: it cannot tell an identical re-fetch from a new one, and it couples identity to wall-clock rather than data.

## Decision

`data_version` is a **content hash** over a stable canonical encoding of the snapshot inputs, truncated to a 64-bit hex id:

```
data_version = sha256( canonical(pair, timeframe, CANDLE_SCHEMA_VERSION, candles) )[..16 hex]
```

The canonical encoding (`adapters/store/version.rs::feed_canonical`) is **unambiguous by construction**:

- Fixed field order: `pair`, `timeframe` (Binance interval string), `CANDLE_SCHEMA_VERSION` (u32, big-endian), candle count (u64, big-endian), then each candle in series order.
- Per candle: `open_time` + `close_time` as big-endian i64 ms; `open/high/low/close/volume` as **exact UTF-8 `Decimal` strings** (the same bytes written to the Parquet columns); `funding_rate` as a 1-byte present/absent tag followed by its `Decimal` string when present.
- All strings are length-prefixed (8-byte big-endian length + bytes) so no two distinct inputs can collide via concatenation-boundary ambiguity.

`CANDLE_SCHEMA_VERSION` is folded into the hash so a schema change re-versions all snapshots automatically. Write-time enforcement (added during the slice-close PR) re-derives the content hash inside `write_snapshot` and rejects any attempt to write under a mismatched version.

## Consequences

**Positive**

- Re-fetching an unchanged window is a content-addressed no-op (the existing file is recognised by id) — the live `--years 2` second run is an `up-to-date` no-op, as demonstrated at slice close.
- Immutability is intrinsic: new data ⇒ new id ⇒ new file; an existing version is never overwritten.
- The id is reproducible across runs and architectures because it hashes the exact decoded values (i64 ms + UTF-8 `Decimal` strings), not floats and not file bytes.
- A 64-bit id is short enough for a readable file name and collision-safe at the snapshot cardinality this system produces.

**Negative / limitations (tracked)**

- **Content-identity, not file-byte-identity.** This scheme guarantees that the *decoded candles* are reproducible and that identical candles share an id. It does **not** guarantee the Parquet *file bytes* are identical across Polars versions — writer metadata varies — so NFR-2's "byte-identical reads" holds at the candle/`data_version` level, not at the raw-file level. Cross-Polars writer-metadata normalisation is tracked in [#5](https://github.com/draco28/pulse-trader/issues/5).
- **`Decimal` formatting sensitivity.** The hash feeds `Decimal::to_string()` with no canonical quantization, so two representations of the same economic value (e.g. differing trailing zeros / scale between the bulk CSV and the REST JSON) would mint different ids. v1 relies on consistent `Decimal` parsing across both sources; canonical per-field quantization is future hardening (surfaced by the slice-close architect-critic; file an issue if bulk/REST scale drift is observed).
- **No in-place schema migration.** Bumping `CANDLE_SCHEMA_VERSION` re-versions snapshots by design, but there is no migration/invalidation path for existing on-disk snapshots when the schema changes — they retain their old ids and a re-fetch produces new ones. A migration story is deferred future work.

## Alternatives considered

- **`YYYYMMDD-seq` sequential id** — rejected: cannot detect an identical re-fetch (breaks idempotency) and couples identity to wall-clock rather than data.
- **Hash of the full Parquet file bytes** — rejected: not stable across Polars/arrow-rs versions (writer metadata differs), so the same candles could yield different ids on a toolchain bump.
- **Full 256-bit (64-hex) id** — rejected as unnecessary: a 64-bit id is collision-safe for per-`(pair,tf)` snapshot counts and keeps file names readable; the scheme can widen later without changing the canonical encoding.
