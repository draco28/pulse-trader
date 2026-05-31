//! `Binance` data adapter (WI-1.1.1.02): bulk historical ingest from
//! `data.binance.vision`.
//!
//! Layering inside the adapter:
//! - [`client`] — the `reqwest` transport wrapper with bounded retry/backoff and
//!   transport-error mapping to [`crate::domain::DataError`] (AC-5).
//! - [`bulk`] — archive URL construction, `.CHECKSUM` (SHA256) verification,
//!   unzip, and 12-column USD-M kline CSV parsing with header detection
//!   (AC-1, AC-2, AC-7, AC-8).
//! - [`normalize`] — deterministic sort + dedup into a [`crate::domain::CandleSeries`]
//!   with gap *reporting* (never gap filling) via
//!   [`crate::domain::CandleSeries::validate`] (AC-3).
//! - [`funding`] — funding-rate CSV parsing and sparse, per-event stamping onto
//!   the single candle whose half-open `[open_time, close_time)` interval
//!   contains the funding timestamp (AC-4).
//!
//! NFR-9: the concrete `MarketDataSource` impl that stitches these together is
//! wired by WI-05; this work item delivers the building blocks plus the
//! [`ingest_window`] orchestration that produces the normalized,
//! funding-bearing, gap-checked series (NFR-2) and disambiguates a
//! pre-listing-month `404` from a transient miss (audit C2).

pub(crate) mod bulk;
pub(crate) mod client;
pub(crate) mod funding;
pub(crate) mod normalize;

use std::future::Future;

use crate::domain::{Candle, CandleSeries, DataError, DataVersion, Gap, Pair, Timeframe};

use bulk::{KlineArchive, archive_urls, parse_klines, unzip_single_csv, verify_checksum};
use client::BinanceClient;
use funding::{parse_funding, stamp_funding};
use normalize::normalize;

pub use funding::FundingEvent;

/// One verified, parsed calendar month of klines + funding (the unit the bulk
/// window assembles).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonthData {
    /// Klines parsed from the month's verified archive.
    pub candles: Vec<Candle>,
    /// Funding events parsed from the month's verified archive.
    pub funding: Vec<FundingEvent>,
}

/// Outcome of attempting to load a single month.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MonthOutcome {
    /// The month's archives were fetched, checksum-verified, and parsed.
    Loaded(MonthData),
    /// The month's archive is legitimately absent (a `404`) — e.g. a month
    /// before the pair was listed. Recorded as out-of-range, NOT a [`Gap`]
    /// (audit C2).
    Absent,
}

/// Per-month archive loader — the seam the bulk window calls once per calendar
/// month. The production impl downloads + checksum-verifies + parses; tests
/// inject fixture months offline (AC-6/AC-7).
pub trait MonthSource {
    /// Load one calendar month. `Ok(Absent)` signals a legitimate `404`;
    /// `Err(DataError::Io)` signals a transient failure that survived retries.
    fn load_month(
        &self,
        pair: &Pair,
        tf: Timeframe,
        year: i32,
        month: u32,
    ) -> impl Future<Output = Result<MonthOutcome, DataError>> + Send;
}

