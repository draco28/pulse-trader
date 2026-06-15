//! End-to-end migration up/down + backup-before-migrate proof (VS-1.1.4
//! work-1.05, auto #2) through the public `Db` open path.
//!
//! Drives 1.04's protocol (`run_migrations_with_backup` / `undo_to`) + 1.01's
//! `Db` open API through the `pulse::` surface — no new protocol code (§9). The
//! embedded migration set ships `0001_init` (tables + immutability triggers) and
//! `0002` (the `idx_strategy_name` index); the embedded max is therefore 2.
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
        2,
        "migrated to embedded max (2)"
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

    // (c) re-running brings it back (reversible round): 1 → 2.
    MIGRATOR
        .run(db.pool())
        .await
        .expect("re-run to embedded max");
    assert_eq!(applied_max(db.pool()).await, 2, "after re-run, max == 2");
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
            assert_eq!(to, 2, "to == the embedded max");
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
    assert_eq!(applied_max(db.pool()).await, 2, "schema now at 0002");
    assert!(
        index_present(db.pool()).await,
        "0002 index present post-migrate"
    );
}
