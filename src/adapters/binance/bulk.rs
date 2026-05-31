//! Bulk-archive plumbing: monthly archive URL construction, `.CHECKSUM`
//! (SHA256) verification, unzip, and 12-column USD-M futures kline CSV parsing
//! with header-row detection.
//!
//! Decisions locked here (spec §3, audit C2/C3):
//! - URLs target `data.binance.vision` USD-M monthly archives (AC-1).
//! - Each archive is verified against its published `.CHECKSUM` (SHA256) before
//!   the CSV is parsed (AC-7, NFR-2).
//! - The kline schema is pinned to the 12 USD-M futures columns and the parser
//!   *detects* whether the first row is a header rather than assuming (AC-8).

use std::io::Read;

use rust_decimal::Decimal;

use crate::domain::{Candle, DataError, Pair, Timeframe};

/// Base host for the no-rate-limit bulk dumps.
const VISION_BASE: &str = "https://data.binance.vision";

/// The number of columns in a USD-M futures kline CSV row (audit C3).
const KLINE_COLUMNS: usize = 12;

/// The set of URLs needed to ingest one monthly klines archive: the archive
/// itself plus its published SHA256 `.CHECKSUM` sidecar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KlineArchive {
    /// The `.zip` archive URL.
    pub archive: String,
    /// The `.CHECKSUM` sidecar URL (SHA256 of the archive).
    pub checksum: String,
}

/// Build the monthly klines + funding archive URLs for `(pair, tf)` and a
/// calendar month (AC-1).
///
/// Returns `(klines, funding)` where `klines` carries both the archive and its
/// checksum sidecar, and `funding` is the funding-rate archive (also paired
/// with its checksum). `year`/`month` identify the calendar month; `month` is
/// 1-based.
///
/// # Panics
///
/// Never — `month` is formatted, not indexed.
#[must_use]
pub(crate) fn archive_urls(
    pair: &Pair,
    tf: Timeframe,
    year: i32,
    month: u32,
) -> (KlineArchive, KlineArchive) {
    let sym = pair.as_str();
    let interval = tf.binance_interval();
    let ym = format!("{year:04}-{month:02}");

    let klines_archive = format!(
        "{VISION_BASE}/data/futures/um/monthly/klines/{sym}/{interval}/\
         {sym}-{interval}-{ym}.zip"
    );
    let funding_archive = format!(
        "{VISION_BASE}/data/futures/um/monthly/fundingRate/{sym}/{sym}-fundingRate-{ym}.zip"
    );

    (
        KlineArchive {
            checksum: format!("{klines_archive}.CHECKSUM"),
            archive: klines_archive,
        },
        KlineArchive {
            checksum: format!("{funding_archive}.CHECKSUM"),
            archive: funding_archive,
        },
    )
}

/// Verify `archive_bytes` against the SHA256 hex digest published in a
/// `.CHECKSUM` sidecar (AC-7, NFR-2).
///
/// Binance `.CHECKSUM` files have the shape `<hex-sha256>  <filename>`; only the
/// leading hex token is significant. Returns `Ok(())` on a match.
///
/// # Errors
///
/// [`DataError::Parse`] if the sidecar carries no hex token; [`DataError::Io`]
/// if the computed digest does not match the published one (a corrupt or
/// truncated download).
pub(crate) fn verify_checksum(archive_bytes: &[u8], checksum_body: &str) -> Result<(), DataError> {
    use sha2::{Digest, Sha256};

    let expected = checksum_body
        .split_whitespace()
        .next()
        .ok_or_else(|| DataError::Parse("empty .CHECKSUM sidecar".to_string()))?
        .to_ascii_lowercase();

    let mut hasher = Sha256::new();
    hasher.update(archive_bytes);
    let actual = hex::encode(hasher.finalize());

    if actual == expected {
        Ok(())
    } else {
        Err(DataError::Io(format!(
            "archive checksum mismatch: expected {expected}, computed {actual}"
        )))
    }
}

