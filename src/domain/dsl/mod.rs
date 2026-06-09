//! Strategy DSL grammar (inner-ring, zero-I/O).
//!
//! The DSL models a trading strategy as **data**: serde-tagged Rust enums that
//! are the deterministic contract between the thin LLM layer (which composes
//! strategies via builder tools, FR-3) and the Rust engine (which executes
//! them).
//!
//! **Leaf + predicate layer:**
//! - [`SweepableValue`] — a tunable numeric leaf (fixed now; sweepable in v2).
//! - [`ValueSource`] / [`PriceField`] / [`IndicatorSpec`] — where a scalar comes
//!   from (a constant, a candle field, or an indicator output).
//! - [`Condition`] / [`Comparator`] — the boolean predicate tree over values.
//!
//! **Strategy document layer:**
//! - [`StrategyDsl`] — the top-level strategy: entry signal + filters + exits +
//!   risk + a [`SchemaVersion`]. The thing the LLM composes and the engine runs.
//! - [`ExitRule`] / [`RiskParams`] / [`Direction`] — exit and risk vocabulary.
//! - [`SchemaVersion`] — semver schema tag (string serde; migration is 2.05).
//!
//! Remaining items build on this: 2.03 validates a `StrategyDsl` into a checked
//! form, 2.04 compiles to an evaluator tree, 2.05 adds the version-safe migration
//! read-path. This module guarantees only the grammar shape + its serde
//! **round-trip** (value equality, not byte-canonical JSON); no evaluation, no
//! indicator math, no semantic validation lives here. **Direct deserialize is
//! migration-unaware** — the version-safe loader is 2.05's.
//!
//! **serde invariant (load-bearing):** the internally-tagged enums
//! ([`ValueSource`], [`IndicatorSpec`], [`Condition`]) use **only struct
//! variants** — serde cannot serialize an internally-tagged tuple/newtype
//! variant wrapping a `Vec`/scalar/enum. [`SweepableValue`] is `#[serde(untagged)]`
//! so `Fixed` is a bare value and `Sweep` is an object.

mod condition;
mod exit;
mod risk;
mod schema_version;
mod strategy;
mod sweepable;
mod value;

pub use condition::{Comparator, Condition};
pub use exit::ExitRule;
pub use risk::{Direction, RiskParams};
pub use schema_version::{SchemaVersion, SchemaVersionParseError};
pub use strategy::StrategyDsl;
pub use sweepable::SweepableValue;
pub use value::{IndicatorSpec, PriceField, ValueSource};
