//! Application ring (r1.s3.w3) — use cases shared by every delivery adapter.
//!
//! **Why a ring at all.** Before this, the version-id backtest flow lived inside
//! `src/cli/backtest.rs`: load version → compile → load snapshots → reject gaps →
//! resolve filters → run → compare fingerprint → persist. The desktop command needs
//! the identical sequence, and a second copy of it is a second place for the order
//! to drift — the FR-7 compare-before-insert ordering alone is a correctness rule
//! that only reads as one if it exists once.
//!
//! **What belongs here.** Orchestration of domain ports, and nothing else. This ring
//! names no infrastructure adapter — no `tauri`, no `specta`, no `sqlx`, and no
//! filesystem type — and is generic over the ports in `crate::domain::port`,
//! returning domain values plus its own typed errors. The one deliberate adapter
//! import is the deterministic engine itself,
//! `crate::adapters::backtest::run_backtest`: it owns no I/O (its `adapters`
//! address is namespace, not infrastructure — it lives there because it owns the
//! concrete `IndicatorEngine`), so running it from the ring breaks no boundary the
//! hexagonal scan enforces. `src/cli/mod.rs` and
//! `src/tauri/commands.rs::DesktopState` are the composition roots that choose
//! implementations (ADR-0015).
//!
//! **What deliberately does not belong here.** Any order, broker or execution
//! capability. r1 is backtest-only, and the risk gate's kill-switch and
//! progressive-exposure controls are discharged by the dependency set being
//! *incapable* of placing an order rather than by a flag that disables one.
//! `tests/tauri_backtest.rs` scans this ring for exactly that.

pub(crate) mod backtest;
