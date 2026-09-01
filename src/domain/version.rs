//! `DataVersion` — newtype tag for an immutable Parquet snapshot.

use serde::{Deserialize, Serialize};

use crate::domain::error::DataError;

/// Identifies an immutable versioned snapshot of a `CandleSeries`.
///
/// WI-01 only defines the *type*; the generation scheme (content hash vs.
/// timestamp vs. monotonic counter) is an ADR-worthy decision deferred to WI-04.
/// ADR-0009 settled it as a content hash, but the tag stays **opaque** here: the
/// domain does not know or assert its shape, so the adapter can change the scheme
/// without a domain edit.
///
/// **Path-component invariant (r1.s3.w2, mirroring [`Pair`](crate::domain::Pair)).**
/// Opaque is not the same as arbitrary. The Parquet adapter joins a tag verbatim
/// into the on-disk snapshot path (`<base>/candles/<PAIR>/<TF>/<tag>.parquet`), so a
/// tag carrying `/`, `\`, NUL, or spelled `.` / `..` would escape or relocate the
/// store root. A tag that crosses a trust boundary — read back out of the database,
/// or about to be persisted — MUST be checked with [`DataVersion::parse`] or
/// [`DataVersion::ensure_path_safe`]. [`DataVersion::new`] stays the unchecked
/// constructor for trusted call sites (the content hash the store itself just
/// derived, test fixtures).
///
/// The check is deliberately a portability rule, not ADR-0009's 16-hex format:
/// asserting the current scheme here would make a future scheme change look like a
/// corruption bug.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DataVersion(String);

impl DataVersion {
    /// Wrap a raw version tag **without validation**.
    ///
    /// For trusted call sites only — a tag the store just derived, or a test
    /// fixture. A tag arriving from storage or from a caller MUST go through
    /// [`DataVersion::parse`].
    #[must_use]
    pub fn new(tag: impl Into<String>) -> Self {
        Self(tag.into())
    }

    /// Wrap a raw version tag, refusing one that is not a safe single path
    /// component.
    ///
    /// # Errors
    ///
    /// Returns [`DataError::Parse`] when the tag is empty, contains `/`, `\` or a
    /// NUL byte, or is exactly `.` or `..`.
    pub fn parse(tag: impl Into<String>) -> Result<Self, DataError> {
        let tag = tag.into();
        Self::check(&tag)?;
        Ok(Self(tag))
    }

    /// Refuse this tag if it is not a safe single path component — the write-side
    /// entry point to the same rule [`DataVersion::parse`] enforces on read.
    ///
    /// # Errors
    ///
    /// Returns [`DataError::Parse`] on the same conditions as
    /// [`DataVersion::parse`].
    pub fn ensure_path_safe(&self) -> Result<(), DataError> {
        Self::check(&self.0)
    }

    /// The one implementation of the path-component rule.
    fn check(tag: &str) -> Result<(), DataError> {
        let unsafe_reason = if tag.is_empty() {
            Some("it is empty")
        } else if tag == "." || tag == ".." {
            Some("it is a relative path component")
        } else if tag.contains('/') || tag.contains('\\') {
            Some("it contains a path separator")
        } else if tag.contains('\0') {
            Some("it contains a NUL byte")
        } else {
            None
        };
        match unsafe_reason {
            None => Ok(()),
            Some(reason) => Err(DataError::Parse(format!(
                "invalid data_version {tag:?}: {reason}; a version tag is joined verbatim \
                 into the snapshot path and must be a single portable path component"
            ))),
        }
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::DataVersion;

    /// The tag shapes the adapter actually mints, plus other harmless ones — the
    /// rule must not become a de-facto assertion of ADR-0009's 16-hex format.
    #[test]
    fn a_safe_single_component_tag_is_accepted() {
        for tag in [
            "b51388284a3a4371",
            "9a1dcf67cf4ea260",
            "v1",
            "2026-09-01T00:00:00Z",
            "mem-8-0-6300000-107",
            "...",
        ] {
            DataVersion::parse(tag)
                .unwrap_or_else(|e| panic!("{tag:?} is a legal single component: {e}"));
            DataVersion::new(tag)
                .ensure_path_safe()
                .unwrap_or_else(|e| panic!("{tag:?} is a legal single component: {e}"));
        }
    }

    /// A tag that is not a single portable path component would escape or relocate
    /// `<base>/candles/<PAIR>/<TF>/<tag>.parquet` when the adapter joins it.
    #[test]
    fn a_tag_that_is_not_a_single_path_component_is_refused() {
        for tag in [
            "",
            ".",
            "..",
            "a/b",
            "/tmp/x",
            "../../../etc/passwd",
            "a\\b",
            "a\0b",
        ] {
            let err = DataVersion::parse(tag)
                .expect_err(&format!("{tag:?} must be refused as a path component"));
            assert!(
                err.to_string().contains("data_version"),
                "the refusal names the field: {err}"
            );
            assert!(
                DataVersion::new(tag).ensure_path_safe().is_err(),
                "{tag:?} must be refused on the write side too"
            );
        }
    }

    /// `new` stays the UNCHECKED constructor — the store's own freshly-derived hash
    /// does not pay for a re-check, and changing that would be a wide blast radius.
    #[test]
    fn new_stays_unchecked() {
        assert_eq!(DataVersion::new("a/b").as_str(), "a/b");
    }
}
