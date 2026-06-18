//! Deterministic backtest event loop adapter.

mod engine;

pub use engine::{BacktestConfig, run_backtest};
