//! End-to-end migration up/down + backup-before-migrate proof (VS-1.1.4
//! work-1.05, auto #2) through the public `Db` open path.
//!
//! Drives 1.04's protocol (`run_migrations_with_backup` / `undo_to`) + 1.01's
//! `Db` open API through the `pulse::` surface — no new protocol code (§9). The
//! embedded migration set ships `0001_init` (tables + immutability triggers),
//! `0002` (the `idx_strategy_name` index), `0003` (the `backtest_run` + `trade`
//! system-of-record tables, VS-1.2.4 work-4.03), `0004` (the append-only
//! `llm_call` ledger, VS-1.3.1 work-1.02), and `0007` (the `llm_call.key_source`
//! provenance column, r1.s1.w2); the embedded max is therefore 7.
//!
//! The 0005/0006 GAP is deliberate, not an omission: those numbers are reserved
//! for `r1.s2` and `r1.s3`, allocated at release planning so three spines in
//! flight cannot collide on a migration number. sqlx does not require contiguous
//! versions — it applies them in numeric order and records what it applied.
//!
//! §4a-4: this uses the EXPORTED `undo_to(&pool, target)` wrapper, NOT
//! `Migrator::undo` (a crate-root name collision resolving to the DSL document
//! migrator, which has no `undo`). Offline + `TempDir`-isolated.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use pulse::{Db, MIGRATOR, run_migrations_with_backup, undo_to};
use sqlx::SqlitePool;
use std::path::Path;
use tempfile::TempDir;

/// The max applied migration version from `_sqlx_migrations` (0 if absent/empty).
async fn applied_max(pool: &SqlitePool) -> i64 {
    let exists: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='_sqlx_migrations'",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    if exists == 0 {
        return 0;
    }
    sqlx::query_scalar("SELECT COALESCE(MAX(version), 0) FROM _sqlx_migrations")
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Whether a named object exists in `sqlite_master`.
async fn object_present(pool: &SqlitePool, kind: &str, name: &str) -> bool {
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE type=?1 AND name=?2")
        .bind(kind)
        .bind(name)
        .fetch_one(pool)
        .await
        .unwrap();
    n == 1
}

/// The `idx_strategy_name` index 0002 creates (gone at v1, present at v2).
async fn index_present(pool: &SqlitePool) -> bool {
    object_present(pool, "index", "idx_strategy_name").await
}

/// Count `pulse.db.bak-*` backup files in `dir`.
fn count_backups(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .expect("read temp dir")
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with("pulse.db.bak-"))
        .count()
}

/// Bring a fresh temp db to version 1 only (run the embedded set, then undo to 1),
/// returning the guard + the db path with 0002 pending.
async fn db_at_0001() -> (TempDir, std::path::PathBuf) {
    let tmp = TempDir::new().expect("tempdir");
    let path = tmp.path().join("pulse.db");
    let db = Db::with_path(&path).await.expect("open db");
    MIGRATOR.run(db.pool()).await.expect("run embedded set");
    undo_to(db.pool(), 1).await.expect("undo to 1");
    assert_eq!(applied_max(db.pool()).await, 1, "fixture sits at version 1");
    assert!(!index_present(db.pool()).await, "0002 index gone at v1");
    (tmp, path)
}

// ---- AC-10 (auto #2): migrate up then undo is reversible ---------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn migrate_up_then_undo_is_reversible() {
    // (a) opening a fresh TempDir Db + running migrations advances to the embedded
    //     max; both tables exist.
    let tmp = TempDir::new().expect("tempdir");
    let path = tmp.path().join("pulse.db");
    let db = Db::with_path(&path).await.expect("open fresh db");
    MIGRATOR
        .run(db.pool())
        .await
        .expect("migrate to embedded max");

    assert_eq!(
        applied_max(db.pool()).await,
        7,
        "migrated to embedded max (7)"
    );
    assert!(
        object_present(db.pool(), "table", "strategy").await,
        "strategy table present"
    );
    assert!(
        object_present(db.pool(), "table", "strategy_version").await,
        "strategy_version table present"
    );
    assert!(
        index_present(db.pool()).await,
        "0002 index present after up"
    );

    // (b) §4a-4: undo_to(&pool, 1) reverses 0002 — the index is gone, max drops.
    undo_to(db.pool(), 1).await.expect("undo to 1");
    assert_eq!(applied_max(db.pool()).await, 1, "after undo, max == 1");
    assert!(
        !index_present(db.pool()).await,
        "after undo, 0002 index gone"
    );

    // (c) re-running brings it back (reversible round): 1 → 3.
    MIGRATOR
        .run(db.pool())
        .await
        .expect("re-run to embedded max");
    assert_eq!(applied_max(db.pool()).await, 7, "after re-run, max == 7");
    assert!(
        index_present(db.pool()).await,
        "after re-run, 0002 index back"
    );
}

