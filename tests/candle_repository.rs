//! r1.s3.w1 (#112) — the `CandleSeriesRepository` port/adapter contract target.
//!
//! ADR-0015 named exactly one hexagonal exception: candle storage had no port, so
//! the use-case modules imported and constructed the concrete `CandleStore`. This
//! target is what closes it. It proves four things at once:
//!
//! 1. **The port is sufficient.** A zero-I/O in-memory double implementing ONLY
//!    `load_head` / `load_version` / `commit` satisfies the same contract exercise
//!    the real Parquet adapter does — so a caller needs nothing else from the seam.
//! 2. **`commit` owns identity.** It takes pair + timeframe + candles and derives
//!    ADR-0009's canonical content hash itself; a mismatched caller identity is
//!    unrepresentable because there is no version parameter to state one in.
//! 3. **The persisted contract survives.** Immutable snapshots, snapshot-before-`HEAD`
//!    ordering, exact-version reads that never fall back to `HEAD`, the zero-candle
//!    no-file/no-`HEAD` outcome, and the absolute debug locator all still hold.
//! 4. **The boundary is real, not aspirational.** The three consumer modules name no
//!    concrete store in CODE (comments blanked first), the engine names neither the
//!    adapter nor the port, and `src/cli/mod.rs` remains the one construction site —
//!    ADR-0015's explicit composition-root exception.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;

use pulse::{
    Candle, CandleSeries, CandleSeriesRepository, CandleStore, DataError, DataVersion, Pair,
    StoredCandleSeries, Timeframe,
};
use rust_decimal::Decimal;
use tempfile::TempDir;

mod source_scan;
use source_scan::{blank_comments, read_source};

/// One M15 bar in milliseconds.
const M15_MS: i64 = 900_000;

fn btc() -> Pair {
    Pair::new("BTCUSDT")
}

/// A contiguous, gap-free M15 candle run — `count` bars starting at epoch 0, with
/// `close` walking upward so two different counts hash to two different versions.
fn candles(count: i64) -> Vec<Candle> {
    (0..count)
        .map(|i| {
            let price = Decimal::from(100 + i);
            Candle {
                open_time: i * M15_MS,
                close_time: i * M15_MS + M15_MS - 1,
                open: price,
                high: price,
                low: price,
                close: price,
                volume: Decimal::ONE,
                funding_rate: None,
            }
        })
        .collect()
}

/// A real Parquet-backed store rooted in a tempdir (never real Application Support).
fn store() -> (CandleStore, TempDir) {
    let tmp = tempfile::tempdir().expect("create tempdir for the candle store");
    let store = CandleStore::with_base_dir(tmp.path().to_path_buf());
    (store, tmp)
}

// ---------------------------------------------------------------------------
// The zero-I/O double: the port's whole surface, and nothing else.
// ---------------------------------------------------------------------------

/// An in-memory `CandleSeriesRepository`. It touches no filesystem, holds no
/// `CandleStore`, and implements exactly the three port methods — so a consumer
/// that compiles against it is proven to need only the port.
#[derive(Default)]
struct InMemoryRepo {
    snapshots: RefCell<HashMap<(Pair, Timeframe, DataVersion), CandleSeries>>,
    heads: RefCell<HashMap<(Pair, Timeframe), DataVersion>>,
}

impl InMemoryRepo {
    /// The double's own deterministic identity derivation — content in, tag out.
    /// It is deliberately NOT the Parquet adapter's hash: the port promises that
    /// `commit` derives identity, not which scheme an adapter derives it with.
    fn derive_version(candles: &[Candle]) -> DataVersion {
        match (candles.first(), candles.last()) {
            (Some(first), Some(last)) => DataVersion::new(format!(
                "mem-{}-{}-{}-{}",
                candles.len(),
                first.open_time,
                last.open_time,
                last.close
            )),
            _ => DataVersion::new("mem-empty"),
        }
    }
}

