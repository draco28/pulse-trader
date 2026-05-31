//! Integration tests for the REST incremental top-up (WI-1.1.1.03), driven
//! entirely over recorded REST JSON fixtures + a deterministic `FakeClock` — NO
//! live network (spec §3 "recorded REST JSON fixtures + a `FakeClock`").
//!
//! These exercise the library's *public* surface (`top_up_with`, `TopUpBoundary`,
//! `PageSource`, `FakeClock`, plus `CandleStore` for the new-`data_version`
//! proof), covering the demo's "second run fetches only new candles" clause:
//! - AC-1: the still-forming final kline is dropped (cutoff via `FakeClock`).
//! - AC-2: new candles append, dedup on `open_time`, merged series validates.
//! - AC-3: a second top-up with no newly-closed data is byte-identical.
//! - AC-4: funding fetched from `last_applied + 1`, stamped on NEW candles only.
//! - AC-5: forward pagination terminates at the cutoff.
//! - AC-1/C1: the topped-up series persists under a NEW `data_version`; the old
//!   (shorter) snapshot is retained on disk (immutable, never appended in place).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::future::Future;
use std::str::FromStr;

use pulse::{
    Candle, CandleSeries, CandleStore, DataError, DataVersion, FakeClock, Gap, PageSource, Pair,
    Timeframe, TopUpBoundary, top_up_with,
};
use rust_decimal::Decimal;
use tempfile::TempDir;

const M15: i64 = 900_000;

// The prior snapshot's three M15 candles (the bulk fixture's timestamps).
const PRIOR_LAST_OPEN: i64 = 1_700_001_800_000;
// The two newly-closed candles + the still-forming one.
const NEW_CLOSED_1: i64 = 1_700_002_700_000;
const NEW_CLOSED_2: i64 = 1_700_003_600_000;
const STILL_FORMING: i64 = 1_700_004_500_000;
// The last funding event already applied to the prior snapshot (bulk fixture).
const LAST_APPLIED_FUNDING: i64 = 1_700_000_900_000;

const KLINES_PAGE1: &[u8] = include_bytes!("fixtures/binance/rest_klines_page1.json");
const KLINES_EMPTY: &[u8] = include_bytes!("fixtures/binance/rest_klines_page2_empty.json");
const FUNDING_PAGE1: &[u8] = include_bytes!("fixtures/binance/rest_funding_page1.json");
const FUNDING_EMPTY: &[u8] = include_bytes!("fixtures/binance/rest_funding_empty.json");

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn btc() -> Pair {
    Pair::new("BTCUSDT")
}

fn candle(open_time: i64, funding: Option<&str>) -> Candle {
    Candle {
        open_time,
        close_time: open_time + M15 - 1,
        open: dec("42000.5"),
        high: dec("42100.0"),
        low: dec("41950.25"),
        close: dec("42050.75"),
        volume: dec("12.34567"),
        funding_rate: funding.map(dec),
    }
}

/// The prior snapshot: three contiguous M15 candles, the last funding event
/// landing on the 2nd (matching the bulk fixture's on-boundary event).
fn prior_series() -> CandleSeries {
    CandleSeries {
        pair: btc(),
        timeframe: Timeframe::M15,
        version: DataVersion::new("prior"),
        candles: vec![
            candle(1_700_000_000_000, None),
            candle(LAST_APPLIED_FUNDING, Some("0.00010000")),
            candle(PRIOR_LAST_OPEN, None),
        ],
    }
}

/// A fixture `PageSource` keyed by URL **substring** (so the test does not
/// hard-code the full host/query): klines pages are keyed by their `startTime`,
/// funding by its `startTime`. Recorded JSON bodies stand in for the live REST
/// responses (spec §3 — no network in the default run).
struct FixtureRest {
    by_marker: HashMap<&'static str, &'static [u8]>,
}

