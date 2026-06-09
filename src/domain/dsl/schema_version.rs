//! `SchemaVersion` — the hand-rolled three-integer semver of the strategy DSL
//! document.
//!
//! A deliberately minimal `{ major, minor, patch }` triple (grill branch 2):
//! the `semver` crate's range/pre-release machinery is YAGNI for a 3-integer
//! version (MASTER-SPEC §A1). Ordering is derived (field order
//! major→minor→patch gives correct version comparison). The **JSON
//! representation is a string** `"MAJOR.MINOR.PATCH"` (e.g. `"1.0.0"`), NOT a
//! `{ "major": … }` object — serde routes through [`Display`]/[`FromStr`] via
//! `#[serde(into/try_from)]`.
//!
//! Scope (2.02): the type + `Ord` + string serde + [`SchemaVersion::CURRENT`].
//! Major-vs-minor breaking-change classification and the migration registry are
//! **2.05's** (the version-safe read-path). A malformed version string is a
//! `FromStr` `Err`, which folds into serde rejection (AC-8).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// The semantic version of a strategy DSL document.
///
/// Ordering is derived from the field declaration order (major, then minor,
/// then patch), which is exactly version-precedence order. Serialized as a
/// `"MAJOR.MINOR.PATCH"` JSON **string** (not an object) through
/// [`Display`]/[`FromStr`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub struct SchemaVersion {
    /// Major version — incremented on a breaking grammar change (2.05 owns the
    /// classification + migration).
    pub major: u16,
    /// Minor version — incremented on a backward-compatible additive change
    /// (e.g. a new `IndicatorSpec` variant).
    pub minor: u16,
    /// Patch version.
    pub patch: u16,
}

impl SchemaVersion {
    /// The schema version this build of the DSL writes (`1.0.0`).
    pub const CURRENT: SchemaVersion = SchemaVersion {
        major: 1,
        minor: 0,
        patch: 0,
    };
}

impl fmt::Display for SchemaVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// The error returned when a `"MAJOR.MINOR.PATCH"` string cannot be parsed into
/// a [`SchemaVersion`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("malformed schema version {0:?}: expected \"MAJOR.MINOR.PATCH\" (three u16 integers)")]
pub struct SchemaVersionParseError(String);

impl FromStr for SchemaVersion {
    type Err = SchemaVersionParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || SchemaVersionParseError(s.to_owned());
        let mut parts = s.split('.');
        let major = parts.next().ok_or_else(err)?;
        let minor = parts.next().ok_or_else(err)?;
        let patch = parts.next().ok_or_else(err)?;
        // Exactly three dot-separated components — reject "1.0.0.0".
        if parts.next().is_some() {
            return Err(err());
        }
        Ok(SchemaVersion {
            major: major.parse().map_err(|_| err())?,
            minor: minor.parse().map_err(|_| err())?,
            patch: patch.parse().map_err(|_| err())?,
        })
    }
}

impl From<SchemaVersion> for String {
    fn from(v: SchemaVersion) -> String {
        v.to_string()
    }
}

impl TryFrom<String> for SchemaVersion {
    type Error = SchemaVersionParseError;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::SchemaVersion;
    use std::str::FromStr;

    /// AC-7: `CURRENT` serializes to a `"MAJOR.MINOR.PATCH"` string and
    /// round-trips; deserializing `"1.0.0"` equals `CURRENT`.
    #[test]
    fn schema_version_current_serializes_semver() {
        let json = serde_json::to_string(&SchemaVersion::CURRENT).expect("serialize CURRENT");
        // A bare JSON string, not an object.
        assert_eq!(json, "\"1.0.0\"");

        let back: SchemaVersion = serde_json::from_str(&json).expect("deserialize CURRENT");
        assert_eq!(back, SchemaVersion::CURRENT);

        // Deserializing the literal "1.0.0" yields CURRENT.
        let from_literal: SchemaVersion =
            serde_json::from_str("\"1.0.0\"").expect("deserialize \"1.0.0\"");
        assert_eq!(from_literal, SchemaVersion::CURRENT);
    }

    #[test]
    fn ordering_is_major_minor_patch() {
        let v100 = SchemaVersion {
            major: 1,
            minor: 0,
            patch: 0,
        };
        let v110 = SchemaVersion {
            major: 1,
            minor: 1,
            patch: 0,
        };
        let v200 = SchemaVersion {
            major: 2,
            minor: 0,
            patch: 0,
        };
        let v101 = SchemaVersion {
            major: 1,
            minor: 0,
            patch: 1,
        };
        assert!(v100 < v101);
        assert!(v101 < v110);
        assert!(v110 < v200);
    }

    #[test]
    fn malformed_version_strings_are_err() {
        // Too few components.
        assert!(SchemaVersion::from_str("1.0").is_err());
        // Non-numeric.
        assert!(SchemaVersion::from_str("abc").is_err());
        assert!(SchemaVersion::from_str("1.x.0").is_err());
        // Too many components.
        assert!(SchemaVersion::from_str("1.0.0.0").is_err());
        // Empty.
        assert!(SchemaVersion::from_str("").is_err());

        // serde rejection folds through FromStr.
        let bad: Result<SchemaVersion, _> = serde_json::from_str("\"1.0\"");
        assert!(bad.is_err());
    }
}
