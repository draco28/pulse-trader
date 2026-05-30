//! `Timeframe` — the candle intervals v1 supports (M15, H4).

use serde::{Deserialize, Serialize};

/// A candle interval. v1 supports two (MASTER-SPEC Phase 1: BTCUSDT M15 + H4).
///
/// The serde representation is the `Binance` interval string (`"15m"` / `"4h"`)
/// so a serialized `Timeframe` round-trips through the exchange's vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Timeframe {
    /// 15-minute candles.
    #[serde(rename = "15m")]
    M15,
    /// 4-hour candles.
    #[serde(rename = "4h")]
    H4,
}

impl Timeframe {
    /// The `Binance` interval string for this timeframe (`"15m"` / `"4h"`).
    #[must_use]
    pub fn binance_interval(self) -> &'static str {
        match self {
            Timeframe::M15 => "15m",
            Timeframe::H4 => "4h",
        }
    }

    /// The nominal duration of one candle, in milliseconds.
    ///
    /// `M15` → `900_000`, `H4` → `14_400_000`. Used by gap detection in
    /// [`CandleSeries::validate`](crate::domain::CandleSeries::validate).
    #[must_use]
    pub fn duration_ms(self) -> i64 {
        match self {
            Timeframe::M15 => 900_000,
            Timeframe::H4 => 14_400_000,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::Timeframe;

    #[test]
    fn binance_interval_maps_correctly() {
        assert_eq!(Timeframe::M15.binance_interval(), "15m");
        assert_eq!(Timeframe::H4.binance_interval(), "4h");
    }

    #[test]
    fn duration_ms_is_correct() {
        assert_eq!(Timeframe::M15.duration_ms(), 900_000);
        assert_eq!(Timeframe::H4.duration_ms(), 14_400_000);
    }

    #[test]
    fn serde_renames_round_trip() {
        // M15 serializes to the Binance interval string, then back.
        let json = serde_json::to_string(&Timeframe::M15).expect("serialize M15");
        assert_eq!(json, "\"15m\"");
        let back: Timeframe = serde_json::from_str(&json).expect("deserialize M15");
        assert_eq!(back, Timeframe::M15);

        let json = serde_json::to_string(&Timeframe::H4).expect("serialize H4");
        assert_eq!(json, "\"4h\"");
        let back: Timeframe = serde_json::from_str(&json).expect("deserialize H4");
        assert_eq!(back, Timeframe::H4);
    }
}