// ---- AC-11 (auto #2): a backup is written before migrate --------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn backup_written_before_migrate() {
    // A db advanced only to 0001 (0002 pending) → run_migrations_with_backup
    // snapshots a `pulse.db.bak-…` file BEFORE applying 0002 (NFR-12 backup-
    // before-migrate), then completes to the embedded max.
    let (tmp, path) = db_at_0001().await;
    assert_eq!(
        count_backups(tmp.path()),
        0,
        "no backup before the migrate call"
    );

    let outcome = run_migrations_with_backup(&path)
        .await
        .expect("migrate from behind");

    // A backup file appeared (the backup-before-migrate leg, §4a-1).
    assert_eq!(count_backups(tmp.path()), 1, "exactly one backup retained");

    // The outcome names the pre-migration backup path + the from/to versions.
    match outcome {
        pulse::MigrationOutcome::Migrated { from, to, backup } => {
            assert_eq!(from, 1, "from == the pre-migration version");
            assert_eq!(to, 7, "to == the embedded max");
            assert!(backup.exists(), "backup file exists: {}", backup.display());
            let name = backup.file_name().unwrap().to_string_lossy().into_owned();
            assert!(
                name.starts_with("pulse.db.bak-1-"),
                "backup name is pulse.db.bak-1-<ts>, got {name}"
            );
        }
        other @ pulse::MigrationOutcome::AlreadyCurrent { .. } => {
            panic!("expected Migrated, got {other:?}")
        }
    }

    // The migration completed to the embedded max.
    let db = Db::with_path(&path).await.expect("reopen migrated db");
    assert_eq!(applied_max(db.pool()).await, 7, "schema now at 0007");
    assert!(
        index_present(db.pool()).await,
        "0002 index present post-migrate"
    );
}

// ---- AC-1 (VS-1.2.4 work-4.03): the 0003 backtest_run + trade roundtrip --------