/// Ingest a contiguous window of calendar months into one normalized,
/// funding-bearing, gap-checked [`CandleSeries`] (AC-3/AC-4/AC-7, NFR-2).
///
/// `months` is an ordered list of `(year, month)` pairs (the window). For each
/// month the [`MonthSource`] is consulted:
/// - `Loaded` months contribute their candles + funding.
/// - `Absent` (a `404`) months are disambiguated (audit C2): if **no earlier
///   month has loaded yet**, the absence is treated as pre-listing/out-of-range
///   and skipped; once some month has loaded, a later `Absent` is a *gap in
///   coverage* surfaced as [`DataError::Io`] (the transient-miss path — the
///   transport already retried before the source returned `Absent`-vs-error,
///   so reaching here on an expected month is terminal).
///
/// After accumulation: [`normalize`] sorts+dedups and reports gaps via `Ok`;
/// [`stamp_funding`] applies the sparse half-open funding alignment.
///
/// # Errors
///
/// Propagates [`DataError`] from the source (transient failure) or from
/// normalization (structural corruption — not expected post-normalize).
pub async fn ingest_window<S: MonthSource + Sync>(
    source: &S,
    pair: &Pair,
    tf: Timeframe,
    version: &DataVersion,
    months: &[(i32, u32)],
) -> Result<(CandleSeries, Vec<Gap>), DataError> {
    let mut all_candles: Vec<Candle> = Vec::new();
    let mut all_funding: Vec<FundingEvent> = Vec::new();
    let mut any_loaded = false;

    for &(year, month) in months {
        match source.load_month(pair, tf, year, month).await? {
            MonthOutcome::Loaded(data) => {
                any_loaded = true;
                all_candles.extend(data.candles);
                all_funding.extend(data.funding);
            }
            MonthOutcome::Absent => {
                if any_loaded {
                    // A month inside the listed range is missing: out-of-range
                    // disambiguation says this is a real coverage hole, not a
                    // pre-listing skip (audit C2).
                    return Err(DataError::Io(format!(
                        "expected month {year:04}-{month:02} absent after the pair was listed"
                    )));
                }
                // else: pre-listing month → skip, NOT a Gap.
            }
        }
    }

    let (mut series, gaps) = normalize(pair, tf, version, all_candles)?;
    stamp_funding(&mut series.candles, &all_funding);
    Ok((series, gaps))
}

/// Decode one month's already-downloaded `.zip` archives into a [`MonthData`]:
/// unzip the single CSV member and parse the 12-column klines (AC-2/AC-8) plus
/// the optional funding CSV (AC-4). Checksum verification happens at *fetch*
/// time ([`BulkMonthSource`]); this is the pure decode half, isolated so it can
/// be exercised offline over recorded fixtures (AC-6).
///
/// # Errors
///
/// [`DataError::Parse`] / [`DataError::Io`] if an archive is not a valid zip or
/// a CSV row does not match the pinned schema.
pub fn decode_month(klines_zip: &[u8], funding_zip: Option<&[u8]>) -> Result<MonthData, DataError> {
    let candles = parse_klines(&unzip_single_csv(klines_zip)?)?;
    let funding = match funding_zip {
        Some(bytes) => parse_funding(&unzip_single_csv(bytes)?)?,
        None => Vec::new(),
    };
    Ok(MonthData { candles, funding })
}

/// Verify an archive against its `.CHECKSUM` sidecar body (SHA256), exposed for
/// fixture-level integrity tests (AC-7). Delegates to [`bulk::verify_checksum`].
///
/// # Errors
///
/// [`DataError::Io`] on digest mismatch; [`DataError::Parse`] on an empty
/// sidecar.
pub fn verify_archive_checksum(archive_bytes: &[u8], checksum_body: &str) -> Result<(), DataError> {
    verify_checksum(archive_bytes, checksum_body)
}

/// Convenience entrypoint: ingest a month window over the live bulk source
/// (`data.binance.vision`), producing a normalized, funding-bearing,
/// gap-checked [`CandleSeries`] plus its reported gaps (the WI-05 seam, NFR-2).
///
/// This is the only path that touches the network; tests drive
/// [`ingest_window`] with a fixture [`MonthSource`] instead (AC-6).
///
/// # Errors
///
/// [`DataError`] if the HTTP client cannot be built, a month transiently fails
/// after retries, a `.CHECKSUM` mismatches, or an archive cannot be parsed.
pub async fn ingest_bulk(
    pair: &Pair,
    tf: Timeframe,
    version: &DataVersion,
    months: &[(i32, u32)],
) -> Result<(CandleSeries, Vec<Gap>), DataError> {
    let source = BulkMonthSource::new()?;
    ingest_window(&source, pair, tf, version, months).await
}

