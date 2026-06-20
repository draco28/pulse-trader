//! Risk-based position sizing — **re-export shim** (VS-1.2.2 work-2.01).
//!
//! The sizing math moved to the shared, exchange-aware
//! [`crate::domain::sizing`] module (the `pulse-broker` money-math home,
//! BACKLOG-5). VS-1.2.1's inline `position_size` body is now
//! [`risk_capped_qty`](crate::domain::sizing::risk_capped_qty) — moved
//! **verbatim** (with its tests) so there is exactly one arithmetic path
//! (`rust_decimal` is order-sensitive, so a byte-identical move keeps 2.04's
//! golden refreeze attributable to lot-step flooring alone).
//!
//! This file is reduced to a **zero-arithmetic** re-export so `engine.rs` keeps
//! compiling against `position_size` and the existing engine tests keep their
//! VS-1.2.1 sizing behavior until **2.04** rewires the engine onto
//! [`compute_position_size`](crate::domain::sizing::compute_position_size). It
//! carries **no** sizing arithmetic (AC-10 verifies this for the shim).

pub use crate::domain::sizing::risk_capped_qty as position_size;
