//! OpenAI-compatible transport adapter (VS-1.3.1 work-1.03 → VS-1.3.2 work-2.01,
//! FR-23 / FR-3, README C2/C8) — the anti-corruption layer between
//! `PulseTrader`'s OWNED [`LlmProvider`] port and the `PulseHive`
//! OpenAI-compatible transport (ADR-0012 thin transport).
//!
//! [`OpenAiCompatProvider`] (generalized from VS-1.3.1's `GlmProvider`, now
//! pointed at Z.AI's coding endpoint) wraps an `OpenAICompatibleProvider` and translates
//! EVERY `PulseHive` LLM type to/from the PulseTrader-owned domain types, so
//! `PulseHive`'s evolving 2.x API cannot ripple inward. **This is the ONLY module
//! in the crate that imports the `PulseHive` SDK crate (AC-9).**
//!
//! Thin transport ONLY: no `HiveMind`/agent/lens substrate, no streaming (the
//! cost-logged path in the decorator needs `usage`, which only the non-streaming
//! `chat()` carries), no key env-read (the key is a ctor arg the composition root
//! supplies — `llm-check` reads the Keychain via
//! [`glm_api_key`](crate::adapters::secrets::glm_api_key); `compose` resolves it
//! through
//! [`resolve_llm_api_key`](crate::adapters::secrets::resolve_llm_api_key), the
//! r1.s1.w2 precedence chain — environment, `$PULSE_CONFIG_DIR/.env`, the
//! working/manifest `.env`, then the application data directory, each file
//! permission-validated fail-closed), and no redaction / cost / persistence (that
//! is the redacting-logging decorator).
//!
//! **Tool-calling (VS-1.3.2 work-2.01, FR-3).** `chat` now forwards a borrowed
//! `&[ToolDefinition]` slice — each `PulseTrader` [`ToolDefinition`] is translated to
//! the `PulseHive` type **field-by-field** (the anti-corruption per-field pattern,
//! NOT a serde round-trip), so the composer (2.04) can advertise its builder tools
//! and receive `tool_calls` back. An empty slice reproduces the no-tools behavior.

use pulsehive::error::PulseHiveError;
use pulsehive::llm::{
    LlmConfig as HiveLlmConfig, LlmProvider as HiveLlmProvider, LlmResponse as HiveLlmResponse,
    Message as HiveMessage, TokenUsage as HiveTokenUsage, ToolCall as HiveToolCall,
    ToolDefinition as HiveToolDefinition,
};
use pulsehive::pulsehive_openai::{OpenAICompatibleProvider, OpenAIConfig};

use crate::domain::{
    LlmConfig, LlmError, LlmProvider, LlmResponse, Message, TokenUsage, ToolCall, ToolDefinition,
};

/// The DEFAULT OpenAI-compatible base URL — Z.AI's coding endpoint (provider
/// default flip 2026-08-29; the prior Ollama Cloud default was unusable, that
/// subscription is dropped). The `const` fallback when the config
/// `[llm].base_url` is absent; [`OpenAiCompatProvider::with_base_url`] accepts a
/// config override (slice-close FIX A).
///
/// The `OLLAMA_` prefix on this and the sibling consts is RETAINED naming debt,
/// deliberately not renamed with the default flip so it stays ONE tracked item.
/// Three things still say Ollama while the traffic is z.ai: these consts, the
/// `OLLAMA_API_KEY` credential env var (which the resolver chain still reads
/// first), and [`LlmBackend::Ollama`](crate::domain::LlmBackend::Ollama), the
/// label persisted on every `llm_call` row — that third one is a migration, not a
/// rename, which is why the set moves together or not at all (ADR-0023).
///
/// `PulseHive`'s `chat_completions_url()` trims a trailing `/` and appends
/// `/chat/completions`, yielding
/// `https://api.z.ai/api/coding/paas/v4/chat/completions`.
const OLLAMA_BASE_URL: &str = "https://api.z.ai/api/coding/paas/v4";