/// Production [`MonthSource`]: downloads each month's klines + funding archive
/// from `data.binance.vision` via a [`BinanceClient`], verifies each against its
/// `.CHECKSUM` (SHA256) before parsing, and parses the 12-column klines + the
/// funding CSV (AC-1/AC-2/AC-7/AC-8). A `404` on the klines archive maps to
/// [`MonthOutcome::Absent`] so the window can disambiguate pre-listing vs.
/// transient (audit C2).
pub struct BulkMonthSource {
    client: BinanceClient,
}

impl BulkMonthSource {
    /// Build a source over a fresh [`BinanceClient`].
    ///
    /// # Errors
    ///
    /// [`DataError::Io`] if the HTTP client cannot be built.
    pub fn new() -> Result<Self, DataError> {
        Ok(Self {
            client: BinanceClient::new()?,
        })
    }

    /// Fetch an archive, verify it against its checksum sidecar, and return the
    /// archive bytes. `Ok(None)` iff the archive itself `404`s (absence).
    async fn fetch_verified(&self, arc: &KlineArchive) -> Result<Option<Vec<u8>>, DataError> {
        let Some(bytes) = self.client.fetch_optional(&arc.archive).await? else {
            return Ok(None);
        };
        let checksum_raw = self.client.fetch(&arc.checksum).await?;
        let checksum_body = String::from_utf8(checksum_raw)
            .map_err(|e| DataError::Parse(format!("non-UTF8 .CHECKSUM body: {e}")))?;
        verify_checksum(&bytes, &checksum_body)?;
        Ok(Some(bytes))
    }
}

