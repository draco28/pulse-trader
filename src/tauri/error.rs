//! The **one** error shape that crosses the Tauri boundary (ADR-0020, bus contract
//! clause 1).
//!
//! Every domain error family the desktop can provoke — `DataError` first, and
//! `ValidationErrors`, `BacktestError`, `ExchangeError`, `LlmError`, `ComposerError`
//! behind it — maps to a single [`BusError`]. The frontend renders errors with one code
//! path and never sniffs which family an error came from.
//!
//! **Three things deliberately never cross.**
//!
//! - **A stringified `Debug`.** [`BusError::message`] is the source error's `Display`
//!   rendering, which is prose a human can read. `Debug` leaks Rust variant syntax and
//!   sometimes internal payloads.
//! - **A panic.** Commands return `Result<_, BusError>`; nothing in the bus unwraps.
//!   The crate-wide `clippy::unwrap_used` / `expect_used` denials hold that structurally.
//! - **A numeric discriminant.** [`BusErrorCode`] serializes as a string, so inserting a
//!   variant cannot silently renumber the ones the frontend already branches on.
//!
//! `tests/tauri_bus_contract.rs::domain_error_maps_to_one_serializable_shape` (AC-4) is
//! the gate on all of it.

use serde::{Deserialize, Serialize};

use crate::agent::ComposerError;
use crate::domain::{BacktestError, DataError, ExchangeError, LlmError, ValidationErrors};

/// Which family a [`BusError`] came from — a closed set the frontend can branch on.
///
/// Serializes as a lower-case string discriminant (`"data"`, `"backtest"`, ...), never
/// as an index, so a variant inserted later cannot renumber the existing ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum BusErrorCode {
    /// The data pipeline / persistence layer (`DataError`).
    Data,
    /// Semantic DSL validation (`ValidationErrors`) — the correctable-rejection family.
    Validation,
    /// The backtest engine (`BacktestError`).
    Backtest,
    /// Exchange metadata (`ExchangeError`).
    Exchange,
    /// The LLM transport (`LlmError`).
    Llm,
    /// The composer agent loop (`ComposerError`).
    Composer,
    /// The shell itself: a dead channel, a failed startup, a bug. Not a domain family.
    Internal,
}

/// The single serializable error shape that crosses the Tauri boundary.
///
/// Two fields, always both present, so the TypeScript type generated from this struct
/// describes every error the frontend can receive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct BusError {
    /// The family this error came from.
    pub code: BusErrorCode,
    /// The source error's `Display` rendering — prose, safe to show a user.
    pub message: String,
}

impl BusError {
    /// Construct a [`BusError`] directly. Prefer the `From` impls for domain errors.
    #[must_use]
    pub fn new(code: BusErrorCode, message: String) -> Self {
        Self { code, message }
    }

    /// A shell-level failure that is not a domain error family.
    #[must_use]
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(BusErrorCode::Internal, message.into())
    }
}

impl std::fmt::Display for BusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for BusError {}

/// Generate a `From<$err> for BusError` that carries the source's **`Display`**.
///
/// A macro rather than six hand-written impls so the "Display, never Debug" rule is
/// stated once and cannot drift between families.
macro_rules! bus_error_from {
    ($($source:ty => $code:expr),+ $(,)?) => {
        $(
            impl From<$source> for BusError {
                fn from(err: $source) -> Self {
                    Self::new($code, err.to_string())
                }
            }
        )+
    };
}

bus_error_from! {
    DataError => BusErrorCode::Data,
    ValidationErrors => BusErrorCode::Validation,
    BacktestError => BusErrorCode::Backtest,
    ExchangeError => BusErrorCode::Exchange,
    LlmError => BusErrorCode::Llm,
    ComposerError => BusErrorCode::Composer,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{BusError, BusErrorCode};
    use crate::domain::DataError;

    #[test]
    fn code_serializes_as_a_string_discriminant() {
        let json = serde_json::to_value(BusErrorCode::Backtest).unwrap();
        assert_eq!(json, serde_json::json!("backtest"));
    }

    #[test]
    fn display_is_the_message_so_anyhow_context_reads_cleanly() {
        let err = BusError::from(DataError::Parse("nope".to_owned()));
        assert_eq!(err.to_string(), "parse error: nope");
    }
}