impl FixtureRest {
    fn second_run() -> Self {
        // First (and only) run that discovers new candles.
        let mut m: HashMap<&'static str, &'static [u8]> = HashMap::new();
        // klines page starting just past the prior boundary → the recorded page.
        m.insert("klines:startTime=1700001800001", KLINES_PAGE1);
        // klines page after advancing past the last open_time → empty (caught up).
        m.insert("klines:startTime=1700004500001", KLINES_EMPTY);
        // funding from last-applied + 1.
        m.insert("fundingRate:startTime=1700000900001", FUNDING_PAGE1);
        Self { by_marker: m }
    }

    fn caught_up() -> Self {
        // An idempotent re-run: the snapshot already holds the two closed
        // candles, so the first klines page returns only the still-forming one
        // (dropped), and there is no newly-closed data.
        let mut m: HashMap<&'static str, &'static [u8]> = HashMap::new();
        // The new boundary is NEW_CLOSED_2; the only thing beyond it is the
        // still-forming candle.
        m.insert("klines:startTime=1700003600001", KLINES_STILL_FORMING_ONLY);
        m.insert("klines:startTime=1700004500001", KLINES_EMPTY);
        m.insert("fundingRate:startTime=1700002700001", FUNDING_EMPTY);
        Self { by_marker: m }
    }

    fn marker_for(url: &str) -> String {
        let endpoint = if url.contains("/fapi/v1/fundingRate") {
            "fundingRate"
        } else {
            "klines"
        };
        let start = url
            .split("startTime=")
            .nth(1)
            .and_then(|s| s.split('&').next())
            .unwrap_or("?");
        format!("{endpoint}:startTime={start}")
    }
}

// A klines page holding only the still-forming candle (for the idempotent run).
const KLINES_STILL_FORMING_ONLY: &[u8] = br#"[[1700004500000,"42210.55","42230.00","42190.00","42205.00","1.05000",1700005399999,"0",3,"0","0","0"]]"#;

impl PageSource for FixtureRest {
    fn get(&self, url: &str) -> impl Future<Output = Result<Vec<u8>, DataError>> + Send {
        let marker = Self::marker_for(url);
        let body = self.by_marker.get(marker.as_str()).map(|b| b.to_vec());
        async move { body.ok_or_else(|| DataError::Io(format!("unscripted REST URL: {url}"))) }
    }
}

fn boundary(last_open: i64, last_funding: i64) -> TopUpBoundary {
    TopUpBoundary {
        last_open_ms: last_open,
        last_applied_funding_ms: last_funding,
    }
}

// ---- AC-1 + AC-2 + AC-4 + AC-5: the demo's "second run" path --------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn second_run_fetches_only_new_closed_candles_and_stamps_funding() {
    let prior = prior_series();
    // FakeClock now = the still-forming candle's open: its close_time
    // (STILL_FORMING + M15 - 1) is >= now, so the cutoff drops it (AC-1, NFR-2).
    let clock = FakeClock::at(STILL_FORMING);
    let source = FixtureRest::second_run();

    let (merged, gaps): (CandleSeries, Vec<Gap>) = top_up_with(
        &source,
        &clock,
        &prior,
        boundary(PRIOR_LAST_OPEN, LAST_APPLIED_FUNDING),
    )
    .await
    .expect("top-up over fixtures");

    // AC-1/AC-5: only the two CLOSED new candles were persisted; the forming
    // kline was dropped and pagination terminated cleanly.
    let times: Vec<i64> = merged.candles.iter().map(|c| c.open_time).collect();
    assert_eq!(
        times,
        vec![
            1_700_000_000_000,
            LAST_APPLIED_FUNDING,
            PRIOR_LAST_OPEN,
            NEW_CLOSED_1,
            NEW_CLOSED_2,
        ],
        "the still-forming candle ({STILL_FORMING}) must NOT be written"
    );
    // AC-2: contiguous merge validates with no gaps.
    assert!(gaps.is_empty(), "contiguous top-up: {gaps:?}");

    // AC-4: funding fetched from last-applied + 1 and stamped on the NEW candle
    // it lands on only (no double-application onto the prior boundary candle).
    assert_eq!(
        merged.candles[1].funding_rate,
        Some(dec("0.00010000")),
        "prior funding untouched"
    );
    assert_eq!(
        merged.candles[3].funding_rate,
        Some(dec("0.00012500")),
        "new funding stamped on the new candle at its calc_time"
    );
    assert_eq!(merged.candles[4].funding_rate, None, "no forward-fill");
}