impl CandleSeriesRepository for InMemoryRepo {
    fn load_head(
        &self,
        pair: &Pair,
        timeframe: Timeframe,
    ) -> Result<Option<StoredCandleSeries>, DataError> {
        let head = self.heads.borrow().get(&(pair.clone(), timeframe)).cloned();
        match head {
            // A HEAD naming an absent snapshot is an error, never `Ok(None)`.
            Some(version) => self.load_version(pair, timeframe, &version).map(Some),
            None => Ok(None),
        }
    }

    fn load_version(
        &self,
        pair: &Pair,
        timeframe: Timeframe,
        version: &DataVersion,
    ) -> Result<StoredCandleSeries, DataError> {
        let key = (pair.clone(), timeframe, version.clone());
        let series = self
            .snapshots
            .borrow()
            .get(&key)
            .cloned()
            .ok_or_else(|| DataError::Io(format!("no in-memory snapshot for {version}")))?;
        Ok(StoredCandleSeries {
            storage_location: Some(format!(
                "memory://{pair}/{}/{version}",
                timeframe.binance_interval()
            )),
            series,
        })
    }

    fn commit(
        &self,
        pair: &Pair,
        timeframe: Timeframe,
        candles: Vec<Candle>,
    ) -> Result<StoredCandleSeries, DataError> {
        let version = Self::derive_version(&candles);
        let series = CandleSeries {
            pair: pair.clone(),
            timeframe,
            version,
            candles,
        };
        series.validate()?;
        if series.candles.is_empty() {
            // Zero candles: write nothing, leave HEAD alone, report no location.
            return Ok(StoredCandleSeries {
                series,
                storage_location: None,
            });
        }
        let location = format!(
            "memory://{pair}/{}/{}",
            timeframe.binance_interval(),
            series.version
        );
        // Snapshot FIRST, HEAD SECOND — the same ordering the real adapter keeps.
        self.snapshots.borrow_mut().insert(
            (pair.clone(), timeframe, series.version.clone()),
            series.clone(),
        );
        self.heads
            .borrow_mut()
            .insert((pair.clone(), timeframe), series.version.clone());
        Ok(StoredCandleSeries {
            series,
            storage_location: Some(location),
        })
    }
}

// ---------------------------------------------------------------------------
// The generic contract exercise — driven through the port bound alone.
// ---------------------------------------------------------------------------

/// Exercise the port's whole semantic contract using nothing but `R`'s three
/// methods. Both the zero-I/O double and the real Parquet adapter must pass it.
fn exercise_port_contract<R: CandleSeriesRepository>(repo: &R) {
    let pair = btc();
    let tf = Timeframe::M15;

    assert!(
        repo.load_head(&pair, tf)
            .expect("load_head on an empty repository")
            .is_none(),
        "no HEAD yet ⇒ Ok(None)"
    );

    // --- first commit: derived identity, snapshot + HEAD, full round trip -----
    let first = candles(8);
    let committed = repo
        .commit(&pair, tf, first.clone())
        .expect("commit the first series");
    assert_eq!(committed.series.pair, pair);
    assert_eq!(committed.series.timeframe, tf);
    assert_eq!(committed.series.candles, first);
    assert!(
        committed.storage_location.is_some(),
        "a persisted snapshot reports a locator"
    );

    let head = repo
        .load_head(&pair, tf)
        .expect("load_head after the first commit")
        .expect("HEAD is present once a snapshot exists");
    assert_eq!(
        head.series, committed.series,
        "HEAD reloads the WHOLE committed series, not just its identity"
    );

    let exact = repo
        .load_version(&pair, tf, &committed.series.version)
        .expect("load_version for the committed identity");
    assert_eq!(
        exact.series, committed.series,
        "the exact-version read returns the same series HEAD does"
    );

    // --- second, distinct commit: HEAD advances, the first stays immutable ----
    let second = candles(12);
    let advanced = repo
        .commit(&pair, tf, second.clone())
        .expect("commit a second, distinct series");
    assert_ne!(
        advanced.series.version, committed.series.version,
        "distinct content ⇒ distinct derived identity"
    );

    let advanced_head = repo
        .load_head(&pair, tf)
        .expect("load_head after the second commit")
        .expect("HEAD is still present");
    assert_eq!(
        advanced_head.series.version, advanced.series.version,
        "HEAD advanced to the second snapshot"
    );
    assert_eq!(advanced_head.series.candles, second);

    let still_there = repo
        .load_version(&pair, tf, &committed.series.version)
        .expect("the first version is still loadable after HEAD moved");
    assert_eq!(
        still_there.series, committed.series,
        "the first snapshot is immutable — HEAD moving did not rewrite it"
    );

    // --- the zero-candle outcome: no write, no HEAD move, no location ---------
    let empty = repo
        .commit(&pair, tf, Vec::new())
        .expect("committing zero candles is a success, not an error");
    assert!(
        empty.storage_location.is_none(),
        "no snapshot exists for a zero-candle commit, so there is no locator"
    );
    assert!(empty.series.candles.is_empty());

    let head_after_empty = repo
        .load_head(&pair, tf)
        .expect("load_head after the empty commit")
        .expect("HEAD survived the empty commit");
    assert_eq!(
        head_after_empty.series.version, advanced.series.version,
        "an empty commit leaves HEAD exactly where it was"
    );
}

