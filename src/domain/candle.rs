//! `Candle` — a single OHLCV(+funding) bar.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// One OHLCV bar plus optional funding rate (FR-5).
///
/// Prices and volume are `rust_decimal::Decimal` (exact, no float
/// nondeterminism — NFR-2) and serialize as strings. Timestamps are `i64` UTC
/// epoch milliseconds (`Binance`-native; no local time). `funding_rate` is
/// `Option<Decimal>` because not every (pair, timeframe) carries funding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candle {
    /// Bar open time, UTC epoch milliseconds.
    pub open_time: i64,
    /// Bar close time, UTC epoch milliseconds.
    pub close_time: i64,
    /// Open price.
    pub open: Decimal,
    /// Highest traded price within the bar.
    pub high: Decimal,
    /// Lowest traded price within the bar.
    pub low: Decimal,
    /// Close price.
    pub close: Decimal,
    /// Traded base-asset volume.
    pub volume: Decimal,
    /// Funding rate effective for the bar, if applicable.
    pub funding_rate: Option<Decimal>,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::Candle;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    fn sample() -> Candle {
        Candle {
            open_time: 1_700_000_000_000,
            close_time: 1_700_000_899_999,
            open: Decimal::from_str("42000.5").unwrap(),
            high: Decimal::from_str("42100.0").unwrap(),
            low: Decimal::from_str("41950.25").unwrap(),
            close: Decimal::from_str("42050.75").unwrap(),
            volume: Decimal::from_str("12.34567").unwrap(),
            funding_rate: Some(Decimal::from_str("0.0001").unwrap()),
        }
    }

    #[test]
    fn candle_serde_round_trips_byte_stable() {
        let candle = sample();
        let json = serde_json::to_string(&candle).expect("serialize candle");
        let back: Candle = serde_json::from_str(&json).expect("deserialize candle");
        assert_eq!(candle, back);
        // Re-serializing the round-tripped value yields the identical bytes.
        let json2 = serde_json::to_string(&back).expect("re-serialize candle");
        assert_eq!(json, json2);
    }

    #[test]
    fn decimal_serializes_as_string_and_timestamps_as_i64() {
        let json = serde_json::to_string(&sample()).expect("serialize candle");
        // Decimal fields are quoted strings (serde-with-str), timestamps bare ints.
        assert!(
            json.contains("\"open\":\"42000.5\""),
            "open as string: {json}"
        );
        assert!(
            json.contains("\"open_time\":1700000000000"),
            "ts as i64: {json}"
        );
        assert!(
            json.contains("\"funding_rate\":\"0.0001\""),
            "funding as string: {json}"
        );
    }

    #[test]
    fn funding_rate_absent_serializes_as_null() {
        let mut candle = sample();
        candle.funding_rate = None;
        let json = serde_json::to_string(&candle).expect("serialize candle");
        assert!(json.contains("\"funding_rate\":null"), "{json}");
        let back: Candle = serde_json::from_str(&json).expect("deserialize candle");
        assert_eq!(back.funding_rate, None);
    }
}
