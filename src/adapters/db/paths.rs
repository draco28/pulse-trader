//! Default `SQLite` db-path resolution (VS-1.1.4 work-1.01).
//!
//! Mirrors `store/paths.rs`'s `default_base_dir`: the production path is the
//! platform Application Support directory
//! (`~/Library/Application Support/PulseTrader/pulse.db` on macOS — the
//! MASTER-SPEC `SQLite` location). Tests inject a `tempfile` path via
//! `Db::with_path` instead, so the suite never touches the real `pulse.db`.

use std::path::PathBuf;

use crate::domain::DataError;

/// The application name used to namespace the default Application Support dir.
const APP_DIR: &str = "PulseTrader";

/// The single-file `SQLite` database name under the platform data dir.
const DB_FILE: &str = "pulse.db";

/// Resolve the platform-default `pulse.db` path
/// (`~/Library/Application Support/PulseTrader/pulse.db` on macOS).
///
/// `pub(crate)` (reached only via [`Db::open_default`]); a `pub` fn unused outside
/// the module would trip `dead_code` under `deny(warnings)`.
///
/// # Errors
///
/// Returns [`DataError::Io`] when no platform data directory can be determined
/// (the `directories` crate returns `None`).
///
/// [`Db::open_default`]: crate::adapters::db::Db::open_default
pub(crate) fn default_db_path() -> Result<PathBuf, DataError> {
    let dirs = directories::ProjectDirs::from("", "", APP_DIR)
        .ok_or_else(|| DataError::Io("no platform data directory available".to_string()))?;
    Ok(dirs.data_dir().join(DB_FILE))
}
