//! `IdSource` — the domain port for "a fresh opaque row id" (r1.s4.w4).
//!
//! Every adapter that mints an entity id has, until now, called `Uuid::new_v4()`
//! inline. That is fine while the minted id is incidental. It stops being fine at
//! the coach accept: `commit_acceptance` mints the child `StrategyVersion` id and
//! the `BacktestRun` id INSIDE the write transaction (the caller cannot supply
//! them — see `PreparedCoachAcceptance`), and the in-memory test adapter has to
//! mint "the same way" as the SQLite one for a test to prove anything about which
//! ids the transaction linked together. A hidden `new_v4()` in each adapter cannot
//! be the same way as anything.
//!
//! So id minting becomes an injected dependency exactly as `now` did: the
//! production adapter is a v4 UUID, the test adapter is a deterministic sequence,
//! and the seam is a `<I: IdSource>` bound rather than a `dyn` (NFR-9, mirroring
//! [`Clock`](super::clock::Clock)).
//!
//! **Sync, and infallible.** Minting an id touches no I/O and has no failure mode
//! worth a `Result` — a source that cannot produce an id is a broken program, not a
//! data error. Implementations must be cheap, must not panic, and must be `Send +
//! Sync` so a repository call can be `spawn`ed.
//!
//! **Zero I/O in the domain:** this file holds only the trait; the concrete sources
//! live in `mod adapters`.

/// A source of fresh, opaque row identifiers.
///
/// Uniqueness is the implementation's promise, not the port's: the callers here
/// use the value as a primary key, so a source that repeats has produced a
/// constraint violation rather than a silent overwrite.
pub trait IdSource {
    /// Mint one fresh identifier.
    fn next_id(&self) -> String;
}

#[cfg(test)]
mod tests {
    use super::IdSource;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// A deterministic test source, mirroring the shape `adapters::ids` ships.
    struct CountingSource(AtomicU64);

    impl IdSource for CountingSource {
        fn next_id(&self) -> String {
            format!("id-{}", self.0.fetch_add(1, Ordering::SeqCst))
        }
    }

    #[test]
    fn id_source_port_yields_fresh_ids() {
        let source = CountingSource(AtomicU64::new(0));
        assert_eq!(source.next_id(), "id-0");
        assert_eq!(source.next_id(), "id-1");
    }

    /// The port is consumed generically (`<I: IdSource>`), never as `dyn`,
    /// mirroring the `Clock` / `MarketDataSource` usage discipline (NFR-9).
    fn mint_via<I: IdSource>(source: &I) -> String {
        source.next_id()
    }

    #[test]
    fn id_source_port_is_used_by_bound_not_dyn() {
        assert_eq!(mint_via(&CountingSource(AtomicU64::new(7))), "id-7");
    }
}
