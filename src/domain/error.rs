//! `DataError` — the data-pipeline error taxonomy (audit C5).
//!
//! WI-01 ships the documented *initial skeleton*. Downstream work items extend
//! it **additively** (e.g. WI-02 adds `Http`, WI-04 adds `SnapshotExists`) — no
//! variant is renamed or removed, so the shared file never needs a rewrite.
//! `thiserror`-derived for ergonomic `Display`/`Error`, and `serde`-serializable
//! so errors can cross the `Tauri` boundary later. No library path panics: the
//! crate denies `clippy::unwrap_used` / `expect_used`.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Errors produced by the data pipeline (domain layer).
///
/// Structural-corruption variants (`Validation`) are *rejections*; `Gap` is a
/// *report* surfaced through `Ok` by [`CandleSeries::validate`], not raised as
/// an error (audit C2). It is kept as a variant so adapters that fetch with a
/// strict gap policy can still raise it explicitly.
///
/// [`CandleSeries::validate`]: crate::domain::CandleSeries::validate
#[derive(Debug, Clone, PartialEq, Eq, Error, Serialize, Deserialize)]
#[non_exhaustive]
pub enum DataError {
    /// A `CandleSeries` is structurally corrupt and cannot be trusted.
    #[error("candle series validation failed: {0}")]
    Validation(#[from] ValidationError),

    /// A spacing discontinuity between two adjacent candles.
    ///
    /// Reported (not rejected) by validation; an error variant only so strict
    /// adapter policies can raise it deliberately.
    #[error("gap in candle series: expected open_time {expected} ms, found {found} ms")]
    Gap {
        /// The `open_time` the next candle was expected at (epoch ms).
        expected: i64,
        /// The `open_time` actually found (epoch ms).
        found: i64,
    },

    /// A value could not be parsed (e.g. a malformed Decimal from the exchange).
    #[error("parse error: {0}")]
    Parse(String),

    /// An I/O failure (filesystem, network — represented as a message so the
    /// domain stays free of `std::io::Error`'s non-`Serialize` payload).
    #[error("io error: {0}")]
    Io(String),
}

/// The specific kinds of structural corruption a `CandleSeries` can exhibit.
#[derive(Debug, Clone, PartialEq, Eq, Error, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ValidationError {
    /// Candle `open_time`s are not monotonically increasing.
    #[error("candles are not sorted by open_time (found {later} ms after {earlier} ms)")]
    Unsorted {
        /// The earlier candle's `open_time` (epoch ms).
        earlier: i64,
        /// The out-of-order candle's `open_time` (epoch ms).
        later: i64,
    },

    /// Two candles share the same `open_time`.
    #[error("duplicate open_time {0} ms")]
    Duplicate(i64),
}
