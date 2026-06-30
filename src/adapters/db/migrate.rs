//! The backup-before-migrate protocol (VS-1.1.4 work-1.04) — FR-4 / NFR-12.
//!
//! [`run_migrations_with_backup`] brings a `pulse.db` schema forward *safely*:
//! detect-behind → consistent-snapshot backup (`VACUUM INTO`) → run the embedded
//! [`MIGRATOR`](super::MIGRATOR) → verify → **restore + refuse to start** on any
//! failure. [`open_migrated`] is the single startup entry point (migrate-then-open)
//! 1.05's CLI wires; [`undo_to`] is the in-process down path the up/down round
//! test exercises.
//!
//! `VACUUM INTO` is `SQLite`'s first-class consistent-backup primitive — WAL-safe by
//! construction (no manual `wal_checkpoint`, no `-wal`/`-shm` sidecar copy, no torn
//! snapshot), the right tool because the DB is the system-of-record for real-money
//! trades. The restore path mirrors `store/mod.rs`'s atomic temp→rename + fsync
//! discipline (those helpers are private to a different module tree — mirrored, not
//! imported — per the audit-C5 re-derive convention).

use std::path::{Path, PathBuf};

use chrono::Utc;
use sqlx::SqlitePool;
use sqlx::migrate::Migrator;

use super::{Db, MIGRATOR};
use crate::domain::DataError;

/// The outcome of a [`run_migrations_with_backup`] call (migration-protocol
/// vocabulary, not a domain type). A small owned result so callers/tests can
/// assert the from/to versions and the backup path without re-querying
/// `_sqlx_migrations`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationOutcome {
    /// Schema already at the embedded max; no backup taken, no migration run.
    AlreadyCurrent {
        /// The current (== embedded max) applied version.
        version: i64,
    },
    /// Migrated `from` → `to`; the pre-migration db was backed up at `backup`.
    Migrated {
        /// The applied version before the migration ran.
        from: i64,
        /// The applied version after the migration ran (the embedded max).
        to: i64,
        /// The retained pre-migration backup file (NFR-12).
        backup: PathBuf,
    },
}

/// Bring `db_path`'s schema forward safely: back up before migrating, verify
/// after, restore + refuse to start on ANY failure.
///
/// Detect-behind first (no backup is paid for when already current); on a real
/// migration the pre-migration db is snapshotted to
/// `pulse.db.bak-<from_version>-<timestamp>` (NFR-12) via `VACUUM INTO` before the
/// embedded [`MIGRATOR`](super::MIGRATOR) runs. A migrate error or post-verify
/// mismatch restores the original db from the backup and returns an error — the
/// caller MUST treat that as fatal (REFUSE TO START, MASTER-SPEC §7.4).
///
/// # Single-process v1 assumption (#38)
/// This protocol assumes a **single process** brings the schema forward — v1 opens
/// the db from exactly one CLI/app instance. There is intentionally NO cross-process
/// advisory lock: two concurrent migrators racing the same `pulse.db` is out of
/// scope (track-forward). The single-writer `SQLite` WAL + the ahead-state refusal
/// below cover the v1 surface; a multi-process deployment would need the advisory
/// lock added before this assumption is relaxed.
///
/// # Errors
/// Returns [`DataError::Migration`] if the backup, migrate, or post-verify step
/// fails (the original db is restored from the backup first; the backup is
/// retained for forensics), OR if the db is **ahead** of the embedded set
/// (`applied_max > embedded_max` — a db newer than the binary must NOT be migrated
/// *or* opened; refuse to start, MASTER-SPEC §7.4). Returns [`DataError::Db`] if the
/// pool cannot be opened.
pub async fn run_migrations_with_backup(db_path: &Path) -> Result<MigrationOutcome, DataError> {
    run_migrations_with_backup_using(db_path, &MIGRATOR).await
}

