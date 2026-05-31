//! Integration tests for the Binance bulk-ingest adapter, driven entirely over
//! checked-in recorded fixtures — NO live network (AC-6).
//!
//! These exercise the library's *public* surface (`decode_month`,
//! `verify_archive_checksum`, `ingest_window` + a fixture `MonthSource`),
//! proving the recorded-fixture path end to end: unzip → parse 12-col klines
//! with header detection (AC-2/AC-8) → normalize/sort/dedup + gap report (AC-3)
//! → sparse half-open funding stamping (AC-4), with `.CHECKSUM` integrity
//! (AC-7).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::future::Future;
use std::str::FromStr;

use pulse::{
    Candle, CandleSeries, DataError, DataVersion, Gap, MonthData, MonthOutcome, MonthSource, Pair,
    Timeframe, decode_month, ingest_window, verify_archive_checksum,
};
use rust_decimal::Decimal;

const KLINES_ZIP: &[u8] = include_bytes!("fixtures/binance/BTCUSDT-15m-2024-01.zip");
const KLINES_CHECKSUM: &str = include_str!("fixtures/binance/BTCUSDT-15m-2024-01.zip.CHECKSUM");
const FUNDING_ZIP: &[u8] = include_bytes!("fixtures/binance/BTCUSDT-fundingRate-2024-01.zip");
const FUNDING_CHECKSUM: &str =
    include_str!("fixtures/binance/BTCUSDT-fundingRate-2024-01.zip.CHECKSUM");

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

// ---- AC-7: recorded archives verify against their .CHECKSUM ---------------

#[test]
fn recorded_archives_match_their_published_checksums() {
    verify_archive_checksum(KLINES_ZIP, KLINES_CHECKSUM).expect("klines checksum matches");
    verify_archive_checksum(FUNDING_ZIP, FUNDING_CHECKSUM).expect("funding checksum matches");
}

#[test]
fn corrupted_archive_fails_checksum() {
    let mut corrupt = KLINES_ZIP.to_vec();
    corrupt.push(0xFF); // single trailing byte flips the digest
    let err = verify_archive_checksum(&corrupt, KLINES_CHECKSUM).expect_err("must reject");
    assert!(matches!(err, DataError::Io(_)));
}

// ---- AC-2/AC-8: recorded zip decodes to hand-verified candles -------------

#[test]
fn recorded_klines_zip_decodes_to_expected_candles() {
    let month = decode_month(KLINES_ZIP, Some(FUNDING_ZIP)).expect("decode month");
    assert_eq!(month.candles.len(), 3, "fixture holds 3 M15 candles");

    let first = &month.candles[0];
    assert_eq!(first.open_time, 1_700_000_000_000);
    assert_eq!(first.close_time, 1_700_000_899_999);
    assert_eq!(first.open, dec("42000.5"));
    assert_eq!(first.high, dec("42100.0"));
    assert_eq!(first.low, dec("41950.25"));
    assert_eq!(first.close, dec("42050.75"));
    assert_eq!(first.volume, dec("12.34567"));

    let last = &month.candles[2];
    assert_eq!(last.open_time, 1_700_001_800_000);
    assert_eq!(last.close, dec("42120.40"));

    // Funding fixture holds one on-boundary event at the 2nd candle's open.
    assert_eq!(month.funding.len(), 1);
    assert_eq!(month.funding[0].calc_time, 1_700_000_900_000);
    assert_eq!(month.funding[0].rate, dec("0.00010000"));
}

// ---- AC-2/AC-3/AC-4: full ingest_window over a fixture MonthSource --------

/// A fixture `MonthSource` that decodes the recorded zips for month 2024-01 and
/// reports every other month as `Absent` (pre-listing) — no network.
struct FixtureSource;

impl MonthSource for FixtureSource {
    fn load_month(
        &self,
        _pair: &Pair,
        _tf: Timeframe,
        year: i32,
        month: u32,
    ) -> impl Future<Output = Result<MonthOutcome, DataError>> + Send {
        let outcome = if (year, month) == (2024, 1) {
            decode_month(KLINES_ZIP, Some(FUNDING_ZIP)).map(MonthOutcome::Loaded)
        } else {
            Ok(MonthOutcome::Absent)
        };
        async move { outcome }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ingest_window_over_fixtures_normalizes_and_stamps_funding() {
    let (series, gaps): (CandleSeries, Vec<Gap>) = ingest_window(
        &FixtureSource,
        &Pair::new("BTCUSDT"),
        Timeframe::M15,
        &DataVersion::new("fixture-v1"),
        // A leading pre-listing month is skipped (AC-7), then the real month.
        &[(2023, 12), (2024, 1)],
    )
    .await
    .expect("ingest over fixtures");

    // Sorted ascending, contiguous → no gaps reported (AC-3).
    let times: Vec<i64> = series.candles.iter().map(|c| c.open_time).collect();
    assert_eq!(
        times,
        vec![1_700_000_000_000, 1_700_000_900_000, 1_700_001_800_000]
    );
    assert!(gaps.is_empty(), "contiguous fixture: {gaps:?}");

    // Funding stamped on exactly the on-boundary candle (sparse, no fill — AC-4).
    let stamped: Vec<Option<Decimal>> = series.candles.iter().map(|c| c.funding_rate).collect();
    assert_eq!(
        stamped,
        vec![None, Some(dec("0.00010000")), None],
        "only the candle that opens at the funding ts is stamped"
    );
}

/// A `MonthSource` over the gapped klines fixture (no funding) to prove gaps are
/// reported through `Ok`, not raised, and not filled (AC-3).
struct GappedSource;

const GAPPED_CSV: &str = include_str!("fixtures/binance/klines-gapped.csv");

impl MonthSource for GappedSource {
    fn load_month(
        &self,
        _pair: &Pair,
        _tf: Timeframe,
        _year: i32,
        _month: u32,
    ) -> impl Future<Output = Result<MonthOutcome, DataError>> + Send {
        // Build candles directly from the gapped CSV via a tiny inline parse so
        // the integration test does not need a zip for this case.
        let candles = gapped_candles();
        async move {
            Ok(MonthOutcome::Loaded(MonthData {
                candles,
                funding: vec![],
            }))
        }
    }
}

fn gapped_candles() -> Vec<Candle> {
    GAPPED_CSV
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let f: Vec<&str> = line.split(',').collect();
            Candle {
                open_time: f[0].parse().unwrap(),
                close_time: f[6].parse().unwrap(),
                open: dec(f[1]),
                high: dec(f[2]),
                low: dec(f[3]),
                close: dec(f[4]),
                volume: dec(f[5]),
                funding_rate: None,
            }
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ingest_window_reports_gap_without_filling() {
    let (series, gaps) = ingest_window(
        &GappedSource,
        &Pair::new("BTCUSDT"),
        Timeframe::M15,
        &DataVersion::new("fixture-v1"),
        &[(2024, 1)],
    )
    .await
    .expect("gapped ingest still Ok");

    // The hole (missing 1_700_000_900_000) is reported, NOT filled.
    assert_eq!(series.candles.len(), 3, "no fill: hole stays a hole");
    assert_eq!(gaps.len(), 1);
    assert_eq!(gaps[0].expected, 1_700_000_900_000);
    assert_eq!(gaps[0].found, 1_700_001_800_000);
}
