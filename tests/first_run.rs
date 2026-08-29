//! r1.s1.w2 — first run on a clean install (issue #42).
//!
//! `Db::open_default` resolves `~/Library/Application Support/PulseTrader/pulse.db`
//! and hands that path to `Db::with_path`. sqlx's `create_if_missing` creates the
//! database **file** but not its parent **directory**, so on a machine that has
//! never run `pulse` the open fails — and every existing test misses it, because
//! every existing test injects an already-present `tempfile` directory through
//! `--db`. A Finder-launched app has no `--db` flag, which is exactly the moment
//! #42 becomes user-facing, and `r1`'s exit criterion 1 cannot pass without it.
//!
//! **What this file drives, and why that is the real path.** A test must never
//! touch the operator's real Application Support directory, so it cannot call
//! `open_default()` itself. It drives `Db::with_path` against a deeply nested
//! directory that does not exist — which is precisely the call `open_default`
//! makes, with precisely the condition a clean install presents. The directory
//! creation lives in `with_path` for that reason: fixing it in `open_default`
//! alone would leave the seam every other caller uses still broken.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use pulse::{Db, MIGRATOR};
use tempfile::TempDir;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn open_default_creates_a_missing_data_directory() {
    let tmp = TempDir::new().expect("tempdir");

    // Several levels deep and entirely absent — a first run has no `PulseTrader`
    // directory at all, and one `mkdir` would not be enough if the platform data
    // root were missing too.
    let data_dir = tmp.path().join("Library/Application Support/PulseTrader");
    let db_path = data_dir.join("pulse.db");
    assert!(
        !data_dir.exists(),
        "the fixture must start with the data directory ABSENT — otherwise this \
         test passes for the same reason every --db test already did"
    );

    let db = Db::with_path(&db_path)
        .await
        .expect("opening the default db path must create its parent directory");

    assert!(
        data_dir.is_dir(),
        "the parent directory was created: {}",
        data_dir.display()
    );
    assert!(
        db_path.is_file(),
        "the database file was created inside it: {}",
        db_path.display()
    );

    // The pool is usable, not merely constructed: migrations run against it, which
    // is the very next thing `open_migrated` does on a real first run.
    MIGRATOR
        .run(db.pool())
        .await
        .expect("migrations run against the freshly-created database");
}

/// Opening a database whose parent directory ALREADY exists must keep working
/// unchanged — the directory creation is additive, not a new precondition. Every
/// `--db`-injecting test in the suite depends on this.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn opening_into_an_existing_directory_is_unchanged() {
    let tmp = TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("pulse.db");

    let db = Db::with_path(&db_path).await.expect("open db");
    MIGRATOR.run(db.pool()).await.expect("run migrations");

    assert!(db_path.is_file(), "the database file was created");
}