/// Whether all four `0003` objects (both tables + both indexes) are present.
async fn schema_0003_present(pool: &SqlitePool) -> bool {
    object_present(pool, "table", "backtest_run").await
        && object_present(pool, "table", "trade").await
        && object_present(pool, "index", "idx_br_strategy_version").await
        && object_present(pool, "index", "idx_trade_run").await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn migration_0003_backtest_run_and_trade_roundtrip() {
    // (a) a fresh db migrated to the embedded max sits at 0003 with both new tables
    //     and both new indexes present (C4 schema landed).
    let tmp = TempDir::new().expect("tempdir");
    let path = tmp.path().join("pulse.db");
    let db = Db::with_path(&path).await.expect("open fresh db");
    MIGRATOR
        .run(db.pool())
        .await
        .expect("migrate to embedded max");

    assert_eq!(
        applied_max(db.pool()).await,
        7,
        "migrated to embedded max (7)"
    );
    assert!(
        schema_0003_present(db.pool()).await,
        "0003 backtest_run + trade tables and both indexes present after up"
    );

    // (b) undo_to(pool, 2) reverses ONLY 0003 — both new tables + indexes are gone,
    //     max drops to 2, while the 0002 index (an earlier step) survives.
    undo_to(db.pool(), 2).await.expect("undo to 2");
    assert_eq!(applied_max(db.pool()).await, 2, "after undo, max == 2");
    assert!(
        !object_present(db.pool(), "table", "backtest_run").await,
        "after undo to 2, backtest_run table gone"
    );
    assert!(
        !object_present(db.pool(), "table", "trade").await,
        "after undo to 2, trade table gone"
    );
    assert!(
        !object_present(db.pool(), "index", "idx_br_strategy_version").await,
        "after undo to 2, idx_br_strategy_version gone"
    );
    assert!(
        !object_present(db.pool(), "index", "idx_trade_run").await,
        "after undo to 2, idx_trade_run gone"
    );
    assert!(
        index_present(db.pool()).await,
        "after undo to 2, the earlier 0002 index survives"
    );

    // (c) re-running brings 0003 back (reversible round): 2 → 4 (0004 rides along).
    MIGRATOR
        .run(db.pool())
        .await
        .expect("re-run to embedded max");
    assert_eq!(applied_max(db.pool()).await, 7, "after re-run, max == 7");
    assert!(
        schema_0003_present(db.pool()).await,
        "after re-run, 0003 backtest_run + trade tables and both indexes back"
    );
}

// ---- VS-1.3.1 work-1.02: the 0004 llm_call up/down reversibility ------------

/// Whether all three `0004` objects (the table + both immutability triggers +
/// the index) are present.
async fn schema_0004_present(pool: &SqlitePool) -> bool {
    object_present(pool, "table", "llm_call").await
        && object_present(pool, "trigger", "llm_call_no_update").await
        && object_present(pool, "trigger", "llm_call_no_delete").await
        && object_present(pool, "index", "idx_llm_call_created_at").await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn migration_0004_llm_call_roundtrip() {
    // (a) a fresh db migrated to the embedded max sits at 0004 with the llm_call
    //     table, both immutability triggers, and the created_at index present.
    let tmp = TempDir::new().expect("tempdir");
    let path = tmp.path().join("pulse.db");
    let db = Db::with_path(&path).await.expect("open fresh db");
    MIGRATOR
        .run(db.pool())
        .await
        .expect("migrate to embedded max");

    assert_eq!(
        applied_max(db.pool()).await,
        7,
        "migrated to embedded max (7)"
    );
    assert!(
        schema_0004_present(db.pool()).await,
        "0004 llm_call table + both triggers + index present after up"
    );

    // (b) undo_to(pool, 3) reverses ONLY 0004 — the llm_call objects are gone, max
    //     drops to 3, while the 0003 backtest_run table (an earlier step) survives.
    undo_to(db.pool(), 3).await.expect("undo to 3");
    assert_eq!(applied_max(db.pool()).await, 3, "after undo, max == 3");
    assert!(
        !object_present(db.pool(), "table", "llm_call").await,
        "after undo to 3, llm_call table gone"
    );
    assert!(
        !object_present(db.pool(), "trigger", "llm_call_no_update").await,
        "after undo to 3, llm_call_no_update trigger gone"
    );
    assert!(
        !object_present(db.pool(), "index", "idx_llm_call_created_at").await,
        "after undo to 3, idx_llm_call_created_at gone"
    );
    assert!(
        object_present(db.pool(), "table", "backtest_run").await,
        "after undo to 3, the earlier 0003 backtest_run table survives"
    );

    // (c) re-running brings 0004 back (reversible round): 3 → 4.
    MIGRATOR
        .run(db.pool())
        .await
        .expect("re-run to embedded max");
    assert_eq!(applied_max(db.pool()).await, 7, "after re-run, max == 7");
    assert!(
        schema_0004_present(db.pool()).await,
        "after re-run, 0004 llm_call table + triggers + index back"
    );
}

/// **The reserved-number scheme actually works** — a later, LOWER-numbered
/// migration still applies after `0007` is already in the database.
///
/// This test exists to settle a PR review finding (#115, Codex P1) claiming the
/// opposite: that once an installation has applied `0007`, adding the reserved
/// `0005` / `0006` files afterwards would make sqlx "report a missing-version /
/// history error rather than execute them", stranding users of this build on a
/// database no later build could open. That would invalidate the whole
/// allocate-numbers-at-release-planning scheme the 0005/0006 gap exists for.
///
/// It is false, and this is the proof rather than an argument. sqlx 0.8.6's
/// `Migrator::run_to` iterates the migration list and applies any version NOT
/// present in `_sqlx_migrations`, regardless of whether it sorts above or below
/// the database's current maximum; the only ordering check it performs
/// (`validate_applied_migrations`) fires in the opposite direction — an APPLIED
/// migration that has vanished from the source set. So the sequence below —
/// apply through 0007, then introduce 0005 — is exactly what `r1.s2` will do,
/// and it succeeds.
#[tokio::test]
async fn a_later_lower_numbered_migration_still_applies_after_0007() {
    use sqlx::migrate::Migrator;

    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("migrations");
    std::fs::create_dir_all(&dir).unwrap();

    // Stage 1: the migration set exactly as this build ships it — 0001-0004
    // plus 0007, with the 0005/0006 gap.
    let shipped = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    for entry in std::fs::read_dir(&shipped).unwrap() {
        let path = entry.unwrap().path();
        std::fs::copy(&path, dir.join(path.file_name().unwrap())).unwrap();
    }

    let db_path = tmp.path().join("pulse.db");
    let db = Db::with_path(&db_path).await.expect("open");
    Migrator::new(dir.as_path())
        .await
        .expect("build the shipped migrator")
        .run(db.pool())
        .await
        .expect("the shipped set applies");
    assert_eq!(applied_max(db.pool()).await, 7, "0007 is applied");

    // Stage 2: `r1.s2` lands its reserved 0005 — a version BELOW the database's
    // current maximum, arriving after 0007 was already applied.
    std::fs::write(
        dir.join("0005_reserved_spine_r1s2.up.sql"),
        "CREATE TABLE reserved_0005_probe (id TEXT PRIMARY KEY);\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("0005_reserved_spine_r1s2.down.sql"),
        "DROP TABLE reserved_0005_probe;\n",
    )
    .unwrap();

    Migrator::new(dir.as_path())
        .await
        .expect("build the migrator with 0005 added")
        .run(db.pool())
        .await
        .expect("adding a lower-numbered migration after 0007 must NOT be an error");

    assert!(
        object_present(db.pool(), "table", "reserved_0005_probe").await,
        "0005 was applied out of numeric order, exactly as the reserved-number scheme needs"
    );
    assert_eq!(
        applied_max(db.pool()).await,
        7,
        "the recorded maximum is still 7 — 0005 is recorded at its own version, not appended"
    );
}