/// Extract the single CSV member from a Binance monthly `.zip` archive.
///
/// # Errors
///
/// [`DataError::Parse`] if the archive is not a valid zip or holds no entries;
/// [`DataError::Io`] if a zip entry cannot be read.
pub(crate) fn unzip_single_csv(archive_bytes: &[u8]) -> Result<String, DataError> {
    let reader = std::io::Cursor::new(archive_bytes);
    let mut zip = zip::ZipArchive::new(reader)
        .map_err(|e| DataError::Parse(format!("invalid zip archive: {e}")))?;

    if zip.is_empty() {
        return Err(DataError::Parse("zip archive holds no entries".to_string()));
    }

    let mut entry = zip
        .by_index(0)
        .map_err(|e| DataError::Io(format!("cannot open zip entry: {e}")))?;
    let mut body = String::new();
    entry
        .read_to_string(&mut body)
        .map_err(|e| DataError::Io(format!("cannot read zip entry: {e}")))?;
    Ok(body)
}

/// Parse a USD-M futures klines CSV body into [`Candle`]s, pinning the 12-column
/// schema and detecting whether the first row is a header (AC-2, AC-8).
///
/// Funding is left `None`; [`super::funding::stamp_funding`] attaches it later.
/// Header detection inspects the first field of the first record: a header row
/// begins with the literal column name `open_time`, whereas a data row begins
/// with an integer timestamp.
///
/// # Errors
///
/// [`DataError::Parse`] if a row does not have exactly 12 columns or a field
/// fails to parse as its pinned type.
pub(crate) fn parse_klines(csv_body: &str) -> Result<Vec<Candle>, DataError> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(false)
        .from_reader(csv_body.as_bytes());

    let mut candles = Vec::new();
    for record in reader.records() {
        let record =
            record.map_err(|e| DataError::Parse(format!("malformed kline CSV row: {e}")))?;

        if record.len() != KLINE_COLUMNS {
            return Err(DataError::Parse(format!(
                "expected {KLINE_COLUMNS} kline columns, found {}",
                record.len()
            )));
        }

        // Header detection (AC-8): a header row's first cell is the literal
        // column name, not an integer timestamp. Skip it.
        let first = record.get(0).unwrap_or_default();
        if first.eq_ignore_ascii_case("open_time") {
            continue;
        }

        candles.push(parse_kline_row(&record)?);
    }

    Ok(candles)
}

/// Parse one 12-column kline record into a [`Candle`] (funding left `None`).
fn parse_kline_row(record: &csv::StringRecord) -> Result<Candle, DataError> {
    let open_time = parse_i64(record.get(0), "open_time")?;
    let open = parse_decimal(record.get(1), "open")?;
    let high = parse_decimal(record.get(2), "high")?;
    let low = parse_decimal(record.get(3), "low")?;
    let close = parse_decimal(record.get(4), "close")?;
    let volume = parse_decimal(record.get(5), "volume")?;
    let close_time = parse_i64(record.get(6), "close_time")?;
    // Columns 7..12 (quote_volume, count, taker_buy_base, taker_buy_quote,
    // ignore) are pinned by the 12-column count check but not retained.

    Ok(Candle {
        open_time,
        close_time,
        open,
        high,
        low,
        close,
        volume,
        funding_rate: None,
    })
}

/// Parse a CSV cell as an `i64`, attributing failures to `field`.
fn parse_i64(cell: Option<&str>, field: &str) -> Result<i64, DataError> {
    cell.unwrap_or_default()
        .trim()
        .parse::<i64>()
        .map_err(|e| DataError::Parse(format!("{field}: {e}")))
}

