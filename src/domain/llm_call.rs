//! The `LlmCall` append-only ledger entity + its `LlmCallId` newtype (VS-1.3.1
//! work-1.01, FR-24, README C4).
//!
//! Pure, zero-I/O value types. [`LlmCallId`] is a `#[serde(transparent)]`
//! `String` newtype (matches [`StrategyId`](crate::domain::strategy::StrategyId))
//! so VS-1.3.2 can populate `StrategyVersion.creating_llm_call_ids: Vec<String>`
//! — **this slice adds NO FK / no attribution wiring** (`creating_llm_call_ids`
//! stays `vec![]`). The record stores the VERBATIM prompt **after redaction**
//! (NFR-6, applied by the 1.04 decorator) + the completion + tokens + the
//! `Decimal` cost in its native `cost_currency` (audit ch3).

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::llm::{LlmBackend, Message};
use super::secret::CredentialSource;
use super::strategy::CreatedBy;

/// Identifier of an [`LlmCall`] — a `#[serde(transparent)]` `String` newtype.
///
/// Same discipline as [`StrategyId`](crate::domain::strategy::StrategyId): an
/// opaque adapter-minted string, serialized as a bare JSON string (matching the
/// `TEXT` primary-key column), so it drops straight into
/// `StrategyVersion.creating_llm_call_ids` (VS-1.3.2). `Hash`/`Ord` let an id key
/// a map or sort a ledger.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LlmCallId(String);

impl LlmCallId {
    /// Wrap a raw (adapter-generated) id string.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Borrow the underlying id string (for SQL binding / map keys).
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The append-only LLM-call ledger record (FR-24, README C4).
///
/// One row per provider round-trip. `prompt_messages` + `completion` are stored
/// **verbatim after redaction** (NFR-6 — the 1.04 decorator scrubs the persisted
/// copy, never the sent bytes); `cost` is a `Decimal` in `cost_currency` (the
/// price table's native billing currency — GLM/Zhipu bills RMB/CNY; no silent FX,
/// audit ch3). `created_at` is injected via the `Clock` (RFC3339 on persist);
/// `created_by` reuses the existing [`CreatedBy`] provenance enum.
///
/// `#[serde(deny_unknown_fields)]` (#17 money-safety): a stored row with an extra
/// key is an error, not a silent drop. `PartialEq` but not `Eq` (it carries
/// [`Message`]s, which are not `Eq`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LlmCall {
    /// Primary key (adapter-generated).
    pub id: LlmCallId,
    /// Which backend served the call (stored as its serde tag, e.g. `"ollama"`).
    pub backend: LlmBackend,
    /// The model id, e.g. `"glm-5.2"`.
    pub model: String,
    /// The VERBATIM prompt AFTER redaction (NFR-6) — what we persist.
    pub prompt_messages: Vec<Message>,
    /// The response text AFTER redaction, if any.
    pub completion: Option<String>,
    /// Prompt (input) tokens billed.
    pub input_tokens: u32,
    /// Completion (output) tokens billed.
    pub output_tokens: u32,
    /// The computed cost (1.04 fills it from usage × the price table), a `Decimal`
    /// (NFR-2), in the currency named by `cost_currency`.
    pub cost: Decimal,
    /// The native billing currency of `cost` (e.g. `"CNY"`) — see README C5.
    pub cost_currency: String,
    /// Creation timestamp (adapter-supplied via the injected `Clock`).
    pub created_at: DateTime<Utc>,
    /// Who/what triggered the call (`Human` | `ComposerLlm` | `CoachLlm` | …).
    pub created_by: CreatedBy,
    /// Which credential source supplied the API key for this call — the r1.s1.w2
    /// audit-trail control, so a call's provenance is reconstructible from the
    /// ledger alone.
    ///
    /// A LABEL (`env` / `config-dir` / `cwd-dotenv` / `app-data-dir`), never the key
    /// or any fragment of it. `None` means the provenance was not recorded: either a
    /// row written before migration `0007`, or a caller that supplied none.
    /// `#[serde(default)]` so a pre-`0007` serialized row still deserializes instead
    /// of failing on the absent field — the same backward-compatibility reasoning
    /// that keeps `LLM_CALL_SCHEMA_VERSION` at 1.
    #[serde(default)]
    pub key_source: Option<CredentialSource>,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{LlmCall, LlmCallId};
    use crate::domain::llm::{LlmBackend, Message};
    use crate::domain::secret::CredentialSource;
    use crate::domain::strategy::CreatedBy;
    use chrono::{TimeZone, Utc};
    use rust_decimal::Decimal;

    #[test]
    fn llm_call_id_is_transparent_string() {
        let id = LlmCallId::new("call-abc");
        assert_eq!(id.as_str(), "call-abc");
        let json = serde_json::to_string(&id).expect("serialize LlmCallId");
        // transparent: a bare JSON string, not a `{ "0": ... }` object.
        assert_eq!(json, "\"call-abc\"");
        let back: LlmCallId = serde_json::from_str(&json).expect("deserialize LlmCallId");
        assert_eq!(id, back);
    }

    #[test]
    fn llm_call_roundtrips_verbatim_and_stays_native_currency() {
        let call = LlmCall {
            id: LlmCallId::new("call-1"),
            backend: LlmBackend::Ollama,
            model: "glm-5.2".to_owned(),
            prompt_messages: vec![Message::system("be terse"), Message::user("hello")],
            completion: Some("hi".to_owned()),
            input_tokens: 12,
            output_tokens: 3,
            cost: Decimal::new(1234, 4), // 0.1234
            cost_currency: "CNY".to_owned(),
            created_at: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
            created_by: CreatedBy::ComposerLlm,
            key_source: Some(CredentialSource::ConfigDir),
        };
        let json = serde_json::to_string(&call).expect("serialize LlmCall");
        let back: LlmCall = serde_json::from_str(&json).expect("deserialize LlmCall");
        assert_eq!(call, back);
        // native currency + verbatim prompt survived the round-trip.
        assert_eq!(back.cost_currency, "CNY");
        assert_eq!(back.prompt_messages.len(), 2);
        assert_eq!(back.backend, LlmBackend::Ollama);
    }

    #[test]
    fn llm_call_id_degrades_to_the_string_the_future_field_holds() {
        // C4: LlmCallId is a String newtype so VS-1.3.2 can push into
        // `StrategyVersion.creating_llm_call_ids: Vec<String>`. This slice wires no
        // FK — prove the id degrades cleanly to the String that field holds.
        let ids: Vec<String> = vec![LlmCallId::new("call-1").as_str().to_owned()];
        assert_eq!(ids, vec!["call-1".to_owned()]);
    }
}