/// The protocol body, parameterised over the migration source so tests can inject
/// a deliberately-broken runtime [`Migrator`] (the forced-failure test) without
/// poisoning the committed embedded set. Production calls it with `&MIGRATOR`.
async fn run_migrations_with_backup_using(
    db_path: &Path,
    migrator: &Migrator,
) -> Result<MigrationOutcome, DataError> {
    let db = Db::with_path(db_path).await?;
    let pool = db.pool();

    let applied_max = applied_max_version(pool).await?;
    let embedded_max = embedded_max_version(migrator);

    if applied_max == embedded_max {
        return Ok(MigrationOutcome::AlreadyCurrent {
            version: applied_max,
        });
    }

    // AHEAD-state refusal (#38): the db's successfully-applied schema is NEWER than
    // this binary's embedded set. A db newer than the binary must NOT be migrated
    // (there is no down path for migrations we don't ship) NOR opened — refuse to
    // start (MASTER-SPEC §7.4). This is a REAL `Err` (#65), never a `debug_assert!`:
    // the determinism gate + CI run `--release`, where a `debug_assert!` is compiled
    // out exactly when this guard matters. It fires BEFORE the behind-branch so no
    // backup is taken and `Migrated{from,to}` can never be reported inverted.
    if applied_max > embedded_max {
        return Err(DataError::Migration(format!(
            "db schema version {applied_max} is ahead of this binary's embedded max \
             {embedded_max}: refusing to migrate or open a db newer than the binary"
        )));
    }

    // Behind: snapshot the live db (consistent, WAL-safe) BEFORE migrating, then
    // make the snapshot crash-durable (fsync the backup file + its parent dir)
    // BEFORE the migrate runs — so a crash mid-migrate cannot lose the only recovery
    // copy (#38 durability).
    let backup = backup_path(db_path, applied_max);
    vacuum_into(pool, &backup).await?;
    fsync_file(&backup)?;
    if let Some(parent) = backup.parent() {
        fsync_dir(parent)?;
    }

    // Migrate, then verify; on ANY failure restore from the backup and refuse.
    match migrate_and_verify(pool, migrator, embedded_max).await {
        Ok(()) => Ok(MigrationOutcome::Migrated {
            from: applied_max,
            to: embedded_max,
            backup,
        }),
        Err(e) => {
            // Drop the pool before restoring so no open handle holds the file.
            drop(db);
            restore_from_backup(db_path, &backup)?;
            Err(e)
        }
    }
}

/// Run the backup-before-migrate protocol on `db_path`, THEN open the working pool.
///
/// The single startup entry point (migrate-then-open) — keeps 1.01's
/// [`Db::with_path`](super::Db::with_path)/`open_default` pure pool-openers (no
/// migration side effect) while giving the CLI/app ONE call satisfying
/// MASTER-SPEC §7.4's "on startup migrate the schema else refuse to start".
///
/// # Errors
/// [`DataError::Migration`] if the migration step fails (the db is already
/// restored — the caller MUST NOT start); [`DataError::Db`] if the pool cannot be
/// opened after a successful migrate.
pub async fn open_migrated(db_path: &Path) -> Result<Db, DataError> {
    run_migrations_with_backup(db_path).await?;
    Db::with_path(db_path).await
}

/// Revert the schema down to `target_version` (in-process, no CLI).
///
/// A thin wrapper over [`Migrator::undo`] — sqlx reverts every applied
/// down-migration with `version > target_version` via the matching `*.down.sql`.
/// No backup is taken: the backup discipline is a forward-migration property
/// (down is an explicit operator/test action and the caller already holds the
/// pool). Used by the up/down round test (run → undo → re-run).
///
/// # Errors
/// Returns [`DataError::Migration`] if the revert fails.
pub async fn undo_to(pool: &SqlitePool, target_version: i64) -> Result<(), DataError> {
    MIGRATOR
        .undo(pool, target_version)
        .await
        .map_err(|e| DataError::Migration(format!("undo to version {target_version} failed: {e}")))
}

/// Run the migrator, then re-derive the applied max and assert it now equals the
/// embedded max (defense-in-depth, mirroring `store/mod.rs`'s audit-C5
/// re-derive-and-reject guard). Any error here drives the caller's restore path.
async fn migrate_and_verify(
    pool: &SqlitePool,
    migrator: &Migrator,
    embedded_max: i64,
) -> Result<(), DataError> {
    migrator
        .run(pool)
        .await
        .map_err(|e| DataError::Migration(format!("migration run failed: {e}")))?;

    let post = applied_max_version(pool).await?;
    if post != embedded_max {
        return Err(DataError::Migration(format!(
            "post-migration verify mismatch: applied max {post} != embedded max {embedded_max}"
        )));
    }
    Ok(())
}