#[test]
fn a_zero_io_double_satisfies_the_port_contract() {
    exercise_port_contract(&InMemoryRepo::default());
}

#[test]
fn the_real_parquet_adapter_satisfies_the_port_contract() {
    let (store, _tmp) = store();
    exercise_port_contract(&store);
}

// ---------------------------------------------------------------------------
// Real-adapter specifics: canonical identity, the locator, and the file effects.
// ---------------------------------------------------------------------------

#[test]
fn commit_derives_the_canonical_content_identity() {
    let (store, _tmp) = store();
    let pair = btc();
    let tf = Timeframe::M15;
    let bars = candles(6);

    // `commit` takes NO version argument — a caller cannot state a wrong identity.
    let stored = store.commit(&pair, tf, bars.clone()).expect("commit");
    let canonical = CandleStore::content_version(&pair, tf, &bars);

    assert_eq!(
        stored.series.version, canonical,
        "commit derives ADR-0009's canonical content hash"
    );
    assert_eq!(
        store
            .read_head(&pair, tf)
            .expect("read the raw HEAD pointer"),
        Some(canonical.clone()),
        "the persisted HEAD names that same canonical identity"
    );
    assert!(
        store.snapshot_exists(&pair, tf, &canonical),
        "the snapshot was published under the canonical identity"
    );
    assert_eq!(
        store
            .load_version(&pair, tf, &canonical)
            .expect("load the canonical version")
            .series
            .candles,
        bars,
        "the canonical identity reads back the committed candles"
    );
}

#[test]
fn commit_reports_the_absolute_snapshot_locator() {
    let (store, tmp) = store();
    let pair = btc();
    let tf = Timeframe::M15;

    let stored = store.commit(&pair, tf, candles(4)).expect("commit");
    let location = stored
        .storage_location
        .expect("a persisted snapshot reports a locator");
    let path = Path::new(&location);

    assert!(
        path.is_absolute(),
        "the debug locator stays absolute (ADR-0017): {location}"
    );
    assert!(
        path.is_file(),
        "the locator names the snapshot that was actually written: {location}"
    );
    assert!(
        path.starts_with(tmp.path()),
        "the locator points inside the injected store root: {location}"
    );
    assert_eq!(
        path,
        store.snapshot_path(&pair, tf, &stored.series.version),
        "the locator is the adapter's own snapshot path"
    );

    // Reads report the same locator the write did.
    let head = store
        .load_head(&pair, tf)
        .expect("load_head")
        .expect("HEAD present");
    assert_eq!(head.storage_location.as_deref(), Some(location.as_str()));
}

#[test]
fn an_empty_commit_writes_no_file_and_sets_no_head() {
    let (store, tmp) = store();
    let pair = btc();
    let tf = Timeframe::M15;

    let empty = store
        .commit(&pair, tf, Vec::new())
        .expect("committing zero candles succeeds");

    assert!(empty.storage_location.is_none());
    assert_eq!(
        store
            .read_head(&pair, tf)
            .expect("read the raw HEAD pointer"),
        None,
        "a zero-candle commit never sets HEAD"
    );
    assert!(
        !store.snapshot_exists(&pair, tf, &empty.series.version),
        "a zero-candle commit publishes no snapshot"
    );
    let tf_dir = tmp
        .path()
        .join("candles")
        .join(pair.as_str())
        .join(tf.binance_interval());
    assert!(
        !tf_dir.exists() || std::fs::read_dir(&tf_dir).expect("read tf dir").count() == 0,
        "a zero-candle commit leaves no file behind in {}",
        tf_dir.display()
    );
}