/// The default model id — the `OpenAIConfig` fallback carried by the provider. The
/// per-request model actually flows from [`LlmConfig::model`] (the composition root
/// resolves it: config `[llm].model` → const), so this is only the transport-level
/// default. `glm-5.3` is the DEVELOPMENT-CYCLE default (Z.AI coding endpoint,
/// OpenAI-compat, tool-capable), walked end-to-end on 2026-08-28 — the current
/// default pending evaluation, not a final model selection, and not what a
/// distributed build inherits (ADR-0023).
const OLLAMA_MODEL_ID: &str = "glm-5.3";

/// Request timeout, in seconds (audit ch4 — a stalled provider must not hang a
/// future coach loop forever; an unset/infinite timeout is a v1 reliability gap).
const OLLAMA_TIMEOUT_SECS: u64 = 60;

/// Max retry attempts for transient (429 / 5xx) errors (audit ch4).
const OLLAMA_MAX_RETRIES: u32 = 2;

/// The OpenAI-compatible transport adapter — implements `PulseTrader`'s
/// [`LlmProvider`] port over the `PulseHive` OpenAI-compatible transport (README
/// C2/C8), pointed at Z.AI's coding endpoint.
///
/// Holds a pre-built provider pinned to the default transport config; its
/// [`chat`](OpenAiCompatProvider::chat) translates domain types (messages, tools,
/// config, response) across the seam and never touches the network in tests
/// (translation is exercised by pure unit tests; the live round-trip is a
/// manual/demo concern — MASTER-SPEC §9.4).
pub struct OpenAiCompatProvider {
    inner: OpenAICompatibleProvider,
}

impl OpenAiCompatProvider {
    /// Build an `OpenAiCompatProvider` from an API key, pinned to the DEFAULT
    /// [`OLLAMA_BASE_URL`].
    ///
    /// The key is a **ctor argument** (never env-read here) — the composition root
    /// supplies it (`llm-check` from the macOS Keychain via
    /// [`glm_api_key`](crate::adapters::secrets::glm_api_key); `compose` from
    /// [`resolve_llm_api_key`](crate::adapters::secrets::resolve_llm_api_key),
    /// which hands back an opaque `ApiKey` the composition root unwraps exactly
    /// once). Use [`with_base_url`](Self::with_base_url) to override the
    /// endpoint from config (slice-close FIX A). The timeout / retry posture is
    /// pinned to the default transport config (README C2/C8, audit ch4).
    #[must_use]
    pub fn new(api_key: impl Into<String>) -> Self {
        Self::with_base_url(api_key, OLLAMA_BASE_URL)
    }

    /// Build an `OpenAiCompatProvider` from an API key + an explicit OpenAI-compatible
    /// `base_url` (the config `[llm].base_url` override — slice-close FIX A). [`new`]
    /// delegates here with the [`OLLAMA_BASE_URL`] default.
    ///
    /// The key provenance is identical to [`new`](Self::new). The model / timeout /
    /// retry posture is unchanged; only the endpoint is caller-chosen.
    #[must_use]
    pub fn with_base_url(api_key: impl Into<String>, base_url: impl Into<String>) -> Self {
        let config = OpenAIConfig::new(api_key, OLLAMA_MODEL_ID)
            .with_base_url(base_url)
            .with_timeout(OLLAMA_TIMEOUT_SECS)
            .with_max_retries(OLLAMA_MAX_RETRIES);
        Self {
            inner: OpenAICompatibleProvider::new(config),
        }
    }
}

