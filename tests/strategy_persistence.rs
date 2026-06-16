//! End-to-end demo integration tests for VS-1.1.4 work-1.05 (auto #1).
//!
//! Realizes the slice's first demo criterion — "a Strategy + immutable
//! `StrategyVersion` is created and reloaded BYTE-IDENTICALLY" (NFR-2) — plus the
//! FR-4 immutability half (a raw `UPDATE`/`DELETE` against `strategy_version` is
//! aborted by the DB trigger). The load-bearing byte-identity + immutability
//! assertions drive the **library path** (the repo over a `TempDir` `Db`), so the
//! test can assert exact bytes + reach a raw `sqlx` tamper; a single smoke test
//! drives the **binary** to prove the clap→dispatch→repo wiring end-to-end.
//!
//! Offline (`SQLX_OFFLINE=true` + committed `.sqlx/` + in-process `MIGRATOR`),
//! `TempDir`-isolated (never the real Application Support dir).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::Command;

use pulse::{
    Comparator, Condition, CreatedBy, DataError, Db, Direction, ExitRule, IndicatorSpec, MIGRATOR,
    NewVersion, RiskParams, SchemaVersion, SqliteStrategyRepo, StrategyDsl, StrategyRepository,
    SweepableValue, ValueSource, VersionId,
};
use rust_decimal::Decimal;
use sqlx::SqlitePool;
use tempfile::TempDir;

/// A `(repo, pool, tempdir)` triple over a fresh migrated tempfile `pulse.db`.
/// The integration test opens its OWN `Db` + pool (the repo's pool is private) —
/// mirrors `strategy_repo.rs`'s in-crate `repo()` helper, but through the public
/// `pulse::{Db, MIGRATOR, SqliteStrategyRepo}` surface (§4a-7).
async fn repo() -> (SqliteStrategyRepo<pulse::SystemClock>, SqlitePool, TempDir) {
    let tmp = TempDir::new().expect("tempdir");
    let db = Db::with_path(&tmp.path().join("pulse.db"))
        .await
        .expect("open db at tempfile path");
    MIGRATOR.run(db.pool()).await.expect("run 0001_init");
    let pool = db.pool().clone();
    (SqliteStrategyRepo::new(pool.clone()), pool, tmp)
}

/// The canonical `1.0.0` RSI-oversold strategy — VALID per `validate()` (it has a
/// `StopLoss`, §4a-5). Built via the typed `StrategyDsl` so the shape is
/// guaranteed schema-current; `create_version` runs `validate()` after
/// `Migrator::load`, so a parseable-but-invalid DSL would be REJECTED. Mirrors
/// `strategy_repo.rs::canonical_dsl()`.
fn canonical_dsl() -> StrategyDsl {
    StrategyDsl {
        schema_version: SchemaVersion::CURRENT,
        name: "RSI Oversold".to_owned(),
        direction: Direction::Long,
        entry: Condition::Compare {
            lhs: ValueSource::Indicator {
                spec: IndicatorSpec::Rsi {
                    period: SweepableValue::Fixed(14),
                },
            },
            op: Comparator::Lt,
            rhs: ValueSource::Constant {
                value: Decimal::new(30, 0),
            },
        },
        filters: vec![],
        exits: vec![
            ExitRule::StopLoss {
                distance_pct: SweepableValue::Fixed(Decimal::new(5, 2)),
            },
            ExitRule::TakeProfit {
                target_r: SweepableValue::Fixed(Decimal::new(2, 0)),
            },
        ],
        risk: RiskParams {
            risk_per_trade_pct: SweepableValue::Fixed(Decimal::new(1, 2)),
            max_leverage: SweepableValue::Fixed(Decimal::new(3, 0)),
        },
    }
}

/// The canonical DSL serialized to a JSON string (the `--dsl <file>` / `dsl_json`
/// contents).
fn canonical_json() -> String {
    serde_json::to_string(&canonical_dsl()).expect("serialize canonical dsl")
}

/// Create a strategy + one version over a fresh repo, returning the source bytes
/// and the created version id for the byte-identity / tamper assertions.
async fn seed_one_version(repo: &SqliteStrategyRepo<pulse::SystemClock>) -> (String, VersionId) {
    let s = repo
        .create_strategy("Demo", Some("alice"), &["btc".to_owned()])
        .await
        .expect("create strategy");
    let dsl_json = canonical_json();
    let created = repo
        .create_version(NewVersion {
            strategy_id: s.id.clone(),
            parent_version_id: None,
            dsl_json: dsl_json.clone(),
            created_by: CreatedBy::Human,
            creating_llm_call_ids: vec![],
        })
        .await
        .expect("create version");
    (dsl_json, created.id)
}

