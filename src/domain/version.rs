//! `DataVersion` — newtype tag for an immutable Parquet snapshot.

use serde::{Deserialize, Serialize};

/// Identifies an immutable versioned snapshot of a `CandleSeries`.
///
/// WI-01 only defines the *type*; the generation scheme (content hash vs.
/// timestamp vs. monotonic counter) is an ADR-worthy decision deferred to WI-04.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DataVersion(String);

impl DataVersion {
    /// Wrap a raw version tag.
    #[must_use]
    pub fn new(tag: impl Into<String>) -> Self {
        Self(tag.into())
    }

    /// Borrow the underlying version tag.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for DataVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
