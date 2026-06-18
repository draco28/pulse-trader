//! Backtest domain: the deterministic event-loop substrate (VS-1.2.1).
//!
//! Pure logic over already-loaded [`CandleSeries`](crate::domain::CandleSeries):
//! no I/O, no `f64`. This slice's first member is the MTF-aligned, no-look-ahead
//! candle [`feed`] (work-1.02); sibling work items extend this module additively.

pub(crate) mod feed;

pub use feed::{AlignedBar, align};
