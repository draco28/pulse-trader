//! The `SQLite` persistence tier (VS-1.1.4 work-1.01) — the `Db` pool wrapper +
//! the embedded migration set.
//!
//! `sqlx` lives ONLY in this module tree (the domain stays I/O-free). This item
//! ships the pool + connect-options PRAGMAs + the embedded `0001_init` migration;
//! the repo CRUD (1.03, with `query!` macros + the committed `.sqlx` cache), the
//! domain types/port (1.02), the backup-before-migrate wrapper (1.04), and the
//! CLI (1.05) compose against it next.
//!
//! The constructor pair mirrors `CandleStore`'s `with_base_dir`/
//! `with_default_base_dir` injectable-base discipline: production resolves the
//! real platform path; tests inject a `tempfile` path so the suite never touches
//! the real `pulse.db`.

mod paths;
pub mod strategy_repo;

pub use strategy_repo::SqliteStrategyRepo;

// VS-1.1.4 work-1.04: the backup-before-migrate protocol. Re-export EVERY public
// item — under `#![deny(warnings)]` a `pub` item unused outside its module is a
// `dead_code` BUILD ERROR, not a warning (VS-1.1.2 harvested gotcha). All three
// fns + the outcome enum are surfaced (lib.rs mirrors these). Append-only across
// the parallel R2 items (trivial keep-both with 1.03's `pub mod strategy_repo;`).
pub mod migrate;
pub use migrate::{MigrationOutcome, open_migrated, run_migrations_with_backup, undo_to};

use std::path::Path;
use std::time::Duration;

use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};

use crate::domain::DataError;

/// The number of seconds a contending writer waits for the WAL write lock before
/// failing with `SQLITE_BUSY` (gate-7 C5). Even in a single process the app's
/// async tasks can contend, so the busy-timeout is set on every pooled connection.
const BUSY_TIMEOUT_SECS: u64 = 5;

/// The embedded migration set (FR-4 / NFR-12). `sqlx::migrate!` reads the
/// crate-root `migrations/` directory **at compile time**, so build / test / demo
/// need neither a live DB nor `sqlx-cli` — the migrations travel in the binary.
/// 1.04 drives `MIGRATOR.run(pool)` / `MIGRATOR.undo(pool, target)` in-process;
/// re-exported from `lib.rs` so it (and the integration boundary) can reach it.
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// A thin newtype over a `sqlx::SqlitePool` for the `PulseTrader` `SQLite` tier.
///
/// Every pooled connection inherits WAL + `foreign_keys = ON` + a 5s busy-timeout
/// from the connect options (the shared-contract PRAGMA mandate + gate-7 C5). The
/// constructor pair is the test seam: `with_path` injects an explicit (tempfile)
/// path; `open_default` resolves the platform `pulse.db`.
#[derive(Debug, Clone)]
pub struct Db {
    pool: SqlitePool,
}

