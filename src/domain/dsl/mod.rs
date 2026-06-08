//! Strategy DSL grammar (inner-ring, zero-I/O).
//!
//! The DSL models a trading strategy as **data**: serde-tagged Rust enums that
//! are the deterministic contract between the thin LLM layer (which composes
//! strategies via builder tools, FR-3) and the Rust engine (which executes
//! them). This module is the foundational **leaf + predicate layer**:
//!
//! - [`SweepableValue`] — a tunable numeric leaf (fixed now; sweepable in v2).
//! - [`ValueSource`] / [`PriceField`] / [`IndicatorSpec`] — where a scalar comes
//!   from (a constant, a candle field, or an indicator output).
//! - [`Condition`] / [`Comparator`] — the boolean predicate tree over values.
//!
//! Later items compose these: 2.02 wraps them in a top-level `StrategyDsl`, 2.03
//! validates, 2.04 compiles to an evaluator tree, 2.05 versions/migrates. This
//! item guarantees only the grammar shape + its serde **round-trip** (value
//! equality, not byte-canonical JSON); no evaluation, no indicator math, no
//! semantic validation lives here.
//!
//! **serde invariant (load-bearing):** the internally-tagged enums
//! ([`ValueSource`], [`IndicatorSpec`], [`Condition`]) use **only struct
//! variants** — serde cannot serialize an internally-tagged tuple/newtype
//! variant wrapping a `Vec`/scalar/enum. [`SweepableValue`] is `#[serde(untagged)]`
//! so `Fixed` is a bare value and `Sweep` is an object.

mod condition;
mod sweepable;
mod value;

pub use condition::{Comparator, Condition};
pub use sweepable::SweepableValue;
pub use value::{IndicatorSpec, PriceField, ValueSource};
