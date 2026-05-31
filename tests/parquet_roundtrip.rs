//! Integration tests for WI-1.1.1.04 — Parquet persistence + content-hash
//! versioning + byte-identical I/O.
//!
//! Every test writes to a `tempfile` base dir (AC-3) and never touches the real
//! Application Support directory. Each test maps to a spec AC (see headers).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::str::FromStr;

use pulse::{
    CANDLE_SCHEMA_VERSION, Candle, CandleSeries, CandleStore, DataError, Gap, Pair, Timeframe,
};
use rust_decimal::Decimal;
use tempfile::TempDir;

fn candle(open_time: i64, funding: Option<&str>) -> Candle {
    Candle {
        open_time,
        close_time: open_time + 899_999,
        open: Decimal::from_str("42000.5").unwrap(),
        high: Decimal::from_str("42100.0").unwrap(),
        low: Decimal::from_str("41950.25").unwrap(),
        close: Decimal::from_str("42050.75").unwrap(),
        volume: Decimal::from_str("12.34567").unwrap(),
        funding_rate: funding.map(|f| Decimal::from_str(f).unwrap()),
    }
}

/// A series whose `version` is the content hash (the store recomputes it on write).
fn series(open_times: &[(i64, Option<&str>)]) -> CandleSeries {
    let candles: Vec<Candle> = open_times.iter().map(|(t, f)| candle(*t, *f)).collect();
    let mut s = CandleSeries {
        pair: Pair::new("BTCUSDT"),
        timeframe: Timeframe::M15,
        version: pulse::DataVersion::new("placeholder"),
        candles,
    };
    // Stamp the canonical content version so equality after read-back holds.
    s.version = CandleStore::content_version(&s.pair, s.timeframe, &s.candles);
    s
}

fn store() -> (CandleStore, TempDir) {
    let tmp = TempDir::new().expect("create tempdir");
    let store = CandleStore::with_base_dir(tmp.path().to_path_buf());
    (store, tmp)
}

// AC-1: CandleSeries -> Parquet -> CandleSeries round-trips equal. OHLCV Decimal
// exact via UTF-8 string, i64 timestamps, funding_rate nullable preserved.
#[test]
fn ac1_round_trips_equal_preserving_decimals_and_nullable_funding() {
    let (store, _tmp) = store();
    let s = series(&[
        (0, Some("0.0001")),
        (900_000, None),
        (1_800_000, Some("-0.00005")),
    ]);

    store.write_snapshot(&s).expect("write");
    let back = store
        .read_snapshot(&s.pair, s.timeframe, &s.version)
        .expect("read");

    assert_eq!(s, back, "round-trip must be exactly equal");
    // Funding nullability preserved precisely.
    assert_eq!(
        back.candles[0].funding_rate,
        Some(Decimal::from_str("0.0001").unwrap())
    );
    assert_eq!(back.candles[1].funding_rate, None);
    assert_eq!(
        back.candles[2].funding_rate,
        Some(Decimal::from_str("-0.00005").unwrap())
    );
}

// AC-2 (NFR-2): for a fixed writer version, writing the same series twice is
// byte-identical after writer-metadata normalization (`created_by` stripped); the
// determinism contract is equal content round-trip + equal data_version.
#[test]
fn ac2_byte_identical_after_normalization_and_equal_version() {
    let (store, _tmp) = store();
    let s = series(&[(0, Some("0.0001")), (900_000, None)]);

    let bytes1 = store.encode_snapshot(&s).expect("encode 1");
    let bytes2 = store.encode_snapshot(&s).expect("encode 2");

    let norm1 = CandleStore::normalize_writer_metadata(&bytes1).expect("normalize 1");
    let norm2 = CandleStore::normalize_writer_metadata(&bytes2).expect("normalize 2");
    assert_eq!(norm1, norm2, "normalized bytes must be byte-identical");

    // Determinism contract leg 2: equal content round-trip + equal data_version.
    let v1 = CandleStore::content_version(&s.pair, s.timeframe, &s.candles);
    let v2 = CandleStore::content_version(&s.pair, s.timeframe, &s.candles);
    assert_eq!(v1, v2, "data_version is deterministic (NFR-2)");
}

// AC-3: path resolves to <base>/candles/<PAIR>/<TF>/<data_version>.parquet; base
// dir injectable; tests use a temp dir.
#[test]
fn ac3_path_layout_under_injectable_base_dir() {
    let tmp = TempDir::new().expect("tempdir");
    let base = tmp.path().to_path_buf();
    let store = CandleStore::with_base_dir(base.clone());
    let s = series(&[(0, None)]);

    let path = store.snapshot_path(&s.pair, s.timeframe, &s.version);
    let expected = base
        .join("candles")
        .join("BTCUSDT")
        .join("15m")
        .join(format!("{}.parquet", s.version));
    assert_eq!(path, expected);

    store.write_snapshot(&s).expect("write");
    assert!(
        path.exists(),
        "snapshot file must exist at the computed path"
    );
}

// AC-4 (audit C5): re-writing identical content is a no-op success; SnapshotExists
// fires only when the path exists with differing content.
#[test]
fn ac4_idempotent_rewrite_is_noop_and_collision_errors() {
    let (store, _tmp) = store();
    let s = series(&[(0, Some("0.0001")), (900_000, None)]);

    store.write_snapshot(&s).expect("first write");
    // Identical re-write: no-op success, not an error.
    store
        .write_snapshot(&s)
        .expect("identical re-write must be a no-op success");

    // Tamper the on-disk content at the same data_version path -> SnapshotExists.
    let path = store.snapshot_path(&s.pair, s.timeframe, &s.version);
    std::fs::write(&path, b"corrupt-not-a-parquet").expect("tamper");
    let err = store
        .write_snapshot(&s)
        .expect_err("differing content must error");
    assert!(
        matches!(err, DataError::SnapshotExists { .. }),
        "expected SnapshotExists, got {err:?}"
    );
}

