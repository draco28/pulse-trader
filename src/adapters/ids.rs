//! `IdSource` adapters (r1.s4.w4) — the production UUID source and the
//! deterministic test source.
//!
//! Mirrors `adapters::clock`: one wall-behaviour implementation for production, one
//! injectable stand-in whose output a test can assert on. `uuid` stays confined to
//! the adapters ring exactly as it already is in the repositories.

use std::sync::atomic::{AtomicU64, Ordering};

use uuid::Uuid;

use crate::domain::IdSource;

/// The production source: a hyphenated v4 UUID, matching the ids every existing
/// repository mints inline.
#[derive(Debug, Clone, Copy, Default)]
pub struct UuidIdSource;

impl IdSource for UuidIdSource {
    fn next_id(&self) -> String {
        Uuid::new_v4().to_string()
    }
}

/// A deterministic test source: `"<prefix>-0"`, `"<prefix>-1"`, …
///
/// `AtomicU64` rather than a `Cell` because the port takes `&self` and repository
/// calls are `Send` — a test that drives two concurrent accepts through one repo
/// must still get distinct ids, and a non-atomic counter would hand out the same
/// one twice and turn a real race into a confusing primary-key error.
#[derive(Debug)]
pub struct SeqIdSource {
    prefix: String,
    next: AtomicU64,
}

impl SeqIdSource {
    /// A sequence starting at `0` under `prefix`.
    #[must_use]
    pub fn with_prefix(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
            next: AtomicU64::new(0),
        }
    }
}

impl IdSource for SeqIdSource {
    fn next_id(&self) -> String {
        format!(
            "{}-{}",
            self.prefix,
            self.next.fetch_add(1, Ordering::SeqCst)
        )
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{SeqIdSource, UuidIdSource};
    use crate::domain::IdSource;

    #[test]
    fn the_sequence_source_is_deterministic_and_distinct() {
        let ids = SeqIdSource::with_prefix("minted");
        assert_eq!(ids.next_id(), "minted-0");
        assert_eq!(ids.next_id(), "minted-1");
        assert_eq!(ids.next_id(), "minted-2");
    }

    #[test]
    fn the_uuid_source_does_not_repeat() {
        let ids = UuidIdSource;
        let a = ids.next_id();
        let b = ids.next_id();
        assert_ne!(a, b, "a v4 UUID source must not hand out the same id twice");
        assert_eq!(a.len(), 36, "hyphenated v4, matching the existing row ids");
    }
}