/// The max **successfully-applied** migration version from `_sqlx_migrations`, or
/// `0` when no migration has ever committed (the table absent or empty). Raw
/// `sqlx::query_scalar` (no `query!` macro — this item ships no `.sqlx` cache).
///
/// **Committed-state filter (#38 / audit C6).** sqlx records a row in
/// `_sqlx_migrations` for *every* attempt, with a `success` column that is `0`
/// until the migration commits. This function MUST read `MAX(version) WHERE
/// success = TRUE` so the value reflects the **applied schema**, not arbitrary
/// migration history: a failed or partially-applied future-version row
/// (`success = 0`) must NOT count, otherwise a single botched future-migration
/// attempt would spuriously trip the ahead-state refusal and brick the binary.
/// The ahead-state guard in [`run_migrations_with_backup_using`] depends on this
/// filter being in place.
async fn applied_max_version(pool: &SqlitePool) -> Result<i64, DataError> {
    // `MAX(version)` over an empty/absent set is NULL → coalesce to 0. The table
    // is created lazily by the migrator; guard against its absence on a fresh db.
    let exists: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations'",
    )
    .fetch_one(pool)
    .await
    .map_err(|e| DataError::Db(e.to_string()))?;
    if exists == 0 {
        return Ok(0);
    }
    let max: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(version), 0) FROM _sqlx_migrations WHERE success = TRUE",
    )
    .fetch_one(pool)
    .await
    .map_err(|e| DataError::Db(e.to_string()))?;
    Ok(max)
}

/// The max version among the migrator's *up* migrations (the `iter()` yields both
/// up and down entries per reversible step; only up entries define the target).
fn embedded_max_version(migrator: &Migrator) -> i64 {
    migrator
        .iter()
        .filter(|m| m.migration_type.is_up_migration())
        .map(|m| m.version)
        .max()
        .unwrap_or(0)
}

