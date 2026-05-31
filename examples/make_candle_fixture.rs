//! One-off fixture producer (dev tool — not part of the build or runtime path).
//!
//! Trims the local 2-year BTCUSDT snapshots — produced by a live
//! `pulse fetch-data BTCUSDT --tf M15,H4 --years 2` and stored under the default
//! Application Support store — down to a single complete UTC month, and writes a
//! ready-to-read [`CandleStore`] fixture that downstream slices load via
//! `CandleStore::with_base_dir(<fixture>)` + `read_head` / `read_snapshot`.
//!
//! Run from the canonical repo root (the real snapshots must already exist):
//!
//! ```sh
//! cargo run --example make_candle_fixture
//! ```
//!
//! Output: `tests/fixtures/btcusdt-1m-store/candles/BTCUSDT/{15m,4h}/<version>.parquet` (+ `HEAD`).

use std::error::Error;
use std::path::PathBuf;

use chrono::{TimeZone, Utc};
use pulse::{Candle, CandleSeries, CandleStore, Pair, Timeframe};

/// The complete UTC month to extract, as a half-open `[start, end)` window.
/// January 2025 sits comfortably inside the 2-year `--years 2` window.
const YEAR: i32 = 2025;
const MONTH: u32 = 1;

/// `[start, end)` epoch-ms bounds of the target month (UTC).
fn month_window_ms() -> Result<(i64, i64), Box<dyn Error>> {
    let start = Utc
        .with_ymd_and_hms(YEAR, MONTH, 1, 0, 0, 0)
        .single()
        .ok_or("invalid window start")?
        .timestamp_millis();
    let (next_year, next_month) = if MONTH == 12 {
        (YEAR + 1, 1)
    } else {
        (YEAR, MONTH + 1)
    };
    let end = Utc
        .with_ymd_and_hms(next_year, next_month, 1, 0, 0, 0)
        .single()
        .ok_or("invalid window end")?
        .timestamp_millis();
    Ok((start, end))
}

fn main() -> Result<(), Box<dyn Error>> {
    let home = std::env::var("HOME")?;
    let src = CandleStore::with_base_dir(PathBuf::from(format!(
        "{home}/Library/Application Support/PulseTrader"
    )));
    let dst_base = PathBuf::from("tests/fixtures/btcusdt-1m-store");
    let dst = CandleStore::with_base_dir(dst_base.clone());

    let pair = Pair::new("BTCUSDT");
    let (start, end) = month_window_ms()?;

    for tf in [Timeframe::M15, Timeframe::H4] {
        let interval = tf.binance_interval();
        let source_version = src
            .read_head(&pair, tf)?
            .ok_or("no source HEAD — run `pulse fetch-data BTCUSDT --tf M15,H4 --years 2` first")?;
        let full = src.read_snapshot(&pair, tf, &source_version)?;

        let candles: Vec<Candle> = full
            .candles
            .into_iter()
            .filter(|c| c.open_time >= start && c.open_time < end)
            .collect();
        if candles.is_empty() {
            return Err(format!("no {interval} candles in {YEAR}-{MONTH:02}").into());
        }
        let count = candles.len();

        let version = CandleStore::content_version(&pair, tf, &candles);
        let trimmed = CandleSeries {
            pair: pair.clone(),
            timeframe: tf,
            version: version.clone(),
            candles,
        };
        let gaps = trimmed.validate()?;

        dst.write_snapshot(&trimmed)?;
        dst.write_head(&pair, tf, &version)?;

        // Read-back round-trip: the fixture must decode to the same candle set.
        let round_tripped = dst.read_snapshot(&pair, tf, &version)?;
        if round_tripped.candles.len() != count {
            return Err(format!("{interval}: round-trip candle-count mismatch").into());
        }

        println!(
            "{pair} {interval}: {count} candles, version {version}, gaps {}",
            gaps.len()
        );
    }

    println!("fixture written to {}", dst_base.display());
    Ok(())
}
