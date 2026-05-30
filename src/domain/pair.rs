//! `Pair` — a trading pair symbol (e.g. `BTCUSDT`).

use serde::{Deserialize, Serialize};

/// A trading-pair symbol in `Binance`'s uppercase, separator-free form
/// (e.g. `BTCUSDT`). A thin newtype so the symbol cannot be confused with an
/// arbitrary string at call sites. v1 only uses `BTCUSDT`; the type is general.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Pair(String);

impl Pair {
    /// Wrap a raw symbol string as a `Pair`.
    #[must_use]
    pub fn new(symbol: impl Into<String>) -> Self {
        Self(symbol.into())
    }

    /// Borrow the underlying symbol string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Pair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