/// `pulse.db.bak-<from_version>-<timestamp>` co-located beside `db_path` (NFR-12).
/// Same directory ⇒ the restore-rename is atomic on the same filesystem. The
/// timestamp is a filesystem-safe UTC stamp (`%Y%m%dT%H%M%SZ`, no colons).
///
/// `VACUUM INTO` requires the target NOT already exist. Second-resolution stamps
/// can collide when two migrations of the same db run inside one second (the
/// up/down round: run → undo → re-run). The common case keeps the exact
/// `pulse.db.bak-<from>-<stamp>` name; a `-N` suffix is appended ONLY on a probed
/// collision, so the name stays unique without losing the documented convention.
fn backup_path(db_path: &Path, from_version: i64) -> PathBuf {
    let dir = db_path.parent();
    let stamp = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let base = format!("pulse.db.bak-{from_version}-{stamp}");
    let join = |name: &str| match dir {
        Some(d) => d.join(name),
        None => PathBuf::from(name),
    };

    let primary = join(&base);
    if !primary.exists() {
        return primary;
    }
    // Disambiguate a within-second collision; bounded probe keeps it total.
    for n in 1..u32::MAX {
        let candidate = join(&format!("{base}-{n}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    primary
}

/// Consistent-snapshot backup via `VACUUM INTO` (WAL-safe by construction). Writes
/// a single transactionally-consistent copy of the live db regardless of WAL
/// state — no manual checkpoint, no sidecar handling, no torn-snapshot race. The
/// snapshot is *logically* identical to the source (defragmented, not byte-equal).
async fn vacuum_into(pool: &SqlitePool, backup: &Path) -> Result<(), DataError> {
    // `VACUUM INTO` takes a string literal, not a bound parameter; the path is a
    // process-local, timestamp-derived name (not user input). Escape single quotes
    // defensively so a quote in the temp dir can't break the statement.
    let target = backup.to_string_lossy().replace('\'', "''");
    let stmt = format!("VACUUM INTO '{target}'");
    sqlx::query(&stmt)
        .execute(pool)
        .await
        .map_err(|e| DataError::Migration(format!("backup VACUUM INTO failed: {e}")))?;
    Ok(())
}

/// Restore `db_path` from `backup` (atomic rename on the same dir, mirroring
/// `store/mod.rs`'s temp→rename discipline) and delete stale `-wal`/`-shm`
/// sidecars beside `db_path`.
///
/// The backup is *copied* to a hidden temp then renamed over `db_path` so the
/// backup file itself is retained (forensics + NFR-12). A failed migrate may have
/// left WAL frames that would otherwise re-apply over the restored file; the
/// `VACUUM INTO` snapshot already has everything committed into the main file, so
/// the sidecars must go.
fn restore_from_backup(db_path: &Path, backup: &Path) -> Result<(), DataError> {
    let dir = db_path.parent().ok_or_else(|| {
        DataError::Migration(format!("db path has no parent: {}", db_path.display()))
    })?;

    let tmp = hidden_temp_path(db_path)?;
    std::fs::copy(backup, &tmp).map_err(|e| {
        DataError::Migration(format!(
            "restore: copy {} -> {} failed: {e}",
            backup.display(),
            tmp.display()
        ))
    })?;
    std::fs::rename(&tmp, db_path).map_err(|e| {
        DataError::Migration(format!(
            "restore: rename {} -> {} failed: {e}",
            tmp.display(),
            db_path.display()
        ))
    })?;

    // Drop stale WAL/SHM sidecars so they cannot re-apply over the restored file.
    for ext in ["-wal", "-shm"] {
        let sidecar = sidecar_path(db_path, ext);
        if sidecar.exists() {
            std::fs::remove_file(&sidecar).map_err(|e| {
                DataError::Migration(format!(
                    "restore: remove sidecar {} failed: {e}",
                    sidecar.display()
                ))
            })?;
        }
    }

    // fsync the directory so the rename + sidecar removals are durable.
    fsync_dir(dir)?;
    Ok(())
}

/// fsync a directory so a just-completed `rename` into it is durable (mirrors
/// `store/mod.rs::fsync_dir` — that fn is private to a different module).
fn fsync_dir(dir: &Path) -> Result<(), DataError> {
    let file = std::fs::File::open(dir).map_err(|e| {
        DataError::Migration(format!("restore: open dir {} failed: {e}", dir.display()))
    })?;
    file.sync_all().map_err(|e| {
        DataError::Migration(format!("restore: fsync dir {} failed: {e}", dir.display()))
    })
}

/// fsync a just-written file so its bytes are durable on disk before the migrate
/// runs (#38 backup durability — pairs with [`fsync_dir`] on the parent so both the
/// file contents AND the directory entry survive a crash).
fn fsync_file(path: &Path) -> Result<(), DataError> {
    let file = std::fs::File::open(path).map_err(|e| {
        DataError::Migration(format!("backup: open {} failed: {e}", path.display()))
    })?;
    file.sync_all().map_err(|e| {
        DataError::Migration(format!("backup: fsync file {} failed: {e}", path.display()))
    })
}

/// A hidden temp path co-located with `db_path` (same dir ⇒ rename is atomic on
/// the same filesystem; mirrors `store/mod.rs::temp_path`).
fn hidden_temp_path(db_path: &Path) -> Result<PathBuf, DataError> {
    let file_name = db_path
        .file_name()
        .ok_or_else(|| {
            DataError::Migration(format!("db path has no file name: {}", db_path.display()))
        })?
        .to_string_lossy();
    let dir = db_path.parent().ok_or_else(|| {
        DataError::Migration(format!("db path has no parent: {}", db_path.display()))
    })?;
    Ok(dir.join(format!(".{file_name}.restore.tmp")))
}

/// The `-wal` / `-shm` sidecar path beside a `SQLite` db file (the suffix is
/// appended to the full file name, e.g. `pulse.db-wal`).
fn sidecar_path(db_path: &Path, ext: &str) -> PathBuf {
    let mut name = db_path.as_os_str().to_os_string();
    name.push(ext);
    PathBuf::from(name)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{
        MigrationOutcome, applied_max_version, embedded_max_version, run_migrations_with_backup,
        run_migrations_with_backup_using, undo_to,
    };
    use crate::adapters::db::{Db, MIGRATOR};
    use crate::domain::DataError;
    use sqlx::migrate::Migrator;
    use std::path::Path;
    use tempfile::TempDir;

    /// Count `pulse.db.bak-*` files in `dir`.
    fn count_backups(dir: &Path) -> usize {
        std::fs::read_dir(dir)
            .expect("read temp dir")
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().starts_with("pulse.db.bak-"))
            .count()
    }

    /// The single `pulse.db.bak-*` file in `dir` (panics if not exactly one).
    fn the_backup(dir: &Path) -> std::path::PathBuf {
        let mut found: Vec<_> = std::fs::read_dir(dir)
            .expect("read temp dir")
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("pulse.db.bak-"))
            })
            .collect();
        assert_eq!(found.len(), 1, "expected exactly one backup, got {found:?}");
        found.pop().unwrap()
    }

    /// `_sqlx_migrations` max version (0 if the table is absent/empty).
    async fn applied_max(db: &Db) -> i64 {
        let exists: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='_sqlx_migrations'",
        )
        .fetch_one(db.pool())
        .await
        .unwrap();
        if exists == 0 {
            return 0;
        }
        sqlx::query_scalar("SELECT COALESCE(MAX(version), 0) FROM _sqlx_migrations")
            .fetch_one(db.pool())
            .await
            .unwrap()
    }

    /// Whether `idx_strategy_name` is present in `sqlite_master`.
    async fn index_present(db: &Db) -> bool {
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_strategy_name'",
        )
        .fetch_one(db.pool())
        .await
        .unwrap();
        n == 1
    }

    /// Bring a fresh temp db up to version 1 only (apply 0001, then undo to 1 so
    /// only 0001 remains applied) and return the guard + db path.
    async fn db_at_0001() -> (TempDir, std::path::PathBuf) {
        let tmp = TempDir::new().expect("tempdir");
        let path = tmp.path().join("pulse.db");
        let db = Db::with_path(&path).await.expect("open db");
        // Run the full embedded set then revert down to 1 — leaves exactly 0001.
        MIGRATOR.run(db.pool()).await.expect("run embedded");
        undo_to(db.pool(), 1).await.expect("undo to 1");
        assert_eq!(applied_max(&db).await, 1, "fixture must sit at version 1");
        assert!(!index_present(&db).await, "0002 index must be gone at v1");
        (tmp, path)
    }

    #[tokio::test]
    async fn migration_backup_created_when_behind() {
        // AC-11 / NFR-12: a db at 0001 is behind 0002 → backup fires, schema advances.
        let (tmp, path) = db_at_0001().await;

        let outcome = run_migrations_with_backup(&path)
            .await
            .expect("migrate from behind");

        match outcome {
            MigrationOutcome::Migrated { from, to, backup } => {
                assert_eq!(from, 1, "from must be the pre-migration version");
                assert_eq!(to, 3, "to must be the embedded max");
                assert!(
                    backup.exists(),
                    "backup file must exist: {}",
                    backup.display()
                );
                let name = backup.file_name().unwrap().to_string_lossy().into_owned();
                assert!(
                    name.starts_with("pulse.db.bak-1-"),
                    "backup name must be pulse.db.bak-1-<ts>, got {name}"
                );
            }
            other @ MigrationOutcome::AlreadyCurrent { .. } => {
                panic!("expected Migrated, got {other:?}")
            }
        }

        let db = Db::with_path(&path).await.expect("reopen db");
        assert_eq!(applied_max(&db).await, 3, "schema must now be at 0003");
        assert!(
            index_present(&db).await,
            "idx_strategy_name must exist after migrate"
        );
        assert_eq!(count_backups(tmp.path()), 1, "exactly one backup retained");
    }

    #[tokio::test]
    async fn migration_already_current_takes_no_backup() {
        // AC-12: a db already at the embedded max → AlreadyCurrent, NO backup.
        let tmp = TempDir::new().expect("tempdir");
        let path = tmp.path().join("pulse.db");
        let db = Db::with_path(&path).await.expect("open db");
        MIGRATOR
            .run(db.pool())
            .await
            .expect("bring up to embedded max");
        drop(db);

        let outcome = run_migrations_with_backup(&path)
            .await
            .expect("already-current must succeed");

        match outcome {
            MigrationOutcome::AlreadyCurrent { version } => {
                assert_eq!(version, 3, "version must be the embedded max");
            }
            other @ MigrationOutcome::Migrated { .. } => {
                panic!("expected AlreadyCurrent, got {other:?}")
            }
        }
        assert_eq!(
            count_backups(tmp.path()),
            0,
            "no backup when already current"
        );

        let db = Db::with_path(&path).await.expect("reopen");
        assert!(
            index_present(&db).await,
            "schema unchanged (index still present)"
        );
    }

    #[tokio::test]
    async fn migration_forced_failure_restores_and_refuses_to_start() {
        // AC-13 (FR-4): a deliberately-broken migration source (test-scoped, NOT in
        // the committed migrations/ dir) → restore-on-failure + REFUSE TO START.
        let tmp = TempDir::new().expect("tempdir");
        let path = tmp.path().join("pulse.db");

        // Seed the db at 0001 (a valid first step) so there is a known-good state
        // to restore to and the broken second step makes the source "behind".
        {
            let db = Db::with_path(&path).await.expect("open db");
            MIGRATOR.run(db.pool()).await.expect("run embedded");
            undo_to(db.pool(), 1).await.expect("undo to 1");
        }

        // Build a broken runtime migrator: a temp migrations dir with a valid
        // 0001 (matching the embedded checksum is unnecessary — we point the
        // protocol at THIS migrator) + a syntactically broken 0002.
        let mig_dir = TempDir::new().expect("migrations tempdir");
        std::fs::write(
            mig_dir.path().join("0001_init.up.sql"),
            "CREATE TABLE IF NOT EXISTS _noop_marker (id INTEGER);",
        )
        .unwrap();
        std::fs::write(
            mig_dir.path().join("0002_broken.up.sql"),
            "THIS IS NOT VALID SQL ;;;",
        )
        .unwrap();
        let broken: Migrator = Migrator::new(mig_dir.path())
            .await
            .expect("build runtime migrator");

        let err = run_migrations_with_backup_using(&path, &broken)
            .await
            .expect_err("a broken migration must fail the protocol");
        assert!(
            matches!(err, DataError::Migration(_)),
            "forced failure must surface DataError::Migration, got {err:?}"
        );

        // The backup is retained for forensics (NFR-12).
        assert_eq!(
            count_backups(tmp.path()),
            1,
            "backup must be retained after restore"
        );
        let bak = the_backup(tmp.path());

        // Restore happened: db_path's bytes equal the pre-migration backup (the
        // restore is a byte copy of the backup over db_path). NOTE: this is the
        // backup snapshot's bytes, NOT the live pre-migration WAL-mode file — the
        // `VACUUM INTO` snapshot is logically identical but byte-defragmented
        // (spec §3), so the contract is "restored == backup", which the restore
        // file-copy makes byte-exact.
        let restored_bytes = std::fs::read(&path).expect("read restored db bytes");
        let backup_bytes = std::fs::read(&bak).expect("read backup db bytes");
        assert_eq!(
            restored_bytes, backup_bytes,
            "db must be restored byte-for-byte from the pre-migration backup"
        );
        // The retained backup is itself a logically-valid db at version 1.
        let bak_db = Db::with_path(&bak).await.expect("open backup as db");
        assert_eq!(
            applied_max(&bak_db).await,
            1,
            "backup snapshot is the v1 db"
        );
    }

    #[tokio::test]
    async fn migration_up_down_round_run_undo_rerun() {
        // run → undo_to(1) → re-run. Index gone then back; max 3→1→3.
        let (_tmp, path) = db_at_0001().await;

        // Up: 1 → 3 (the embedded max, now that 0003 ships).
        run_migrations_with_backup(&path).await.expect("up to 0003");
        let db = Db::with_path(&path).await.expect("reopen after up");
        assert_eq!(applied_max(&db).await, 3, "after run, max == 3");
        assert!(index_present(&db).await, "after run, index present");

        // Down: 3 → 1.
        undo_to(db.pool(), 1).await.expect("undo to 1");
        assert_eq!(applied_max(&db).await, 1, "after undo, max == 1");
        assert!(!index_present(&db).await, "after undo, index gone");
        drop(db);

        // Re-run: 1 → 3.
        run_migrations_with_backup(&path)
            .await
            .expect("re-run to 0003");
        let db = Db::with_path(&path).await.expect("reopen after re-run");
        assert_eq!(applied_max(&db).await, 3, "after re-run, max == 3");
        assert!(index_present(&db).await, "after re-run, index back");
    }

    /// Insert a synthetic `_sqlx_migrations` row at `version` with the given
    /// `success` flag (all NOT NULL columns supplied — `description`, `checksum`
    /// BLOB, `execution_time`). Used to fabricate an ahead-of-embedded / failed
    /// future-version state without shipping a real future migration.
    async fn seed_migration_row(db: &Db, version: i64, success: bool) {
        sqlx::query(
            "INSERT INTO _sqlx_migrations \
             (version, description, installed_on, success, checksum, execution_time) \
             VALUES (?1, ?2, CURRENT_TIMESTAMP, ?3, ?4, 0)",
        )
        .bind(version)
        .bind(format!("synthetic future migration {version}"))
        .bind(success)
        .bind(vec![0_u8; 32])
        .execute(db.pool())
        .await
        .expect("seed _sqlx_migrations row");
    }

    #[tokio::test]
    async fn migration_refuses_ahead_of_embedded() {
        // AC-12 / #38 / #65: a db whose SUCCESSFULLY-applied schema is ahead of the
        // binary's embedded max must be REFUSED with a real DataError::Migration Err
        // (refuse to start, MASTER-SPEC §7.4) — and NO backup is taken (the guard
        // fires before the behind-branch).
        let tmp = TempDir::new().expect("tempdir");
        let path = tmp.path().join("pulse.db");
        let db = Db::with_path(&path).await.expect("open db");
        MIGRATOR
            .run(db.pool())
            .await
            .expect("bring up to embedded max");

        let embedded = embedded_max_version(&MIGRATOR);
        // Seed a COMMITTED (success = TRUE) future-version row → applied_max > embedded.
        seed_migration_row(&db, embedded + 1, true).await;
        assert_eq!(
            applied_max_version(db.pool()).await.unwrap(),
            embedded + 1,
            "a committed future row advances applied_max above embedded"
        );
        drop(db);

        let err = run_migrations_with_backup(&path)
            .await
            .expect_err("an ahead-of-embedded db must be refused");
        assert!(
            matches!(err, DataError::Migration(_)),
            "ahead-state refusal must surface DataError::Migration, got {err:?}"
        );
        assert_eq!(
            count_backups(tmp.path()),
            0,
            "no backup is taken on the ahead-state refusal (it precedes the behind-branch)"
        );
    }

    #[tokio::test]
    async fn applied_max_ignores_failed_migration_rows() {
        // AC-15 / audit C6: a FAILED/partial future-version row (success = 0) must
        // NOT count toward applied_max — otherwise one botched future-migration
        // attempt would spuriously trip the ahead-state refusal and brick the binary.
        let tmp = TempDir::new().expect("tempdir");
        let path = tmp.path().join("pulse.db");
        let db = Db::with_path(&path).await.expect("open db");
        MIGRATOR
            .run(db.pool())
            .await
            .expect("bring up to embedded max");

        let embedded = embedded_max_version(&MIGRATOR);
        // Seed a FAILED (success = 0) future-version row.
        seed_migration_row(&db, embedded + 5, false).await;

        // applied_max_version filters on success = TRUE, so the failed row is ignored.
        assert_eq!(
            applied_max_version(db.pool()).await.unwrap(),
            embedded,
            "a failed (success = 0) future row must NOT raise applied_max"
        );
        drop(db);

        // And because applied_max == embedded, the protocol proceeds normally
        // (AlreadyCurrent) rather than spuriously refusing to start.
        let outcome = run_migrations_with_backup(&path)
            .await
            .expect("a failed future row must not brick the migrator");
        assert!(
            matches!(outcome, MigrationOutcome::AlreadyCurrent { version } if version == embedded),
            "expected AlreadyCurrent at the embedded max, got {outcome:?}"
        );
    }
}
