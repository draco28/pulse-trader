//! GLM transport adapter (VS-1.3.1 work-1.03, FR-23, README C8) — the
//! anti-corruption layer between `PulseTrader`'s OWNED [`LlmProvider`] port and the
//! `PulseHive` OpenAI-compatible transport (ADR-0012 thin transport).
//!
//! [`GlmProvider`] wraps an `OpenAICompatibleProvider` pointed at the Z.AI
//! coding-plan GLM endpoint and translates EVERY `PulseHive` LLM type to/from the
//! PulseTrader-owned domain types, so `PulseHive`'s evolving 2.x API cannot ripple
//! inward. **This is the ONLY module in the crate that imports the `PulseHive` SDK
//! crate (AC-6).**
//!
//! Thin transport ONLY: no `HiveMind`/agent/tool/lens substrate, no streaming (the
//! cost-logged path in 1.04 needs `usage`, which only the non-streaming `chat()`
//! carries), no key env-read (the key is a ctor arg the composition root sources
//! from the Keychain via [`glm_api_key`](crate::adapters::secrets::glm_api_key)),
//! and no redaction / cost / persistence (that is 1.04's decorator).

use pulsehive::error::PulseHiveError;
use pulsehive::llm::{
    LlmConfig as HiveLlmConfig, LlmProvider as HiveLlmProvider, LlmResponse as HiveLlmResponse,
    Message as HiveMessage, TokenUsage as HiveTokenUsage, ToolCall as HiveToolCall,
};
use pulsehive::pulsehive_openai::{OpenAICompatibleProvider, OpenAIConfig};

use crate::domain::{LlmConfig, LlmError, LlmProvider, LlmResponse, Message, TokenUsage, ToolCall};

/// The Z.AI coding-plan OpenAI-compatible base URL (owner-confirmed 2026-07-05).
///
/// `PulseHive`'s `chat_completions_url()` trims a trailing `/` and appends
/// `/chat/completions`, yielding
/// `https://api.z.ai/api/coding/paas/v4/chat/completions`. Kept a single named
/// const (config, not alpha) so an endpoint swap is one edit; it mirrors the
/// canonical `.env` `GLM_BASE_URL`.
const GLM_BASE_URL: &str = "https://api.z.ai/api/coding/paas/v4";

/// The GLM model id (owner-confirmed 2026-07-05 — GLM 5.2 via the Z.AI coding
/// plan). Mirrors the canonical `.env` `GLM_MODEL_ID`.
const GLM_MODEL_ID: &str = "glm-5.2";

/// Request timeout, in seconds (audit ch4 — a stalled provider must not hang a
/// future coach loop forever; an unset/infinite timeout is a v1 reliability gap).
const GLM_TIMEOUT_SECS: u64 = 60;

/// Max retry attempts for transient (429 / 5xx) errors (audit ch4).
const GLM_MAX_RETRIES: u32 = 2;

/// The GLM adapter — implements `PulseTrader`'s [`LlmProvider`] port over the
/// `PulseHive` OpenAI-compatible transport (README C8).
///
/// Holds a pre-built provider pinned to the GLM 5.2 / Z.AI coding-plan config; its
/// [`chat`](GlmProvider::chat) translates domain types across the seam and never
/// touches the network in tests (translation is exercised by pure unit tests; the
/// live round-trip is a manual/demo concern — MASTER-SPEC §9.4).
pub struct GlmProvider {
    inner: OpenAICompatibleProvider,
}

impl GlmProvider {
    /// Build a `GlmProvider` from an API key.
    ///
    /// The key is a **ctor argument** (never env-read here) — the composition root
    /// (1.05) sources it from the macOS Keychain via
    /// [`glm_api_key`](crate::adapters::secrets::glm_api_key) and injects it. The
    /// endpoint / model / timeout / retry posture is pinned to the Z.AI
    /// coding-plan GLM 5.2 config (README C8, audit ch4).
    #[must_use]
    pub fn new(api_key: impl Into<String>) -> Self {
        let config = OpenAIConfig::new(api_key, GLM_MODEL_ID)
            .with_base_url(GLM_BASE_URL)
            .with_timeout(GLM_TIMEOUT_SECS)
            .with_max_retries(GLM_MAX_RETRIES);
        Self {
            inner: OpenAICompatibleProvider::new(config),
        }
    }
}

