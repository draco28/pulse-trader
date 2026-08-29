//! Shared fixtures for the three coach test binaries (`coach_turn` = demo `d6`,
//! `coach_failures` = demo `d7`, `coach_redaction`).
//!
//! A `tests/<dir>/mod.rs` module rather than a fourth test binary: cargo compiles
//! only `tests/*.rs` as test targets, so this file is linked into each binary that
//! declares `mod coach_support;` and never runs as a suite of its own.
//!
//! Everything here is offline by construction — a scripted provider, a temp
//! `SQLite`, a fixture price table. **No live LLM call exists in any of it**: two
//! of the three binaries are demo-ledger lines re-run at every future spine close.
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use pulse::{
    DataError, LlmBackend, LlmCall, LlmCallCapture, LlmCallId, LlmCallRepository, LlmConfig,
    ModelPrice, PriceTable,
};
use rust_decimal::Decimal;

/// An `LlmCallRepository` decorator that records every minted id into a shared
/// buffer — the test-side equivalent of the CLI's `CapturingRepo`, which is
/// `pub(crate)` and therefore not reachable from an integration test.
///
/// The coach reads this buffer to learn which ledger row its turn produced, the
/// same way the composer does.
pub struct CapturingLlmRepo<R> {
    inner: R,
    ids: LlmCallCapture,
}

impl<R> CapturingLlmRepo<R> {
    pub fn new(inner: R, ids: LlmCallCapture) -> Self {
        Self { inner, ids }
    }
}

impl<R: LlmCallRepository + Send + Sync> LlmCallRepository for CapturingLlmRepo<R> {
    async fn save_call(&self, call: &LlmCall) -> Result<LlmCallId, DataError> {
        // Persist through the real repo FIRST (its clock owns `created_at`), and
        // only share the id once the write actually succeeded.
        let id = self.inner.save_call(call).await?;
        if let Ok(mut ids) = self.ids.lock() {
            ids.push(id.clone());
        }
        Ok(id)
    }

    async fn get_call(&self, id: &LlmCallId) -> Result<Option<LlmCall>, DataError> {
        self.inner.get_call(id).await
    }
}

/// The fixture chat config — a priced model, deterministic settings.
pub fn config() -> LlmConfig {
    LlmConfig {
        backend: LlmBackend::Ollama,
        model: "glm-5.3-flash".to_owned(),
        temperature: 0.0,
        max_tokens: 2_048,
    }
}

/// A fixture price table covering [`config`]'s model. Nominal values — the point
/// is that a cost is computed and recorded, not what it is.
pub fn test_prices() -> PriceTable {
    let mut models = HashMap::new();
    models.insert(
        "glm-5.3-flash".to_owned(),
        ModelPrice {
            input_per_mtok: Decimal::new(1, 0),
            output_per_mtok: Decimal::new(2, 0),
        },
    );
    PriceTable::from_config("CNY", models)
}

/// The canonical RSI-oversold strategy as DSL JSON — the document every coach
/// fixture mutates. Written as JSON (not a built `StrategyDsl`) because
/// `create_version` takes the raw document and routes it through the `Migrator`.
pub fn canonical_dsl_json() -> String {
    serde_json::json!({
        "schema_version": "1.0.0",
        "name": "RSI Oversold",
        "direction": "long",
        // `ValueSource` is tagged `type`; the nested `IndicatorSpec` is tagged
        // `indicator` under the `spec` field — both struct variants, per the
        // DSL-wide serde invariant.
        "entry": {
            "type": "Compare",
            "lhs": { "type": "Indicator", "spec": { "indicator": "Rsi", "period": 14 } },
            "op": "Lt",
            "rhs": { "type": "Constant", "value": "30" }
        },
        "filters": [],
        "exits": [
            { "type": "StopLoss", "distance_pct": "0.05" },
            { "type": "TakeProfit", "target_r": "2" }
        ],
        "risk": { "risk_per_trade_pct": "0.01", "max_leverage": "3" }
    })
    .to_string()
}

/// A shared, empty capture buffer.
pub fn capture() -> LlmCallCapture {
    Arc::new(Mutex::new(Vec::new()))
}
