//! `Clock` — the domain port for "now" (spec §3 audit C6, grill-locked).
//!
//! WI-1.1.1.03 introduces the incremental top-up's closed-candle cutoff: a
//! candle is persisted iff `close_time < clock.now_ms()`. To keep that cutoff
//! testable without a wall-clock dependency, "now" is abstracted behind this
//! port: `SystemClock` (production, `chrono::Utc::now`) and `FakeClock` (tests)
//! both implement it.
//!
//! **Sync by design (audit C6):** `now_ms` is a plain sync method, not `async`.
//! `SystemClock` and `FakeClock` are trivially sync. A future
//! `BinanceServerClock` that corrects local-clock skew implements the same sync
//! signature by **caching a fetched `/fapi/v1/time` offset at construction**
//! (`now_ms = local_now_ms + cached_offset`) — no async reshape of the port.
//!
//! **Zero I/O in the domain:** this file holds only the trait. The `SystemClock`
//! adapter (which touches the system clock) lives in `mod adapters`.

/// A source of the current UTC time in epoch milliseconds.
///
/// The incremental top-up reads `now_ms()` exactly once per fetch to decide the
/// closed-candle cutoff (`close_time < now_ms` ⇒ persist; otherwise the candle
/// is still forming and is dropped). Implementations must be cheap and must not
/// panic.
pub trait Clock {
    /// The current UTC time, epoch milliseconds.
    fn now_ms(&self) -> i64;
}

#[cfg(test)]
mod tests {
    use super::Clock;

    /// A deterministic test clock returning a fixed `now`.
    struct FixedClock(i64);

    impl Clock for FixedClock {
        fn now_ms(&self) -> i64 {
            self.0
        }
    }

    #[test]
    fn clock_port_returns_injected_now() {
        let clock = FixedClock(1_700_000_000_000);
        assert_eq!(clock.now_ms(), 1_700_000_000_000);
    }

    /// The port is consumed generically (`<C: Clock>`), never as `dyn`, mirroring
    /// the `MarketDataSource` usage discipline (NFR-9).
    fn now_via<C: Clock>(clock: &C) -> i64 {
        clock.now_ms()
    }

    #[test]
    fn clock_port_is_used_by_bound_not_dyn() {
        assert_eq!(now_via(&FixedClock(42)), 42);
    }
}
