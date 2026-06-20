# PulseTrader — Architecture Overview

> High-level guide to the code in this repository. Describes what exists today;
> it is not a roadmap.

---

## Repository layout

```
pulse-trader/
├── src/
│   ├── domain/        # Pure domain — types, ports, no external deps
│   ├── adapters/      # Binance, SQLite, Parquet, indicator implementations
│   ├── agent/         # Agent orchestration (PulseHive-backed)
│   ├── cli/           # CLI surface (proof-of-concept entry point)
│   └── tauri/         # Tauri command-bus stubs (future native-app shell)
├── docs/adr/          # Architecture Decision Records
├── migrations/        # sqlx migration files for the SQLite schema
├── tests/             # Integration tests
└── rust-toolchain.toml
```

---

## Hexagonal (ports-and-adapters) layers

PulseTrader follows a strict hexagonal architecture. Dependency direction is
always inward — outer layers depend on inner ones, never the reverse.

### `domain/` — the pure core

Contains all business types (`Candle`, `Pair`, `Timeframe`, `Strategy`,
`StrategyVersion`, `CandleSeries`, indicator traits) and the port traits that
define every external concern:

| Port trait | Purpose |
|---|---|
| `ExchangeAdapter` | Fetch historical candles + live feeds |
| `LlmProvider` | Issue prompts, receive structured responses |
| `StrategyRepository` | Persist and load `Strategy` / `StrategyVersion` |
| `CandleSeriesRepository` | Read / write immutable Parquet candle snapshots |
| `MarketDataSource` | Abstraction over bulk, REST, and WebSocket data |
| `EventBus` | In-process event dispatch |
| `Clock` | Mockable wall time |
| `Indicator` | Streaming indicator computation |

The domain module has zero external crate dependencies (no sqlx, no reqwest,
no Polars). All computation inside `domain/` is deterministic by construction.

### `adapters/` — concrete implementations

| Sub-module | Implements |
|---|---|
| `adapters/binance/` | `ExchangeAdapter` + `MarketDataSource` via Binance bulk dumps, REST, and WebSocket |
| `adapters/db/` | `StrategyRepository` using sqlx + SQLite WAL |
| `adapters/store/` | `CandleSeriesRepository` using Polars + immutable Parquet snapshots |
| `adapters/indicators/` | EMA, RSI, MACD, ATR, Bollinger streaming implementations |

### `agent/` — agent orchestration

Wraps PulseHive (the author-owned in-Rust multi-agent SDK) to run the
Composer and Coach agent roles entirely in-process. No Python layer; no
sidecar process.

### `cli/` — command-line surface

The `pulse` binary entry point. Parses arguments via `clap`, wires up
adapters, and drives the compose → backtest → coach loop. The CLI is a
proof-of-concept proving ground; the end product is the native macOS app.

### `tauri/` — desktop shell stubs

Stub module for the future Tauri command bus. The Tauri desktop app will
share this same Rust core; the CLI and the app differ only at this outermost
layer.

---

## Storage tiers

Four storage tiers; no server processes, no Postgres, no Redis.

### 1. SQLite (WAL) — entity store and event log

- Path: `~/Library/Application Support/PulseTrader/pulse.db`
- Managed by sqlx with compile-time-checked queries and `sqlx migrate`.
- Exact version pin: `sqlx = "=0.8.6"` (determinism — see ADR-0007 rationale
  in `docs/adr/`).
- `StrategyVersion` rows are **immutable by DB trigger** (`BEFORE UPDATE` /
  `BEFORE DELETE` each `RAISE(ABORT, …)`) — see [ADR-0010](docs/adr/0010-strategyversion-identity-immutability-provenance-storage.md).
- Startup protocol: if a schema migration would alter a table that already
  holds rows, the app backs up the database first and refuses to start if the
  backup fails.

### 2. Parquet candle snapshots — immutable time-series store

- Path: `<base>/candles/<PAIR>/<TF>/<data_version>.parquet`
- Each snapshot is **write-once, content-addressed**. `data_version` is a
  truncated SHA-256 over a canonical encoding of `(pair, timeframe,
  CANDLE_SCHEMA_VERSION, candles)` — see [ADR-0009](docs/adr/0009-data-version-content-hash-scheme.md).
- Read via Polars / arrow-rs. Snapshots are never overwritten; a new content
  hash means a new file.

### 3. macOS Keychain — secrets

API keys and other secrets are stored in and retrieved from the macOS Keychain
only. They are never written to disk in plaintext.

### 4. Filesystem — logs and exports

Structured logs and optional verbatim LLM-call archives are written to the
filesystem under the application support directory. These are auxiliary;
the SQLite DB is the system-of-record for all entities.

---

## Determinism stance

Byte-reproducible backtests are a load-bearing requirement (NFR-2). Three
mechanisms enforce it:

1. **FMA / fast-math disabled.** The backtester opts out of fused
   multiply-add and other re-associating compiler optimisations so floating-
   point results are identical on aarch64 and x86\_64.

2. **Pinned toolchain.** `rust-toolchain.toml` pins an exact Rust version
   (`1.92.0`), not a floating `stable` channel. The toolchain version feeds
   `engine_fingerprint`. See the comment in `rust-toolchain.toml` — bumps are
   deliberate and ADR-recorded.

3. **`engine_fingerprint`.** Every `BacktestRun` record stores a hash of
   (crate versions + Rust toolchain + DSL schema version + target architecture
   triple). A CI test asserts that running the same strategy 100× on both
   Apple Silicon and x86\_64 produces byte-identical results.

Key dependency pins that protect reproducibility:
- `ta = "=0.5.0"` — indicator library; an upgrade can shift low-decimal outputs.
- `sqlx = "=0.8.6"` — pins the compile-time query cache format.

---

## Architecture Decision Records

All load-bearing architectural decisions are recorded in `docs/adr/`:

| ADR | Subject |
|---|---|
| [0001](docs/adr/0001-record-architecture-decisions.md) | Why and how ADRs are used in this project |
| [0009](docs/adr/0009-data-version-content-hash-scheme.md) | Content-hash scheme for candle-snapshot `data_version` ids |
| [0010](docs/adr/0010-strategyversion-identity-immutability-provenance-storage.md) | `StrategyVersion` identity, immutability, and provenance storage |

ADRs 0002–0008 are recorded as Proposed in ADR-0001 (decision queue); full
write-ups will be added as each decision becomes implementation-load-bearing.
