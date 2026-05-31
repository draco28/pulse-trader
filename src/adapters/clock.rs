//! Clock adapters (spec §3 audit C6): the production [`SystemClock`] backed by
//! `chrono::Utc`, and a deterministic [`FakeClock`] test double.
//!
//! The [`Clock`](crate::domain::Clock) port itself lives in `mod domain` (zero
//! I/O — trait only); these are the outer-ring implementations.

use crate::domain::Clock;

/// Production [`Clock`]: the wall clock via `chrono::Utc::now`.
///
/// Audit C5: the closed-candle cutoff is tested **only** through [`FakeClock`]
/// (deterministic). `SystemClock` itself gets a single trivial `> 0` smoke and
/// **no** monotonic/timing assertion, so the suite never depends on wall-clock
/// behaviour.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        chrono::Utc::now().timestamp_millis()
    }
}

/// A deterministic [`Clock`] returning a fixed `now`, for offline tests of the
/// closed-candle cutoff (audit C5 — the cutoff is exercised exclusively here).
///
/// Construct with the millisecond instant the test wants "now" to be; the
/// incremental top-up then treats every candle whose `close_time` is `<` this
/// value as closed (persisted) and everything at/after it as still-forming
/// (dropped).
#[derive(Debug, Clone, Copy)]
pub struct FakeClock {
    now_ms: i64,
}

impl FakeClock {
    /// A clock pinned to `now_ms` (UTC epoch milliseconds).
    #[must_use]
    pub fn at(now_ms: i64) -> Self {
        Self { now_ms }
    }
}

impl Clock for FakeClock {
    fn now_ms(&self) -> i64 {
        self.now_ms
    }
}

#[cfg(test)]
mod tests {
    use super::{Clock, FakeClock, SystemClock};

    // ---- AC-6: SystemClock smoke ONLY (no timing assertion, audit C5) -----

    #[test]
    fn system_clock_returns_a_plausible_positive_now() {
        // A single `> 0` smoke. NOT a monotonic/ordering/wall-clock assertion:
        // the cutoff logic is proven deterministically via FakeClock elsewhere.
        assert!(SystemClock.now_ms() > 0);
    }

    // ---- AC-6: FakeClock is deterministic ---------------------------------

    #[test]
    fn fake_clock_returns_exactly_the_injected_instant() {
        let clock = FakeClock::at(1_700_000_900_000);
        assert_eq!(clock.now_ms(), 1_700_000_900_000);
        // Deterministic across repeated reads (the cutoff reads it once/fetch).
        assert_eq!(clock.now_ms(), clock.now_ms());
    }
}
