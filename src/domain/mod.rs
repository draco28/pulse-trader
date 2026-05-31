//! Domain layer (innermost ring): pure value types, the `MarketDataSource`
//! port, and validation logic. Zero I/O — no `reqwest`/`sqlx`/`polars`/`tokio`
//! in non-test paths (the port's `Send` test uses tokio as a dev-dependency).
//!
//! Dependency policy is "zero I/O", not "zero deps": `serde`, `rust_decimal`,
//! `thiserror`, and `chrono` are permitted.

mod candle;
mod clock;
mod error;
mod pair;
mod port;
mod series;
mod timeframe;
mod version;

pub use candle::Candle;
pub use clock::Clock;
pub use error::{DataError, ValidationError};
pub use pair::Pair;
pub use port::MarketDataSource;
pub use series::{CandleSeries, Gap};
pub use timeframe::Timeframe;
pub use version::DataVersion;

/// Version of the `CandleSeries` on-disk schema (audit C7).
///
/// WI-01 defined no schema version; WI-1.1.1.04 introduces it additively so it
/// can be folded into the content-hash `data_version` (see
/// `crate::adapters::store`). Bumping this constant on any future schema change
/// forces every snapshot to a new `data_version`, preventing a stale snapshot
/// from being mistaken for one written under the new schema.
pub const CANDLE_SCHEMA_VERSION: u32 = 1;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{DataError, ValidationError};

    #[test]
    fn data_error_skeleton_variants_exist_and_serialize() {
        // The audit-C5 documented skeleton: Validation{Unsorted,Duplicate}, Gap, Parse, Io.
        let cases = vec![
            DataError::Validation(ValidationError::Unsorted {
                earlier: 1,
                later: 0,
            }),
            DataError::Validation(ValidationError::Duplicate(7)),
            DataError::Gap {
                expected: 900_000,
                found: 1_800_000,
            },
            DataError::Parse("bad decimal".to_string()),
            DataError::Io("disk full".to_string()),
        ];

        for err in cases {
            // serde round-trip (errors must cross the Tauri boundary later).
            let json = serde_json::to_string(&err).expect("serialize DataError");
            let back: DataError = serde_json::from_str(&json).expect("deserialize DataError");
            assert_eq!(err, back);
            // thiserror Display is non-empty.
            assert!(!err.to_string().is_empty());
        }
    }
}
