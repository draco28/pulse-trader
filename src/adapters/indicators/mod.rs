//! Indicator adapters (VS-1.1.3) — concrete [`crate::domain::Indicator`] impls
//! wrapping ta-rs, plus the `Decimal↔f64` conversion seam.
//!
//! This is the **only** module in the crate where `f64` is permitted: floats are
//! quarantined behind the domain port. `convert` is the conversion contract; `ema`
//! is the walking-skeleton adapter (3.01). RSI/ADX/MACD land in 3.02; the
//! multi-indicator engine / `EvalContext` impl in 3.03.

pub mod adx;
pub mod convert;
pub mod ema;