#[test]
fn a_head_naming_an_absent_snapshot_is_an_error_not_none() {
    let (store, _tmp) = store();
    let pair = btc();
    let tf = Timeframe::M15;

    store
        .write_head(&pair, tf, &DataVersion::new("0000000000000000"))
        .expect("write a dangling HEAD pointer");

    let err = store
        .load_head(&pair, tf)
        .expect_err("a HEAD naming an absent snapshot must error, never read as Ok(None)");
    assert!(
        matches!(err, DataError::Io(_)),
        "expected an IO error for the missing snapshot, got {err:?}"
    );
}

#[test]
fn load_version_never_falls_back_to_head() {
    let (store, _tmp) = store();
    let pair = btc();
    let tf = Timeframe::M15;

    let committed = store.commit(&pair, tf, candles(5)).expect("commit");

    let err = store
        .load_version(&pair, tf, &DataVersion::new("1111111111111111"))
        .expect_err("an unknown version must error rather than silently return HEAD");
    assert!(
        matches!(err, DataError::Io(_)),
        "expected an IO error for the unknown version, got {err:?}"
    );

    // HEAD itself is untouched by the failed exact read.
    assert_eq!(
        store
            .load_head(&pair, tf)
            .expect("load_head")
            .expect("HEAD present")
            .series
            .version,
        committed.series.version
    );
}

// ---------------------------------------------------------------------------
// Stored-integrity guards: a tag or file that came off disk is input, not truth.
// ---------------------------------------------------------------------------

/// A corrupted `HEAD` holding a traversal tag must be refused at the read, before
/// `load_head` can join it into a snapshot path that escapes the store root — and
/// the refusal is an error, never `Ok(None)` (which would read as "first run" and
/// trigger a silent full re-fetch).
#[test]
fn a_traversal_head_tag_is_refused_before_any_path_join() {
    let (store, _tmp) = store();
    let pair = btc();
    let tf = Timeframe::M15;
    store.commit(&pair, tf, candles(4)).expect("commit");

    // Hand-corrupt the pointer the way a stray edit or bad restore would.
    std::fs::write(store.head_path(&pair, tf), b"../../../tmp/other").expect("corrupt HEAD");

    let err = store
        .read_head(&pair, tf)
        .expect_err("a traversal HEAD tag must be refused, not returned unchecked");
    assert!(
        matches!(err, DataError::Parse(_)),
        "expected a Parse refusal for the unsafe tag, got {err:?}"
    );
    let err = store
        .load_head(&pair, tf)
        .expect_err("load_head must propagate the refusal — Ok(None) would mean 'first run'");
    assert!(matches!(err, DataError::Parse(_)), "{err:?}");
}

/// The same rule on the listing side: a directory entry whose stem is not a safe
/// single path component is skipped by `latest_version`, never surfaced as a tag.
#[test]
fn latest_version_skips_a_filename_that_is_not_a_safe_component() {
    let (store, _tmp) = store();
    let pair = btc();
    let tf = Timeframe::M15;
    let committed = store.commit(&pair, tf, candles(4)).expect("commit");

    let tf_dir = store
        .snapshot_path(&pair, tf, &committed.series.version)
        .parent()
        .unwrap()
        .to_path_buf();
    // A backslash stem is a legal POSIX filename but not a portable path
    // component; `latest_version` must report only the sane snapshot.
    std::fs::write(tf_dir.join("bad\\name.parquet"), b"junk").expect("plant unsafe filename");

    assert_eq!(
        store.latest_version(&pair, tf).expect("latest_version"),
        Some(committed.series.version),
        "the unsafe filename is skipped, the real snapshot still wins"
    );
}

