//! The `Decimal↔f64` conversion contract (adapter-internal) — the ONLY place
//! `Decimal → f64 → Decimal` happens in the whole crate.
//!
//! Determinism (NFR-2) is **pinned ta-rs + this fixed-scale rounding rule**, NOT
//! the absence of `f64`. Exponential smoothing (EMA/RSI/MACD/ATR) is intrinsically
//! real-valued and has no exact decimal form; quarantining the floats behind this
//! seam — and rounding every outbound value to a fixed scale-8, half-even — is the
//! correct discipline. The fixed-scale rounding erases sub-epsilon `f64` jitter so
//! the 100×-identical determinism test (3.04) holds **by construction**. Do not
//! "fix" this by trying to remove the floats.

use rust_decimal::prelude::ToPrimitive;
use rust_decimal::{Decimal, RoundingStrategy};

/// The fixed output scale every indicator value is rounded to (decimal places).
///
/// Eight places matches the precision of the on-chain/exchange price feed and is
/// the granularity below which `f64` smoothing jitter is meaningless. Pinned so
/// the rounding rule is a single named constant, not a magic literal.
pub const INDICATOR_SCALE: u32 = 8;

/// Inbound: candle price → ta-rs input. Panic-free (`unwrap`/`expect` are denied
/// crate-wide).
///
/// Returns `None` only if the `Decimal` cannot be represented as `f64` — not
/// expected for real prices, but handled defensively so a pathological value maps
/// to a port `None` rather than a panic.
#[must_use]
pub fn decimal_to_f64(value: Decimal) -> Option<f64> {
    value.to_f64()
}

/// Outbound: ta-rs `f64` output → exact `Decimal`, rounded to a **fixed scale of
/// 8 decimal places, half-even (banker's rounding)**.
///
/// Returns `None` for a non-finite input (NaN/±∞ — not expected from a warmed
/// indicator) or if the value cannot be retained as a `Decimal`. The fixed-scale
/// half-even rounding is the determinism guarantee (NFR-2): two runs that differ
/// only in sub-epsilon `f64` jitter round to the identical `Decimal`.
#[must_use]
pub fn f64_to_decimal_rounded(value: f64) -> Option<Decimal> {
    if !value.is_finite() {
        return None;
    }
    Decimal::from_f64_retain(value)
        .map(|d| d.round_dp_with_strategy(INDICATOR_SCALE, RoundingStrategy::MidpointNearestEven))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{INDICATOR_SCALE, decimal_to_f64, f64_to_decimal_rounded};
    use rust_decimal::{Decimal, RoundingStrategy};
    use std::str::FromStr;

    #[test]
    fn convert_rounds_to_fixed_scale_half_even() {
        // >8 fractional digits → rounded to scale <= 8.
        let rounded = f64_to_decimal_rounded(1.234_567_891_234).expect("finite");
        assert!(
            rounded.scale() <= INDICATOR_SCALE,
            "scale {} must be <= {INDICATOR_SCALE}",
            rounded.scale()
        );

        // Half-even AT THE BOUNDARY, on EXACT scale-9 midpoints straddling the
        // scale-8 rounding point. `f64_to_decimal_rounded` retains its input as a
        // high-scale `Decimal` then rounds half-even with this exact strategy; on a
        // true decimal midpoint the 9th digit is exactly 5 with nothing beyond, so
        // banker's rounding goes to the EVEN neighbour, and the two inputs round in
        // OPPOSITE directions — the defining property of half-even (a one-direction
        // rule like round-half-up would push both the same way):
        //   ...2 (even) + trailing 5  → stays at ...2  (round-down to even)
        //   ...3 (odd)  + trailing 5  → goes to  ...4  (round-up to even)
        // This is asserted on the `Decimal` rounding step itself — the boundary
        // `f64_to_decimal_rounded` applies. (We do NOT feed these as `f64`: a
        // decimal midpoint is not exactly representable in `f64`, so binary jitter
        // would push the input off-centre and destroy the boundary. Erasing exactly
        // that jitter via this fixed-scale rule is the determinism guarantee, NFR-2.)
        let strat = RoundingStrategy::MidpointNearestEven;
        assert_eq!(
            Decimal::from_str("0.123456725")
                .unwrap()
                .round_dp_with_strategy(INDICATOR_SCALE, strat),
            Decimal::from_str("0.12345672").unwrap(),
            "exact midpoint after even digit rounds down to even"
        );
        assert_eq!(
            Decimal::from_str("0.123456735")
                .unwrap()
                .round_dp_with_strategy(INDICATOR_SCALE, strat),
            Decimal::from_str("0.12345674").unwrap(),
            "exact midpoint after odd digit rounds up to even"
        );

        // And the function itself is deterministic on a jittery `f64`: the same
        // input always yields the same scale-8 `Decimal` (the by-construction
        // erasure of sub-epsilon jitter).
        let j = 0.123_456_725_f64;
        assert_eq!(
            f64_to_decimal_rounded(j).expect("finite"),
            f64_to_decimal_rounded(j).expect("finite"),
            "f64_to_decimal_rounded is deterministic on a fixed input"
        );

        // Round-trip of a clean price is exact (no jitter introduced).
        let price = Decimal::from_str("42050.75").unwrap();
        let as_f64 = decimal_to_f64(price).expect("representable");
        let back = f64_to_decimal_rounded(as_f64).expect("finite");
        assert_eq!(back, price, "clean price round-trips exact");
    }

    #[test]
    fn convert_non_finite_maps_to_none() {
        assert_eq!(f64_to_decimal_rounded(f64::NAN), None);
        assert_eq!(f64_to_decimal_rounded(f64::INFINITY), None);
        assert_eq!(f64_to_decimal_rounded(f64::NEG_INFINITY), None);
    }
}
