//! `CandleStore` — immutable, content-versioned Parquet persistence for
//! `CandleSeries` (FR-5, NFR-2, NFR-9; BACKLOG-1).
//!
//! Storage sits behind this port-shaped struct so it stays swappable (NFR-9).
//! Snapshots are immutable and content-addressed: `data_version` is a SHA-256
//! content hash (audit C7), so re-writing identical data targets the same path
//! and is a no-op success (audit C5). Writes are atomic (temp → fsync → rename,
//! audit C8) so an interrupted write never publishes a partial snapshot. For a
//! fixed writer version the encoded bytes are byte-stable (audit C6).

mod parquet;
mod paths;
mod version;

pub use parquet::SnapshotProvenance;

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::domain::CANDLE_SCHEMA_VERSION;
use crate::domain::{Candle, CandleSeries, DataError, DataVersion, Pair, Timeframe};

/// A store of immutable, content-versioned `CandleSeries` Parquet snapshots.
///
/// The base directory is injectable (AC-3): production uses the platform
/// Application Support dir; tests inject a `tempfile` dir.
#[derive(Debug, Clone)]
pub struct CandleStore {
    base_dir: PathBuf,
}

impl CandleStore {
    /// Construct a store rooted at an explicit base directory (AC-3).
    #[must_use]
    pub fn with_base_dir(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    /// Construct a store rooted at the platform default Application Support dir.
    ///
    /// # Errors
    ///
    /// Returns [`DataError::Io`] if no platform data directory can be resolved.
    pub fn with_default_base_dir() -> Result<Self, DataError> {
        Ok(Self {
            base_dir: paths::default_base_dir()?,
        })
    }

    /// Compute the content-hash `data_version` for a candle set under the current
    /// `CANDLE_SCHEMA_VERSION` (audit C7).
    #[must_use]
    pub fn content_version(pair: &Pair, tf: Timeframe, candles: &[Candle]) -> DataVersion {
        version::content_version(pair, tf, CANDLE_SCHEMA_VERSION, candles)
    }

    /// Compute the content-hash `data_version` under an explicit schema version.
    ///
    /// Exposed so callers (and tests) can prove that bumping the schema version
    /// changes the `data_version` (AC-5).
    #[must_use]
    pub fn content_version_with_schema(
        pair: &Pair,
        tf: Timeframe,
        schema_version: u32,
        candles: &[Candle],
    ) -> DataVersion {
        version::content_version(pair, tf, schema_version, candles)
    }

    /// Resolve the on-disk path for a snapshot (AC-3):
    /// `<base>/candles/<PAIR>/<TF>/<data_version>.parquet`.
    #[must_use]
    pub fn snapshot_path(&self, pair: &Pair, tf: Timeframe, version: &DataVersion) -> PathBuf {
        paths::snapshot_path(&self.base_dir, pair, tf, version)
    }

    /// Encode a series to in-memory Parquet bytes (no I/O).
    ///
    /// # Errors
    ///
    /// Returns [`DataError`] if the series is structurally invalid or encoding
    /// fails.
    pub fn encode_snapshot(&self, series: &CandleSeries) -> Result<Vec<u8>, DataError> {
        parquet::encode(series, CANDLE_SCHEMA_VERSION)
    }

    /// Normalize writer-version footer metadata for a byte-stability comparison
    /// (audit C6 / AC-2).
    ///
    /// # Errors
    ///
    /// Returns [`DataError::Io`] if the bytes cannot be parsed as Parquet.
    pub fn normalize_writer_metadata(bytes: &[u8]) -> Result<Vec<u8>, DataError> {
        parquet::normalize_writer_metadata(bytes)
    }

    /// Write a snapshot atomically (audit C8).
    ///
    /// Content-addressed and idempotent (audit C5): if a byte-identical snapshot
    /// already exists at the target path, this is a **no-op success**. If a
    /// snapshot exists at the path but differs, returns
    /// [`DataError::SnapshotExists`].
    ///
    /// # Errors
    ///
    /// Returns [`DataError`] on validation failure, a same-path-different-content
    /// collision, or any filesystem error.
    pub fn write_snapshot(&self, series: &CandleSeries) -> Result<(), DataError> {
        let path = self.snapshot_path(&series.pair, series.timeframe, &series.version);
        let bytes = self.encode_snapshot(series)?;

        if path.exists() {
            return reconcile_existing(&path, &bytes);
        }
        publish_atomically(&path, &bytes)
    }

    /// Test-only: write the temp file but skip the rename, simulating an
    /// interrupted write (AC-7).
    ///
    /// # Errors
    ///
    /// Returns [`DataError`] on encode or filesystem failure.
    pub fn write_temp_only_for_test(&self, series: &CandleSeries) -> Result<(), DataError> {
        let path = self.snapshot_path(&series.pair, series.timeframe, &series.version);
        let bytes = self.encode_snapshot(series)?;
        let dir = parent_of(&path)?;
        fs::create_dir_all(dir).map_err(|e| io(&format!("create dir {}", dir.display()), &e))?;
        let tmp = temp_path(&path)?;
        write_temp(&tmp, &bytes)
    }

    /// Read a snapshot back into a `CandleSeries` (AC-1).
    ///
    /// # Errors
    ///
    /// Returns [`DataError`] if the snapshot is absent or cannot be decoded.
    pub fn read_snapshot(
        &self,
        pair: &Pair,
        tf: Timeframe,
        version: &DataVersion,
    ) -> Result<CandleSeries, DataError> {
        let path = self.snapshot_path(pair, tf, version);
        let bytes =
            fs::read(&path).map_err(|e| io(&format!("read snapshot {}", path.display()), &e))?;
        let candles = parquet::decode_candles(&bytes)?;
        Ok(CandleSeries {
            pair: pair.clone(),
            timeframe: tf,
            version: version.clone(),
            candles,
        })
    }

    /// Read the embedded provenance metadata from a snapshot (AC-8).
    ///
    /// # Errors
    ///
    /// Returns [`DataError`] if the snapshot is absent or carries no provenance.
    pub fn read_provenance(
        &self,
        pair: &Pair,
        tf: Timeframe,
        version: &DataVersion,
    ) -> Result<SnapshotProvenance, DataError> {
        let path = self.snapshot_path(pair, tf, version);
        let bytes =
            fs::read(&path).map_err(|e| io(&format!("read snapshot {}", path.display()), &e))?;
        parquet::decode_provenance(&bytes)
    }

    /// Whether a snapshot exists for `(pair, tf, version)` (AC-6).
    #[must_use]
    pub fn snapshot_exists(&self, pair: &Pair, tf: Timeframe, version: &DataVersion) -> bool {
        self.snapshot_path(pair, tf, version).is_file()
    }

    /// The most-recently-written snapshot version for `(pair, tf)`, by file mtime
    /// (AC-6). Returns `Ok(None)` when no snapshot exists.
    ///
    /// # Errors
    ///
    /// Returns [`DataError::Io`] on a filesystem error while listing snapshots.
    pub fn latest_version(
        &self,
        pair: &Pair,
        tf: Timeframe,
    ) -> Result<Option<DataVersion>, DataError> {
        let dir = paths::timeframe_dir(&self.base_dir, pair, tf);
        if !dir.is_dir() {
            return Ok(None);
        }

        let mut newest: Option<(std::time::SystemTime, DataVersion)> = None;
        for entry in
            fs::read_dir(&dir).map_err(|e| io(&format!("read dir {}", dir.display()), &e))?
        {
            let entry = entry.map_err(|e| io(&format!("dir entry in {}", dir.display()), &e))?;
            let path = entry.path();
            let Some(version) = snapshot_version_of(&path) else {
                continue;
            };
            let mtime = entry
                .metadata()
                .and_then(|m| m.modified())
                .map_err(|e| io(&format!("mtime of {}", path.display()), &e))?;
            if newest.as_ref().is_none_or(|(t, _)| mtime >= *t) {
                newest = Some((mtime, version));
            }
        }
        Ok(newest.map(|(_, v)| v))
    }
}

/// Reconcile a write against an already-present file at `path` (audit C5).
fn reconcile_existing(path: &Path, bytes: &[u8]) -> Result<(), DataError> {
    let existing =
        fs::read(path).map_err(|e| io(&format!("read existing {}", path.display()), &e))?;
    if content_equivalent(&existing, bytes)? {
        // Identical content ⇒ no-op success ("already up to date").
        return Ok(());
    }
    Err(DataError::SnapshotExists {
        path: path.display().to_string(),
    })
}

/// Write `bytes` to a temp file, fsync, then atomically rename into `path`
/// (audit C8). The temp file is hidden (`.`-prefixed) so a crash between create
/// and rename leaves no visible `*.parquet`.
fn publish_atomically(path: &Path, bytes: &[u8]) -> Result<(), DataError> {
    let dir = parent_of(path)?;
    fs::create_dir_all(dir).map_err(|e| io(&format!("create dir {}", dir.display()), &e))?;

    let tmp = temp_path(path)?;
    write_temp(&tmp, bytes)?;
    fs::rename(&tmp, path).map_err(|e| {
        io(
            &format!("rename {} -> {}", tmp.display(), path.display()),
            &e,
        )
    })?;
    Ok(())
}

/// Write the temp file and fsync it to durable storage (no rename).
fn write_temp(tmp: &Path, bytes: &[u8]) -> Result<(), DataError> {
    let mut file =
        File::create(tmp).map_err(|e| io(&format!("create temp {}", tmp.display()), &e))?;
    file.write_all(bytes)
        .map_err(|e| io(&format!("write temp {}", tmp.display()), &e))?;
    file.sync_all()
        .map_err(|e| io(&format!("fsync temp {}", tmp.display()), &e))?;
    Ok(())
}

/// The hidden temp path co-located with the final path (same dir ⇒ rename is
/// atomic on the same filesystem).
fn temp_path(path: &Path) -> Result<PathBuf, DataError> {
    let file_name = path
        .file_name()
        .ok_or_else(|| {
            DataError::Io(format!(
                "snapshot path has no file name: {}",
                path.display()
            ))
        })?
        .to_string_lossy();
    let dir = parent_of(path)?;
    Ok(dir.join(format!(".{file_name}.tmp")))
}

/// The parent directory of a snapshot path, or a domain error if it has none.
fn parent_of(path: &Path) -> Result<&Path, DataError> {
    path.parent()
        .ok_or_else(|| DataError::Io(format!("snapshot path has no parent: {}", path.display())))
}

/// Extract the `data_version` from a `<version>.parquet` file path, skipping
/// hidden temp files and non-parquet entries.
fn snapshot_version_of(path: &Path) -> Option<DataVersion> {
    let name = path.file_name()?.to_str()?;
    if name.starts_with('.') {
        return None;
    }
    let stem = name.strip_suffix(".parquet")?;
    Some(DataVersion::new(stem))
}

/// Two snapshots are content-equivalent if their writer-normalized bytes match
/// (audit C6). A non-Parquet tamper (which fails to parse) is treated as a
/// difference, not an error, so the collision path (AC-4) is reached.
fn content_equivalent(existing: &[u8], incoming: &[u8]) -> Result<bool, DataError> {
    let Ok(norm_existing) = parquet::normalize_writer_metadata(existing) else {
        return Ok(false);
    };
    let norm_incoming = parquet::normalize_writer_metadata(incoming)?;
    Ok(norm_existing == norm_incoming)
}

/// Build a `DataError::Io` from a context string and an error.
fn io(context: &str, err: &impl std::fmt::Display) -> DataError {
    DataError::Io(format!("{context}: {err}"))
}