impl LlmProvider for GlmProvider {
    async fn chat(
        &self,
        messages: Vec<Message>,
        config: &LlmConfig,
    ) -> Result<LlmResponse, LlmError> {
        let hive_messages: Vec<HiveMessage> = messages.into_iter().map(to_hive_message).collect();
        let hive_config = to_hive_config(config);
        // Non-streaming ONLY (v1): only `chat()` carries `usage`. The v1 port takes
        // no tools IN, so an empty tool list crosses the seam (composer tools are
        // VS-1.3.2). `PulseHive`'s LLM/transport error maps to `LlmError::Provider`.
        let response = self
            .inner
            .chat(hive_messages, Vec::new(), &hive_config)
            .await
            .map_err(map_hive_error)?;
        Ok(from_hive_response(response))
    }
}

/// Translate a `PulseTrader` [`Message`] into the `PulseHive` wire message.
fn to_hive_message(message: Message) -> HiveMessage {
    match message {
        Message::System { content } => HiveMessage::System { content },
        Message::User { content } => HiveMessage::User { content },
        Message::Assistant {
            content,
            tool_calls,
        } => HiveMessage::Assistant {
            content,
            tool_calls: tool_calls.into_iter().map(to_hive_tool_call).collect(),
        },
        Message::ToolResult {
            tool_call_id,
            content,
        } => HiveMessage::ToolResult {
            tool_call_id,
            content,
        },
    }
}

/// Translate a `PulseTrader` [`ToolCall`] into the `PulseHive` tool call (same wire
/// shape: `id` / `name` / opaque JSON `arguments`).
fn to_hive_tool_call(tool_call: ToolCall) -> HiveToolCall {
    HiveToolCall {
        id: tool_call.id,
        name: tool_call.name,
        arguments: tool_call.arguments,
    }
}

/// Translate a `PulseTrader` [`LlmConfig`] into the `PulseHive` request config.
///
/// `temperature` is `f32` on BOTH sides — no conversion. `provider` is a routing
/// label unused on a direct `OpenAICompatibleProvider` call (set to the backend
/// tag for legibility); `model` flows through (the composition root sets
/// `"glm-5.2"`, matching [`GLM_MODEL_ID`], which `OpenAIConfig` also carries as the
/// fallback).
fn to_hive_config(config: &LlmConfig) -> HiveLlmConfig {
    HiveLlmConfig {
        provider: "glm".to_owned(),
        model: config.model.clone(),
        temperature: config.temperature,
        max_tokens: config.max_tokens,
    }
}

/// Translate a `PulseHive` [`LlmResponse`](HiveLlmResponse) back into the
/// PulseTrader-owned response.
fn from_hive_response(response: HiveLlmResponse) -> LlmResponse {
    LlmResponse {
        content: response.content,
        tool_calls: response
            .tool_calls
            .into_iter()
            .map(from_hive_tool_call)
            .collect(),
        usage: from_hive_usage(&response.usage),
    }
}

/// Translate a `PulseHive` tool call back into the PulseTrader-owned one.
fn from_hive_tool_call(tool_call: HiveToolCall) -> ToolCall {
    ToolCall {
        id: tool_call.id,
        name: tool_call.name,
        arguments: tool_call.arguments,
    }
}

/// Translate `PulseHive` token usage into the PulseTrader-owned usage (the
/// cost-model input consumed by 1.04).
fn from_hive_usage(usage: &HiveTokenUsage) -> TokenUsage {
    TokenUsage {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
    }
}