impl LlmProvider for OpenAiCompatProvider {
    async fn chat(
        &self,
        messages: Vec<Message>,
        tools: &[ToolDefinition],
        config: &LlmConfig,
    ) -> Result<LlmResponse, LlmError> {
        let hive_messages: Vec<HiveMessage> = messages.into_iter().map(to_hive_message).collect();
        // Translate the advertised tool defs field-by-field (anti-corruption per-field
        // pattern, NOT a serde round-trip). An empty slice crosses as an empty Vec —
        // the no-tools flow reproduces VS-1.3.1 behavior exactly.
        let hive_tools: Vec<HiveToolDefinition> = tools.iter().map(to_hive_tool_def).collect();
        let hive_config = to_hive_config(config);
        // Non-streaming ONLY (v1): only `chat()` carries `usage`. `PulseHive`'s
        // LLM/transport error maps to `LlmError::Provider`.
        let response = self
            .inner
            .chat(hive_messages, hive_tools, &hive_config)
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

/// Translate a `PulseTrader` [`ToolDefinition`] into the `PulseHive` tool schema
/// **field-by-field** (the anti-corruption per-field pattern — the two types share
/// a shape but are deliberately distinct so `PulseHive`'s API cannot ripple inward,
/// ADR-0012). Borrows the def and clones each field.
fn to_hive_tool_def(tool: &ToolDefinition) -> HiveToolDefinition {
    HiveToolDefinition {
        name: tool.name.clone(),
        description: tool.description.clone(),
        parameters: tool.parameters.clone(),
    }
}

/// Translate a `PulseTrader` [`LlmConfig`] into the `PulseHive` request config.
///
/// `temperature` is `f32` on BOTH sides — no conversion. `provider` is a routing
/// label unused on a direct `OpenAICompatibleProvider` call (set to the backend tag
/// for legibility); `model` flows through (the composition root sets the demo model,
/// which `OpenAIConfig` also carries as the fallback).
fn to_hive_config(config: &LlmConfig) -> HiveLlmConfig {
    HiveLlmConfig {
        provider: "ollama".to_owned(),
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
/// cost-model input consumed by the decorator).
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
        OLLAMA_BASE_URL, OLLAMA_MODEL_ID, OpenAiCompatProvider, from_hive_response, map_hive_error,
        to_hive_config, to_hive_message, to_hive_tool_def,
    };
    use crate::domain::{LlmBackend, LlmConfig, LlmError, Message, ToolCall, ToolDefinition};
    use pulsehive::error::PulseHiveError;
    use pulsehive::llm::{
        LlmResponse as HiveLlmResponse, Message as HiveMessage, TokenUsage as HiveTokenUsage,
        ToolCall as HiveToolCall,
    };

    fn sample_config() -> LlmConfig {
        LlmConfig {
            backend: LlmBackend::Ollama,
            model: "gpt-oss:120b".to_owned(),
            temperature: 0.3,
            max_tokens: 256,
        }
    }

    #[test]
    fn provider_constructs_with_pinned_ollama_config() {
        // Smoke test: the adapter builds against the pinned default config with NO
        // network. The consts are the shipped endpoint + default model id (README
        // C2/C8); `glm-5.3` on Z.AI's coding endpoint is the dev-cycle default.
        let _provider = OpenAiCompatProvider::new("test-key");
        // FIX A: the config `[llm].base_url` override ctor also builds (NO network).
        let _override = OpenAiCompatProvider::with_base_url("test-key", "https://example.test/v1");
        assert_eq!(OLLAMA_BASE_URL, "https://api.z.ai/api/coding/paas/v4");
        assert_eq!(OLLAMA_MODEL_ID, "glm-5.3");
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
    fn to_hive_tool_def_translates_field_by_field() {
        // The anti-corruption per-field translation: name/description/parameters cross
        // the seam verbatim into the (distinct) PulseHive type (README C2, ADR-0012).
        let tool = ToolDefinition {
            name: "set_entry".to_owned(),
            description: "Set the entry condition".to_owned(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": { "indicator": { "type": "string" } }
            }),
        };
        let hive = to_hive_tool_def(&tool);
        assert_eq!(hive.name, "set_entry");
        assert_eq!(hive.description, "Set the entry condition");
        assert_eq!(hive.parameters["type"], "object");
        assert_eq!(hive.parameters["properties"]["indicator"]["type"], "string");
    }

    #[test]
    fn to_hive_config_maps_fields_without_temperature_conversion() {
        let hive = to_hive_config(&sample_config());
        assert_eq!(hive.model, "gpt-oss:120b");
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
