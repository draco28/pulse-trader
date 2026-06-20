//! Broker adapters (`adapters/broker`) — the `pulse-broker` exchange-metadata
//! home, realized as a **module** this slice (VS-1.2.2 work-2.01).
//!
//! [`BinanceAdapter`] implements the [`ExchangeAdapter`](crate::domain::ExchangeAdapter)
//! port over **pinned BTCUSDT USD-M futures constants**. The consts are pinned
//! here (not fetched) for golden-fixture reproducibility — a networked
//! symbol-filter fetch is a later realism item. The pure
//! [`compute_position_size`](crate::domain::compute_position_size) consumes the
//! returned [`SymbolFilters`](crate::domain::SymbolFilters).

use rust_decimal::Decimal;

use crate::domain::{ExchangeAdapter, ExchangeError, Pair, SymbolFilters};

/// `Binance` USD-M futures exchange adapter (v1: `BTCUSDT` only).
///
/// Returns the **pinned BTCUSDT USD-M futures filters** from
/// `symbol_filters`. Any non-`BTCUSDT` pair yields
/// [`ExchangeError::UnknownSymbol`].
#[derive(Debug, Clone, Copy, Default)]
pub struct BinanceAdapter;

impl BinanceAdapter {
    /// The symbol whose filters this v1 adapter knows.
    const BTCUSDT: &'static str = "BTCUSDT";

    // Pinned Binance USD-M `BTCUSDT` perpetual-futures filters (pinned here for
    // golden-fixture reproducibility — `LOT_SIZE.stepSize` / `LOT_SIZE.minQty` /
    // `MIN_NOTIONAL` / the symbol's max leverage tier).
    //
    // - `lot_step  = 0.001` (LOT_SIZE.stepSize)
    // - `min_qty   = 0.001` (LOT_SIZE.minQty)
    // - `min_notional = 100` (MIN_NOTIONAL)
    // - `max_leverage = 125` (BTCUSDT top leverage tier)

    /// `LOT_SIZE.stepSize` = `0.001`.
    fn lot_step() -> Decimal {
        Decimal::new(1, 3)
    }

    /// `LOT_SIZE.minQty` = `0.001`.
    fn min_qty() -> Decimal {
        Decimal::new(1, 3)
    }

    /// `MIN_NOTIONAL` = `100`.
    fn min_notional() -> Decimal {
        Decimal::new(100, 0)
    }

    /// Max leverage tier = `125`.
    fn max_leverage() -> Decimal {
        Decimal::new(125, 0)
    }

    /// Construct a new adapter. Stateless.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl ExchangeAdapter for BinanceAdapter {
    fn symbol_filters(&self, pair: &Pair) -> Result<SymbolFilters, ExchangeError> {
        if pair.as_str() == Self::BTCUSDT {
            Ok(SymbolFilters {
                lot_step: Self::lot_step(),
                min_qty: Self::min_qty(),
                min_notional: Self::min_notional(),
                max_leverage: Self::max_leverage(),
            })
        } else {
            Err(ExchangeError::UnknownSymbol(pair.as_str().to_owned()))
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::BinanceAdapter;
    use crate::domain::{ExchangeAdapter as _, ExchangeError, Pair, SymbolFilters};
    use rust_decimal::Decimal;

    #[test]
    fn btcusdt_returns_pinned_usdm_filters() {
        let adapter = BinanceAdapter::new();
        let filters = adapter
            .symbol_filters(&Pair::new("BTCUSDT"))
            .expect("BTCUSDT filters");
        assert_eq!(
            filters,
            SymbolFilters {
                lot_step: Decimal::new(1, 3),       // 0.001
                min_qty: Decimal::new(1, 3),        // 0.001
                min_notional: Decimal::new(100, 0), // 100
                max_leverage: Decimal::new(125, 0), // 125
            }
        );
    }

    #[test]
    fn unknown_pair_errors_unknown_symbol() {
        let adapter = BinanceAdapter::default();
        let err = adapter
            .symbol_filters(&Pair::new("ETHUSDT"))
            .expect_err("non-BTCUSDT pair is unknown");
        assert_eq!(err, ExchangeError::UnknownSymbol("ETHUSDT".to_owned()));
    }
}