/// Parse a CSV cell as a `Decimal`, attributing failures to `field`.
fn parse_decimal(cell: Option<&str>, field: &str) -> Result<Decimal, DataError> {
    cell.unwrap_or_default()
        .trim()
        .parse::<Decimal>()
        .map_err(|e| DataError::Parse(format!("{field}: {e}")))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{archive_urls, parse_klines, unzip_single_csv, verify_checksum};
    use crate::domain::{Pair, Timeframe};
    use rust_decimal::Decimal;
    use std::str::FromStr;

    // ---- AC-1: URL construction ------------------------------------------

    #[test]
    fn builds_monthly_klines_and_funding_urls_for_btcusdt_m15() {
        let (klines, funding) = archive_urls(&Pair::new("BTCUSDT"), Timeframe::M15, 2024, 1);
        assert_eq!(
            klines.archive,
            "https://data.binance.vision/data/futures/um/monthly/klines/\
             BTCUSDT/15m/BTCUSDT-15m-2024-01.zip"
        );
        assert_eq!(klines.checksum, format!("{}.CHECKSUM", klines.archive));
        assert_eq!(
            funding.archive,
            "https://data.binance.vision/data/futures/um/monthly/fundingRate/\
             BTCUSDT/BTCUSDT-fundingRate-2024-01.zip"
        );
        assert_eq!(funding.checksum, format!("{}.CHECKSUM", funding.archive));
    }

    #[test]
    fn builds_h4_url_and_zero_pads_month() {
        let (klines, _) = archive_urls(&Pair::new("BTCUSDT"), Timeframe::H4, 2023, 11);
        assert_eq!(
            klines.archive,
            "https://data.binance.vision/data/futures/um/monthly/klines/\
             BTCUSDT/4h/BTCUSDT-4h-2023-11.zip"
        );
        // Single-digit months are zero-padded.
        let (single, _) = archive_urls(&Pair::new("BTCUSDT"), Timeframe::H4, 2023, 3);
        assert!(single.archive.ends_with("BTCUSDT-4h-2023-03.zip"));
    }

    // ---- AC-8: 12-column schema + header detection -----------------------

    const HEADERLESS_ROW: &str = "1700000000000,42000.5,42100.0,41950.25,42050.75,12.34567,\
         1700000899999,518000.0,1234,6.0,252000.0,0";

    fn header_line() -> &'static str {
        "open_time,open,high,low,close,volume,close_time,quote_volume,\
         count,taker_buy_base,taker_buy_quote,ignore"
    }

    #[test]
    fn parses_headerless_12col_row_to_exact_candle() {
        let candles = parse_klines(HEADERLESS_ROW).expect("headerless parses");
        assert_eq!(candles.len(), 1);
        let c = &candles[0];
        assert_eq!(c.open_time, 1_700_000_000_000);
        assert_eq!(c.close_time, 1_700_000_899_999);
        assert_eq!(c.open, Decimal::from_str("42000.5").unwrap());
        assert_eq!(c.high, Decimal::from_str("42100.0").unwrap());
        assert_eq!(c.low, Decimal::from_str("41950.25").unwrap());
        assert_eq!(c.close, Decimal::from_str("42050.75").unwrap());
        assert_eq!(c.volume, Decimal::from_str("12.34567").unwrap());
        assert_eq!(c.funding_rate, None);
    }

    #[test]
    fn header_and_headerless_parse_to_identical_candles() {
        let headerless = parse_klines(HEADERLESS_ROW).expect("headerless parses");
        let with_header =
            parse_klines(&format!("{}\n{HEADERLESS_ROW}", header_line())).expect("header parses");
        assert_eq!(headerless, with_header);
        assert_eq!(with_header.len(), 1, "header row must not become a candle");
    }

    #[test]
    fn wrong_column_count_is_parse_error() {
        // 11 columns (missing the trailing `ignore`).
        let row = "1700000000000,1,2,3,4,5,1700000899999,6,7,8,9";
        assert!(parse_klines(row).is_err());
    }

    // ---- AC-7: CHECKSUM verification -------------------------------------

    #[test]
    fn checksum_matches_for_correct_digest() {
        // SHA256 of the literal bytes "abc".
        let body = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  x.zip";
        assert!(verify_checksum(b"abc", body).is_ok());
    }

    #[test]
    fn checksum_mismatch_is_io_error() {
        let body = "0000000000000000000000000000000000000000000000000000000000000000  x.zip";
        let err = verify_checksum(b"abc", body).expect_err("mismatch rejects");
        assert!(matches!(err, crate::domain::DataError::Io(_)));
    }

    #[test]
    fn unzip_then_parse_round_trips() {
        // Build a tiny zip in memory holding the headerless CSV row, then
        // round-trip it through unzip + parse.
        let zipped = make_zip("BTCUSDT-15m-2024-01.csv", HEADERLESS_ROW);
        let csv = unzip_single_csv(&zipped).expect("unzip");
        let candles = parse_klines(&csv).expect("parse");
        assert_eq!(candles.len(), 1);
        assert_eq!(candles[0].open_time, 1_700_000_000_000);
    }

    /// Build an in-memory `.zip` with one named member (test helper).
    fn make_zip(name: &str, body: &str) -> Vec<u8> {
        use std::io::Write;
        use zip::write::SimpleFileOptions;
        let mut buf = Vec::new();
        {
            let cursor = std::io::Cursor::new(&mut buf);
            let mut writer = zip::ZipWriter::new(cursor);
            writer
                .start_file(name, SimpleFileOptions::default())
                .unwrap();
            writer.write_all(body.as_bytes()).unwrap();
            writer.finish().unwrap();
        }
        buf
    }
}