// ---- AC-1/C1: the top-up persists under a NEW data_version; old retained ---

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn topped_up_series_writes_a_new_data_version_old_retained() {
    let tmp = TempDir::new().expect("tempdir");
    let store = CandleStore::with_base_dir(tmp.path().to_path_buf());

    // Persist the prior snapshot under its content version.
    let mut prior = prior_series();
    prior.version = CandleStore::content_version(&prior.pair, prior.timeframe, &prior.candles);
    store.write_snapshot(&prior).expect("write prior");

    // Top up, then re-derive the content version for the merged series (C1).
    let clock = FakeClock::at(STILL_FORMING);
    let source = FixtureRest::second_run();
    let (mut merged, _) = top_up_with(
        &source,
        &clock,
        &prior,
        boundary(PRIOR_LAST_OPEN, LAST_APPLIED_FUNDING),
    )
    .await
    .expect("top-up");
    merged.version = CandleStore::content_version(&merged.pair, merged.timeframe, &merged.candles);
    store.write_snapshot(&merged).expect("write merged");

    // C1: a top-up that adds candles yields a NEW data_version.
    assert_ne!(
        prior.version, merged.version,
        "adding candles must mint a new content version"
    );
    // Both snapshots coexist on disk — the prior is immutable, never appended.
    assert!(
        store.snapshot_exists(&prior.pair, prior.timeframe, &prior.version),
        "prior (shorter) snapshot retained"
    );
    assert!(
        store.snapshot_exists(&merged.pair, merged.timeframe, &merged.version),
        "new merged snapshot written"
    );
    // The merged snapshot reads back byte-identical.
    let back = store
        .read_snapshot(&merged.pair, merged.timeframe, &merged.version)
        .expect("read merged");
    assert_eq!(back, merged);
}

// ---- AC-3: idempotent — a second top-up with no newly-closed data is no-op --

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idempotent_second_top_up_is_byte_identical() {
    // Start from the already-topped-up state: prior + the two closed candles.
    let mut state = prior_series();
    state.candles.push(candle(NEW_CLOSED_1, Some("0.00012500")));
    state.candles.push(candle(NEW_CLOSED_2, None));
    state.version = CandleStore::content_version(&state.pair, state.timeframe, &state.candles);

    // Re-run: the only candle beyond NEW_CLOSED_2 is still forming (dropped), so
    // zero new candles are merged. Funding from NEW_CLOSED_1 + 1 returns empty.
    let clock = FakeClock::at(STILL_FORMING);
    let source = FixtureRest::caught_up();
    let (merged, gaps) = top_up_with(
        &source,
        &clock,
        &state,
        boundary(NEW_CLOSED_2, NEW_CLOSED_1),
    )
    .await
    .expect("idempotent top-up");

    // AC-3: zero candles added, series byte-identical (incl. the version, since
    // the content hash is a pure function of the unchanged candle vector).
    let merged_version =
        CandleStore::content_version(&merged.pair, merged.timeframe, &merged.candles);
    assert_eq!(merged.candles.len(), state.candles.len(), "zero added");
    assert_eq!(merged, state, "byte-identical series");
    assert_eq!(
        merged_version, state.version,
        "same content version ⇒ no new version"
    );
    assert!(gaps.is_empty());
}
