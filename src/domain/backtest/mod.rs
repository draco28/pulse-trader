//! `backtest` (pure domain) — the money-math + value-type foundation of the
//! deterministic backtester (VS-1.2.1, FR-5 / FR-6, BACKLOG-4).
//!
//! This tree is **zero-I/O, `Decimal`-only** (NFR-2): the immutable trade-record
//! types ([`Trade`] / [`Fill`] / [`ExitReason`] / [`TradeSource`]), the run
//! aggregate ([`BacktestResult`]), the error taxonomy ([`BacktestError`]), and
//! the pure cost / P&L / collision / sizing primitives the event loop (1.03)
//! composes. The concrete loop + `IndicatorEngine` orchestration lives in
//! `adapters::backtest` (gate-2 amendment S1); nothing here iterates candles,
//! reads indicators, or persists.
//!
//! Submodules are kept **additive** (work-1.02 extends this file with the
//! MTF-feed module at the R1→R2 merge — keep-both overlap).

// work-1.01: pure money-math + entities.
mod collision;
mod cost;
mod error;
mod result;
mod sizing;
mod trade;

// Re-exports kept at the `domain::backtest` surface so `domain/mod.rs` + `lib.rs`
// can curate them onto the crate's public API (an un-re-exported public domain
// type is a `dead_code` BUILD error under `deny(warnings)`).
pub use collision::{IntraBarExit, resolve_intra_bar_exit};
pub use cost::{Side, apply_slippage, funding_payment, realized_pnl, realized_r, taker_fee};
pub use error::BacktestError;
pub use result::BacktestResult;
pub use sizing::position_size;
pub use trade::{ExitReason, Fill, Trade, TradeSource};