// AC-5 (audit C7): data_version = sha256(canonical)[..16hex] over
// (pair,tf,CANDLE_SCHEMA_VERSION,candles). Deterministic; differs on candle-set
// change; differs when CANDLE_SCHEMA_VERSION changes.
#[test]
fn ac5_content_hash_is_16_hex_deterministic_and_sensitive() {
    let pair = Pair::new("BTCUSDT");
    let candles_a = vec![candle(0, Some("0.0001")), candle(900_000, None)];
    let candles_b = vec![
        candle(0, Some("0.0001")),
        candle(900_000, None),
        candle(1_800_000, None),
    ];

    let v_a1 = CandleStore::content_version(&pair, Timeframe::M15, &candles_a);
    let v_a2 = CandleStore::content_version(&pair, Timeframe::M15, &candles_a);
    let v_b = CandleStore::content_version(&pair, Timeframe::M15, &candles_b);

    // 16 hex chars.
    assert_eq!(v_a1.as_str().len(), 16, "data_version is 16 hex chars");
    assert!(v_a1.as_str().chars().all(|c| c.is_ascii_hexdigit()));
    // Deterministic for identical data.
    assert_eq!(v_a1, v_a2);
    // Differs on candle-set change.
    assert_ne!(v_a1, v_b);
    // Differs when schema_version changes (explicit-version helper).
    let v_schema_bump = CandleStore::content_version_with_schema(
        &pair,
        Timeframe::M15,
        CANDLE_SCHEMA_VERSION + 1,
        &candles_a,
    );
    assert_ne!(
        v_a1, v_schema_bump,
        "bumping schema_version must change data_version"
    );
    // Differs across pair / timeframe.
    assert_ne!(
        v_a1,
        CandleStore::content_version(&Pair::new("ETHUSDT"), Timeframe::M15, &candles_a)
    );
    assert_ne!(
        v_a1,
        CandleStore::content_version(&pair, Timeframe::H4, &candles_a)
    );
}

// AC-6: latest_version / snapshot_exists report correctly on a populated temp store.
#[test]
fn ac6_latest_version_and_snapshot_exists() {
    let (store, _tmp) = store();
    let pair = Pair::new("BTCUSDT");

    // Empty store: nothing exists, no latest.
    let s1 = series(&[(0, None), (900_000, None)]);
    assert!(!store.snapshot_exists(&pair, Timeframe::M15, &s1.version));
    assert!(
        store
            .latest_version(&pair, Timeframe::M15)
            .expect("latest on empty store ok")
            .is_none()
    );

    store.write_snapshot(&s1).expect("write s1");
    assert!(store.snapshot_exists(&pair, Timeframe::M15, &s1.version));

    let latest = store
        .latest_version(&pair, Timeframe::M15)
        .expect("latest ok")
        .expect("some latest");
    assert_eq!(latest, s1.version);
}

// AC-7 (audit C8): an interrupted/failed write (temp never renamed) leaves no
// visible partial snapshot; only fsync+rename publishes.
#[test]
fn ac7_failed_write_leaves_no_visible_partial_snapshot() {
    let (store, _tmp) = store();
    let s = series(&[(0, None), (900_000, None)]);
    let path = store.snapshot_path(&s.pair, s.timeframe, &s.version);

    // Simulate a write that produced a temp file but was interrupted before rename.
    store.write_temp_only_for_test(&s).expect("temp-only write");

    // No visible snapshot at the final path.
    assert!(!path.exists(), "no published snapshot before rename");
    assert!(!store.snapshot_exists(&s.pair, s.timeframe, &s.version));
    // No leftover .parquet visible in the directory listing (temp uses a hidden/.tmp name).
    let dir = path.parent().unwrap();
    if dir.exists() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let name = entry.unwrap().file_name().to_string_lossy().into_owned();
            assert!(
                !name.ends_with(".parquet") || name.starts_with('.'),
                "no visible *.parquet partial: found {name}"
            );
        }
    }

    // A real write then publishes atomically.
    store.write_snapshot(&s).expect("real write");
    assert!(path.exists());
}

// AC-8 (audit C9): written Parquet carries provenance KV metadata and reads back.
#[test]
fn ac8_provenance_kv_metadata_round_trips() {
    let (store, _tmp) = store();
    let s = series(&[(0, Some("0.0001")), (1_800_000, None)]); // gap at 900_000

    store.write_snapshot(&s).expect("write");
    let prov = store
        .read_provenance(&s.pair, s.timeframe, &s.version)
        .expect("read provenance");

    assert_eq!(prov.pair, "BTCUSDT");
    assert_eq!(prov.timeframe, "15m");
    assert_eq!(prov.data_version, s.version.as_str());
    assert_eq!(prov.schema_version, CANDLE_SCHEMA_VERSION);
    assert_eq!(prov.source, "binance-um");
    assert_eq!(prov.first_open_ms, Some(0));
    assert_eq!(prov.last_open_ms, Some(1_800_000));
    // The detected gap (expected 900_000, found 1_800_000) is recorded.
    assert_eq!(
        prov.gaps,
        vec![Gap {
            expected: 900_000,
            found: 1_800_000
        }]
    );
}