impl Db {
    /// Open (creating if absent) a pool at an explicit path — the test seam.
    ///
    /// Builds the connect options with WAL, `foreign_keys = ON`, and a 5s
    /// busy-timeout so every pooled connection inherits them (set on the options
    /// object, not via a stray `PRAGMA`), then connects via
    /// `SqlitePoolOptions::connect_with`.
    ///
    /// # Errors
    ///
    /// Returns [`DataError::Db`] if the pool cannot be opened (the flattened
    /// `sqlx::Error` message).
    pub async fn with_path(path: &Path) -> Result<Self, DataError> {
        let opts = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(BUSY_TIMEOUT_SECS));
        let pool = SqlitePoolOptions::new()
            .connect_with(opts)
            .await
            .map_err(|e| DataError::Db(e.to_string()))?;
        Ok(Self { pool })
    }

    /// Open the pool at the platform-default `pulse.db` path
    /// (`~/Library/Application Support/PulseTrader/pulse.db` on macOS).
    ///
    /// # Errors
    ///
    /// Returns [`DataError::Io`] if no platform data directory is resolvable, or
    /// [`DataError::Db`] if the pool cannot be opened.
    pub async fn open_default() -> Result<Self, DataError> {
        let path = paths::default_db_path()?;
        Self::with_path(&path).await
    }

    /// Borrow the underlying pool (so 1.03's repo can run queries against it).
    #[must_use]
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{Db, MIGRATOR, paths};
    use crate::domain::DataError;
    use tempfile::TempDir;

    /// A `Db` opened at a tempfile path, with the `0001_init` migration applied.
    /// Returns the `TempDir` guard so the scratch DB outlives the test body.
    async fn migrated_db() -> (Db, TempDir) {
        let tmp = TempDir::new().expect("tempdir");
        let path = tmp.path().join("pulse.db");
        let db = Db::with_path(&path)
            .await
            .expect("open db at tempfile path");
        MIGRATOR
            .run(db.pool())
            .await
            .expect("run 0001_init migration");
        (db, tmp)
    }

    /// Insert one `strategy` + one `strategy_version` fixture row (raw
    /// `sqlx::query`, no `query!` macro — this item ships no `.sqlx` cache).
    async fn seed_one_version(db: &Db) {
        sqlx::query("INSERT INTO strategy (id, name, created_at) VALUES (?1, ?2, ?3)")
            .bind("strat-1")
            .bind("Test Strategy")
            .bind("2026-06-14T00:00:00Z")
            .execute(db.pool())
            .await
            .expect("insert strategy row");

        sqlx::query(
            "INSERT INTO strategy_version \
             (id, strategy_id, dsl_schema_version, dsl, dsl_original, version_hash, created_by, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )
        .bind("ver-1")
        .bind("strat-1")
        .bind("1.0.0")
        .bind("{}")
        .bind("{}")
        .bind("deadbeef")
        .bind("Human")
        .bind("2026-06-14T00:00:00Z")
        .execute(db.pool())
        .await
        .expect("insert strategy_version row");
    }

    #[tokio::test]
    async fn default_db_path_ends_in_pulse_db() {
        // The default resolver names the single-file DB; tests still inject a
        // tempfile path, never this real location.
        let path = paths::default_db_path().expect("resolve default db path");
        assert!(
            path.to_string_lossy().ends_with("pulse.db"),
            "default db path must end in pulse.db, got {}",
            path.display()
        );
        assert!(
            path.to_string_lossy().contains("PulseTrader"),
            "default db path must be namespaced under PulseTrader, got {}",
            path.display()
        );
    }

    #[tokio::test]
    async fn with_path_roundtrips_the_pool_constructor() {
        // The explicit-path constructor opens a usable pool at a tempfile path.
        let tmp = TempDir::new().expect("tempdir");
        let path = tmp.path().join("pulse.db");
        let db = Db::with_path(&path).await.expect("open db");
        // A trivial query proves the pool is live.
        let one: i64 = sqlx::query_scalar("SELECT 1")
            .fetch_one(db.pool())
            .await
            .expect("SELECT 1 against the pool");
        assert_eq!(one, 1);
    }

    #[tokio::test]
    async fn db_applies_migrations_and_creates_schema() {
        // AC-6: the embedded 0001_init migration applies in-process (no sqlx-cli,
        // no live DB) and creates the full schema: both tables, both idx_sv_*
        // indexes, and both immutability triggers.
        let (db, _tmp) = migrated_db().await;

        let names: Vec<String> = sqlx::query_scalar(
            "SELECT name FROM sqlite_master \
             WHERE type IN ('table', 'index', 'trigger') ORDER BY name",
        )
        .fetch_all(db.pool())
        .await
        .expect("read sqlite_master");

        for expected in [
            "strategy",
            "strategy_version",
            "idx_sv_strategy_id",
            "idx_sv_parent",
            "strategy_version_no_update",
            "strategy_version_no_delete",
        ] {
            assert!(
                names.iter().any(|n| n == expected),
                "0001_init must create `{expected}`; sqlite_master has {names:?}"
            );
        }
    }

    #[tokio::test]
    async fn strategy_version_update_is_rejected_by_trigger() {
        // AC-7 (FR-4): a raw UPDATE on a strategy_version row is aborted by the
        // BEFORE UPDATE trigger; the RAISE(ABORT, ...) surfaces as DataError::Db
        // whose message contains "strategy_version is immutable".
        let (db, _tmp) = migrated_db().await;
        seed_one_version(&db).await;

        let err: DataError = sqlx::query("UPDATE strategy_version SET dsl = ?1 WHERE id = ?2")
            .bind("{\"mutated\":true}")
            .bind("ver-1")
            .execute(db.pool())
            .await
            .map_err(|e| DataError::Db(e.to_string()))
            .expect_err("UPDATE on an immutable strategy_version must fail");

        match err {
            DataError::Db(msg) => assert!(
                msg.contains("strategy_version is immutable"),
                "trigger ABORT message must surface; got: {msg}"
            ),
            other => panic!("expected DataError::Db, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn strategy_version_delete_is_rejected_by_trigger() {
        // AC-8 (FR-4): SQLite needs a SEPARATE BEFORE DELETE trigger — this pins
        // that BOTH are wired, not just the UPDATE one.
        let (db, _tmp) = migrated_db().await;
        seed_one_version(&db).await;

        let err: DataError = sqlx::query("DELETE FROM strategy_version WHERE id = ?1")
            .bind("ver-1")
            .execute(db.pool())
            .await
            .map_err(|e| DataError::Db(e.to_string()))
            .expect_err("DELETE on an immutable strategy_version must fail");

        match err {
            DataError::Db(msg) => assert!(
                msg.contains("strategy_version is immutable"),
                "trigger ABORT message must surface; got: {msg}"
            ),
            other => panic!("expected DataError::Db, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn pragmas_wal_and_foreign_keys_are_enabled() {
        // AC-9 (gate-7 C5): the connect-options PRAGMAs apply to pooled
        // connections — journal_mode=wal, foreign_keys=1, busy_timeout=5000ms.
        let tmp = TempDir::new().expect("tempdir");
        let path = tmp.path().join("pulse.db");
        let db = Db::with_path(&path).await.expect("open db");

        let journal_mode: String = sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(db.pool())
            .await
            .expect("read PRAGMA journal_mode");
        assert_eq!(
            journal_mode.to_ascii_lowercase(),
            "wal",
            "WAL must be persisted on first connect"
        );

        let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(db.pool())
            .await
            .expect("read PRAGMA foreign_keys");
        assert_eq!(foreign_keys, 1, "foreign_keys must be ON");

        let busy_timeout: i64 = sqlx::query_scalar("PRAGMA busy_timeout")
            .fetch_one(db.pool())
            .await
            .expect("read PRAGMA busy_timeout");
        assert_eq!(
            busy_timeout, 5000,
            "busy_timeout must be 5000ms (gate-7 C5)"
        );
    }
}