// ---- AC-7 (NFR-2 / auto #1): byte-identical reload through the repo ----------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reload_is_byte_identical_through_repo() {
    let (repo, _pool, _tmp) = repo().await;
    let s = repo
        .create_strategy("ByteId", Some("alice"), &["scalp".to_owned()])
        .await
        .unwrap();
    let dsl_json = canonical_json();
    let created = repo
        .create_version(NewVersion {
            strategy_id: s.id.clone(),
            parent_version_id: None,
            dsl_json: dsl_json.clone(),
            created_by: CreatedBy::Human,
            creating_llm_call_ids: vec![],
        })
        .await
        .unwrap();

    let fetched = repo.get_version(&created.id).await.unwrap().unwrap();

    // (a) NFR-2: the verbatim source bytes survive create→read BYTE-FOR-BYTE.
    assert_eq!(
        fetched.dsl_original, dsl_json,
        "dsl_original must round-trip byte-identical"
    );

    // (b) NFR-2: the stored version_hash re-derives equal (the read defense in
    // `row_to_version` already rejects a mismatch — a successful get_version IS
    // the re-derivation proof; assert the field is the 64-char SHA-256 hex).
    assert_eq!(
        fetched.version_hash.len(),
        64,
        "version_hash is SHA-256 hex"
    );
    assert!(
        fetched.version_hash.chars().all(|c| c.is_ascii_hexdigit()),
        "version_hash is lowercase hex"
    );

    // (c) the migrated `.dsl` round-trips through Migrator::load to the canonical
    // typed document.
    assert_eq!(
        fetched.dsl,
        canonical_dsl(),
        "loaded dsl is the canonical doc"
    );

    // (d) exact field equality on the reloaded Strategy + StrategyVersion.
    let reloaded_strategy = repo.get_strategy(&s.id).await.unwrap().unwrap();
    assert_eq!(reloaded_strategy, s, "Strategy reloads field-identical");
    assert_eq!(fetched.strategy_id, s.id);
    assert_eq!(fetched.parent_version_id, None);
    assert_eq!(fetched.dsl_schema_version, SchemaVersion::CURRENT);
    assert_eq!(fetched.created_by, CreatedBy::Human);
    assert!(
        fetched.creating_llm_call_ids.is_empty(),
        "FR-11: no LLM ⇒ empty creating_llm_call_ids"
    );
}

// ---- AC-8 (FR-4): a raw UPDATE on strategy_version is aborted by the trigger -

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn raw_update_on_strategy_version_aborts() {
    let (repo, pool, _tmp) = repo().await;
    let (_dsl, vid) = seed_one_version(&repo).await;
    let id = vid.as_str().to_owned();

    // A RAW UPDATE bypassing the repo (which has no update_version) must be
    // aborted by the BEFORE UPDATE trigger (FR-4 end-to-end immutability proof).
    let err = sqlx::query("UPDATE strategy_version SET dsl = 'tampered' WHERE id = ?1")
        .bind(&id)
        .execute(&pool)
        .await
        .map_err(|e| DataError::Db(e.to_string()))
        .expect_err("raw UPDATE on an immutable row must fail");
    match err {
        DataError::Db(msg) => assert!(
            msg.contains("strategy_version is immutable"),
            "trigger ABORT message must surface; got: {msg}"
        ),
        other => panic!("expected DataError::Db, got {other:?}"),
    }

    // The row is UNCHANGED (the abort rolled the statement back).
    let after = repo.get_version(&vid).await.unwrap().unwrap();
    assert_eq!(
        after.dsl,
        canonical_dsl(),
        "row unchanged after aborted UPDATE"
    );
}

// ---- AC-9 (FR-4): a raw DELETE on strategy_version is aborted by the trigger -

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn raw_delete_on_strategy_version_aborts() {
    let (repo, pool, _tmp) = repo().await;
    let (_dsl, vid) = seed_one_version(&repo).await;
    let id = vid.as_str().to_owned();

    // SQLite needs a SEPARATE BEFORE DELETE trigger — this pins it is wired.
    let err = sqlx::query("DELETE FROM strategy_version WHERE id = ?1")
        .bind(&id)
        .execute(&pool)
        .await
        .map_err(|e| DataError::Db(e.to_string()))
        .expect_err("raw DELETE on an immutable row must fail");
    match err {
        DataError::Db(msg) => assert!(
            msg.contains("strategy_version is immutable"),
            "trigger ABORT message must surface; got: {msg}"
        ),
        other => panic!("expected DataError::Db, got {other:?}"),
    }

    // The row still exists (the abort rolled the DELETE back).
    let still = repo.get_version(&vid).await.unwrap();
    assert!(still.is_some(), "row still present after aborted DELETE");
}

// ---- binary smoke test (AC-5): clap→dispatch→repo wiring end-to-end ----------

#[test]
fn binary_create_then_show_over_tempdb() {
    let tmp = TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("pulse.db");

    // `strategy create demo --db <tempdb>` exits 0 and echoes a UUID-shaped id.
    let create = Command::new(env!("CARGO_BIN_EXE_pulse"))
        .args([
            "strategy",
            "create",
            "demo",
            "--db",
            db_path.to_str().expect("utf8 db path"),
        ])
        .output()
        .expect("run pulse strategy create");
    assert!(
        create.status.success(),
        "create status={:?}\nstderr={}",
        create.status.code(),
        String::from_utf8_lossy(&create.stderr)
    );
    let id = String::from_utf8(create.stdout)
        .expect("stdout utf8")
        .trim()
        .to_owned();
    // A UUID v4 is 36 hyphenated chars (8-4-4-4-12).
    assert_eq!(id.len(), 36, "create echoes a UUID-shaped id, got {id:?}");
    assert_eq!(id.matches('-').count(), 4, "UUID has 4 hyphens, got {id:?}");

    // A follow-up `strategy show <id> --db <tempdb>` exits 0 and prints it.
    let show = Command::new(env!("CARGO_BIN_EXE_pulse"))
        .args([
            "strategy",
            "show",
            &id,
            "--db",
            db_path.to_str().expect("utf8 db path"),
        ])
        .output()
        .expect("run pulse strategy show");
    assert!(
        show.status.success(),
        "show status={:?}\nstderr={}",
        show.status.code(),
        String::from_utf8_lossy(&show.stderr)
    );
    let stdout = String::from_utf8(show.stdout).expect("stdout utf8");
    assert!(
        stdout.contains(&id),
        "show output must name the strategy id {id:?}; got: {stdout}"
    );
    assert!(
        stdout.contains("demo"),
        "show output must name the strategy; got: {stdout}"
    );
}