/// A snapshot file replaced with another *valid* snapshot (wrong content under a
/// surviving name) must be refused, not decoded as the requested version — the
/// persisted-provenance flow would otherwise display and replay market data that
/// does not match the content hash it reports (closes #6).
#[test]
fn a_swapped_snapshot_file_is_refused_not_served() {
    let (store, _tmp) = store();
    let pair = btc();
    let tf = Timeframe::M15;
    let a = store.commit(&pair, tf, candles(5)).expect("commit A");
    let b = store.commit(&pair, tf, candles(9)).expect("commit B");

    // Swap: B's bytes now sit at A's content-addressed path.
    std::fs::copy(
        store.snapshot_path(&pair, tf, &b.series.version),
        store.snapshot_path(&pair, tf, &a.series.version),
    )
    .expect("swap the file");

    let err = store
        .load_version(&pair, tf, &a.series.version)
        .expect_err("a swapped snapshot must be refused, not served as A");
    assert!(
        matches!(err, DataError::Parse(ref msg) if msg.contains("embedded provenance")),
        "the refusal names the embedded-tag mismatch, got {err:?}"
    );
    // The untouched snapshot still reads: the guard verifies, it does not lock.
    store
        .load_version(&pair, tf, &b.series.version)
        .expect("the un-swapped snapshot still loads");
}

/// The deeper forgery: a file re-encoded so its embedded provenance *claims* the
/// requested tag while its candles hash elsewhere. The re-derived content hash —
/// the read-side twin of `write_snapshot`'s check — is what refuses it.
#[test]
fn an_edited_snapshot_with_forged_provenance_is_refused() {
    let (store, _tmp) = store();
    let pair = btc();
    let tf = Timeframe::M15;
    let a = store.commit(&pair, tf, candles(5)).expect("commit A");
    let b = store.commit(&pair, tf, candles(9)).expect("commit B");

    // B's candles wearing A's tag: `encode_snapshot` embeds `series.version`
    // verbatim, so the provenance lies exactly the way a tamperer would want.
    let forged = CandleSeries {
        pair: pair.clone(),
        timeframe: tf,
        version: a.series.version.clone(),
        candles: b.series.candles.clone(),
    };
    let bytes = store.encode_snapshot(&forged).expect("encode forged snapshot");
    std::fs::write(store.snapshot_path(&pair, tf, &a.series.version), bytes)
        .expect("plant forged snapshot");

    let err = store
        .read_snapshot(&pair, tf, &a.series.version)
        .expect_err("content that re-hashes elsewhere must be refused");
    assert!(
        matches!(err, DataError::Parse(ref msg) if msg.contains("re-derive")),
        "the refusal names the re-derived hash mismatch, got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// The boundary scan: code, not comments.
// ---------------------------------------------------------------------------

/// The three use-case modules ADR-0015 called out as the violation.
const CONSUMER_MODULES: [&str; 3] = [
    "src/cli/fetch_data.rs",
    "src/cli/indicators.rs",
    "src/cli/backtest.rs",
];

#[test]
fn consumer_modules_depend_on_the_port_not_the_concrete_store() {
    for relative in CONSUMER_MODULES {
        let code = blank_comments(&read_source(relative));
        assert!(
            !code.contains("CandleStore"),
            "{relative} still names CandleStore in CODE — only src/cli/mod.rs, \
             ADR-0015's composition-root exception, may construct the adapter"
        );
        assert!(
            code.contains("CandleSeriesRepository"),
            "{relative} must consume the domain port"
        );
    }
}

#[test]
fn the_engine_names_neither_the_adapter_nor_the_port() {
    let code = blank_comments(&read_source("src/adapters/backtest/engine.rs"));
    assert!(
        !code.contains("CandleStore"),
        "the deterministic engine must not name the storage adapter"
    );
    assert!(
        !code.contains("CandleSeriesRepository"),
        "the deterministic engine consumes loaded series — it must not name the repository port either"
    );
}

#[test]
fn the_cli_composition_root_is_the_one_construction_site() {
    let code = blank_comments(&read_source("src/cli/mod.rs"));
    assert!(
        code.contains("CandleStore"),
        "src/cli/mod.rs is ADR-0015's explicit composition-root exception and still \
         chooses the concrete adapter"
    );
}