impl MonthSource for BulkMonthSource {
    fn load_month(
        &self,
        pair: &Pair,
        tf: Timeframe,
        year: i32,
        month: u32,
    ) -> impl Future<Output = Result<MonthOutcome, DataError>> + Send {
        let (klines_arc, funding_arc) = archive_urls(pair, tf, year, month);
        async move {
            // Klines are the authority for month presence: a 404 here is the
            // "absent" signal the window disambiguates.
            let Some(klines_zip) = self.fetch_verified(&klines_arc).await? else {
                return Ok(MonthOutcome::Absent);
            };
            let candles = parse_klines(&unzip_single_csv(&klines_zip)?)?;

            // Funding may be independently absent for a present klines month;
            // treat its 404 as "no funding events", not a month absence.
            let funding = match self.fetch_verified(&funding_arc).await? {
                Some(funding_zip) => parse_funding(&unzip_single_csv(&funding_zip)?)?,
                None => Vec::new(),
            };

            Ok(MonthOutcome::Loaded(MonthData { candles, funding }))
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{FundingEvent, MonthData, MonthOutcome, MonthSource, ingest_window};
    use crate::domain::DataVersion;
    use crate::domain::{Candle, DataError, Pair, Timeframe};
    use rust_decimal::Decimal;
    use std::collections::HashMap;
    use std::future::Future;

    fn candle(open_time: i64) -> Candle {
        Candle {
            open_time,
            close_time: open_time + 14_400_000,
            open: Decimal::ONE,
            high: Decimal::ONE,
            low: Decimal::ONE,
            close: Decimal::ONE,
            volume: Decimal::ONE,
            funding_rate: None,
        }
    }

    /// A scripted month source: maps `(year, month)` → outcome.
    struct ScriptedSource {
        script: HashMap<(i32, u32), Result<MonthOutcome, ()>>,
    }

    impl MonthSource for ScriptedSource {
        fn load_month(
            &self,
            _pair: &Pair,
            _tf: Timeframe,
            year: i32,
            month: u32,
        ) -> impl Future<Output = Result<MonthOutcome, DataError>> + Send {
            let entry = self.script.get(&(year, month)).cloned();
            async move {
                match entry {
                    Some(Ok(outcome)) => Ok(outcome),
                    Some(Err(())) => Err(DataError::Io("transient".into())),
                    None => Ok(MonthOutcome::Absent),
                }
            }
        }
    }

    fn version() -> DataVersion {
        DataVersion::new("v1")
    }

    // ---- AC-7: pre-listing 404 skipped (NOT a gap) ------------------------

    #[tokio::test]
    async fn pre_listing_absent_months_are_skipped_not_a_gap() {
        // Window of 3 months; the first two are pre-listing (Absent), the third
        // loads two contiguous H4 candles.
        let mut script = HashMap::new();
        script.insert((2019, 8), Ok(MonthOutcome::Absent));
        script.insert((2019, 9), Ok(MonthOutcome::Absent));
        script.insert(
            (2019, 10),
            Ok(MonthOutcome::Loaded(MonthData {
                candles: vec![candle(0), candle(14_400_000)],
                funding: vec![],
            })),
        );
        let source = ScriptedSource { script };
        let (series, gaps) = ingest_window(
            &source,
            &Pair::new("BTCUSDT"),
            Timeframe::H4,
            &version(),
            &[(2019, 8), (2019, 9), (2019, 10)],
        )
        .await
        .expect("pre-listing skip succeeds");
        assert_eq!(series.candles.len(), 2);
        assert!(
            gaps.is_empty(),
            "pre-listing absence is NOT a gap: {gaps:?}"
        );
    }

    // ---- AC-7: transient absence on an expected month surfaces Io ---------

    #[tokio::test]
    async fn absent_month_after_a_loaded_month_is_io_error() {
        let mut script = HashMap::new();
        script.insert(
            (2024, 1),
            Ok(MonthOutcome::Loaded(MonthData {
                candles: vec![candle(0)],
                funding: vec![],
            })),
        );
        script.insert((2024, 2), Ok(MonthOutcome::Absent));
        let source = ScriptedSource { script };
        let err = ingest_window(
            &source,
            &Pair::new("BTCUSDT"),
            Timeframe::H4,
            &version(),
            &[(2024, 1), (2024, 2)],
        )
        .await
        .expect_err("expected month absent after listing → Io");
        assert!(matches!(err, DataError::Io(_)));
    }

    #[tokio::test]
    async fn transient_failure_propagates_as_io() {
        let mut script = HashMap::new();
        script.insert((2024, 1), Err(()));
        let source = ScriptedSource { script };
        let err = ingest_window(
            &source,
            &Pair::new("BTCUSDT"),
            Timeframe::H4,
            &version(),
            &[(2024, 1)],
        )
        .await
        .expect_err("transient → Io");
        assert!(matches!(err, DataError::Io(_)));
    }

    // ---- End-to-end: window normalizes + stamps funding -------------------

    #[tokio::test]
    async fn window_normalizes_and_stamps_funding_across_months() {
        let mut script = HashMap::new();
        script.insert(
            (2024, 1),
            Ok(MonthOutcome::Loaded(MonthData {
                // Out-of-order across the month boundary on purpose.
                candles: vec![candle(14_400_000), candle(0)],
                funding: vec![FundingEvent {
                    calc_time: 0, // on-boundary: stamps candle(0)
                    rate: Decimal::new(1, 4),
                }],
            })),
        );
        let source = ScriptedSource { script };
        let (series, gaps) = ingest_window(
            &source,
            &Pair::new("BTCUSDT"),
            Timeframe::H4,
            &version(),
            &[(2024, 1)],
        )
        .await
        .expect("window ingests");
        // Sorted ascending.
        assert_eq!(series.candles[0].open_time, 0);
        assert_eq!(series.candles[1].open_time, 14_400_000);
        // Funding stamped on the on-boundary candle only (sparse).
        assert_eq!(series.candles[0].funding_rate, Some(Decimal::new(1, 4)));
        assert_eq!(series.candles[1].funding_rate, None);
        assert!(gaps.is_empty());
    }
}
