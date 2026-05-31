//! Snapshot path resolution with an injectable base directory (AC-3).
//!
//! Layout: `<base>/candles/<PAIR>/<TF>/<data_version>.parquet`. The default base
//! is the platform Application Support directory
//! (`~/Library/Application Support/PulseTrader/` on macOS), but production code
//! and tests inject the base explicitly so the suite writes only to a `tempfile`
//! dir and never touches real Application Support.

use std::path::{Path, PathBuf};

use crate::domain::{DataError, DataVersion, Pair, Timeframe};

/// Sub-directory under the base dir that holds all candle snapshots.
const CANDLES_DIR: &str = "candles";

/// The application name used to namespace the default Application Support dir.
const APP_DIR: &str = "PulseTrader";

/// Resolve the platform default base directory (Application Support on macOS).
///
/// # Errors
///
/// Returns [`DataError::Io`] when no platform data directory can be determined
/// (the `directories` crate returns `None`).
pub(crate) fn default_base_dir() -> Result<PathBuf, DataError> {
    let dirs = directories::ProjectDirs::from("", "", APP_DIR)
        .ok_or_else(|| DataError::Io("no platform data directory available".to_string()))?;
    Ok(dirs.data_dir().to_path_buf())
}

/// The directory holding all snapshots for one `(pair, timeframe)`.
pub(crate) fn timeframe_dir(base: &Path, pair: &Pair, tf: Timeframe) -> PathBuf {
    base.join(CANDLES_DIR)
        .join(pair.as_str())
        .join(tf.binance_interval())
}

/// The full path for one snapshot: `<base>/candles/<PAIR>/<TF>/<version>.parquet`.
pub(crate) fn snapshot_path(
    base: &Path,
    pair: &Pair,
    tf: Timeframe,
    version: &DataVersion,
) -> PathBuf {
    timeframe_dir(base, pair, tf).join(format!("{version}.parquet"))
}