/// Map a [`PulseHiveError`] into the `PulseTrader` port error.
///
/// The thin transport only ever yields `PulseHiveError::Llm` (every error path in
/// the OpenAI-compatible provider's `chat` uses it); it maps to
/// [`LlmError::Provider`], preserving the message verbatim. Any other variant (not
/// reachable on this path) also maps to `Provider` defensively, so the mapping is
/// total and the domain never learns `PulseHive`'s error type.
fn map_hive_error(error: PulseHiveError) -> LlmError {
    match error {
        PulseHiveError::Llm(message) => LlmError::Provider(message),
        other => LlmError::Provider(other.to_string()),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{
        GLM_BASE_URL, GLM_MODEL_ID, GlmProvider, from_hive_response, map_hive_error,
        to_hive_config, to_hive_message,
    };
    use crate::domain::{LlmBackend, LlmConfig, LlmError, Message, ToolCall};
    use pulsehive::error::PulseHiveError;
    use pulsehive::llm::{
        LlmResponse as HiveLlmResponse, Message as HiveMessage, TokenUsage as HiveTokenUsage,
        ToolCall as HiveToolCall,
    };

    fn sample_config() -> LlmConfig {
        LlmConfig {
            backend: LlmBackend::Glm,
            model: "glm-5.2".to_owned(),
            temperature: 0.3,
            max_tokens: 256,
        }
    }

    #[test]
    fn provider_constructs_with_pinned_glm_config() {
        // Smoke test: the adapter builds against the pinned GLM 5.2 / Z.AI
        // coding-plan config with NO network. The consts are the owner-confirmed
        // endpoint + model id (README C8).
        let _provider = GlmProvider::new("test-key");
        assert_eq!(GLM_BASE_URL, "https://api.z.ai/api/coding/paas/v4");
        assert_eq!(GLM_MODEL_ID, "glm-5.2");
    }

    #[test]
    fn to_hive_message_preserves_every_variant() {
        match to_hive_message(Message::system("sys")) {
            HiveMessage::System { content } => assert_eq!(content, "sys"),
            other => panic!("expected System, got {other:?}"),
        }
        match to_hive_message(Message::user("hi")) {
            HiveMessage::User { content } => assert_eq!(content, "hi"),
            other => panic!("expected User, got {other:?}"),
        }
        match to_hive_message(Message::assistant("ok")) {
            HiveMessage::Assistant {
                content,
                tool_calls,
            } => {
                assert_eq!(content.as_deref(), Some("ok"));
                assert!(tool_calls.is_empty());
            }
            other => panic!("expected Assistant, got {other:?}"),
        }
        match to_hive_message(Message::tool_result("call-1", "42")) {
            HiveMessage::ToolResult {
                tool_call_id,
                content,
            } => {
                assert_eq!(tool_call_id, "call-1");
                assert_eq!(content, "42");
            }
            other => panic!("expected ToolResult, got {other:?}"),
        }
    }

    #[test]
    fn to_hive_message_translates_assistant_tool_calls() {
        let message = Message::Assistant {
            content: None,
            tool_calls: vec![ToolCall {
                id: "call-1".to_owned(),
                name: "search".to_owned(),
                arguments: serde_json::json!({"q": "btc"}),
            }],
        };
        match to_hive_message(message) {
            HiveMessage::Assistant {
                content,
                tool_calls,
            } => {
                assert!(content.is_none());
                assert_eq!(tool_calls.len(), 1);
                assert_eq!(tool_calls[0].id, "call-1");
                assert_eq!(tool_calls[0].name, "search");
                assert_eq!(tool_calls[0].arguments["q"], "btc");
            }
            other => panic!("expected Assistant, got {other:?}"),
        }
    }

    #[test]
    fn to_hive_config_maps_fields_without_temperature_conversion() {
        let hive = to_hive_config(&sample_config());
        assert_eq!(hive.model, "glm-5.2");
        assert!((hive.temperature - 0.3).abs() < f32::EPSILON);
        assert_eq!(hive.max_tokens, 256);
    }

    #[test]
    fn from_hive_response_maps_content_usage_and_tool_calls() {
        let hive = HiveLlmResponse {
            content: Some("pong".to_owned()),
            tool_calls: vec![HiveToolCall {
                id: "c1".to_owned(),
                name: "noop".to_owned(),
                arguments: serde_json::json!({}),
            }],
            usage: HiveTokenUsage {
                input_tokens: 11,
                output_tokens: 4,
            },
        };
        let response = from_hive_response(hive);
        assert_eq!(response.content.as_deref(), Some("pong"));
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].id, "c1");
        assert_eq!(response.tool_calls[0].name, "noop");
        assert_eq!(response.usage.input_tokens, 11);
        assert_eq!(response.usage.output_tokens, 4);
    }

    #[test]
    fn map_hive_error_llm_becomes_provider_verbatim() {
        let err = map_hive_error(PulseHiveError::Llm("upstream 500".to_owned()));
        assert!(
            matches!(&err, LlmError::Provider(message) if message == "upstream 500"),
            "expected Provider(\"upstream 500\"), got {err:?}"
        );
    }
}
