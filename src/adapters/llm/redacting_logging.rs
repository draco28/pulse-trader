//! The redacting + cost-logging decorator (VS-1.3.1 work-1.04, FR-24 / NFR-6,
//! README C7).
//!
//! [`RedactingLoggingProvider`] wraps ANY inner [`LlmProvider`] (1.05 wraps
//! 1.03's `GlmProvider`) and turns a bare provider call into the audited,
//! cost-logged, leak-at-rest-safe ledger write FR-24 requires — WITHOUT changing
//! what the model receives. On each non-streaming `chat()` it:
//!
//! 1. calls the inner provider with the **real, un-redacted** messages — grill
//!    OQ-A: redaction guards the STORED copy, never the sent bytes (API keys ride
//!    the `Authorization` header inside the transport, and the coach legitimately
//!    needs real numeric context);
//! 2. computes the `Decimal` cost from the response `usage` times the
//!    [`PriceTable`] (README C5), in the table's native billing currency,
//!    fail-closed on an unknown model;
//! 3. persists an [`LlmCall`] whose prompt + completion have been passed through
//!    the [`Redactor`] (a COPY — the inner call already happened on the real
//!    bytes), timestamped from the injected [`Clock`]; and
//! 4. returns the inner [`LlmResponse`] to the caller, unchanged.
//!
//! The [`Redactor`] is deliberately scoped (audit ch1): it strips (a)
//! API-key-shaped tokens and (b) caller-declared tagged secret VALUES, and does
//! NOT free-text-regex numbers/balances (which share no lexical shape — a
//! "strip any number" rule would nuke the coach's context, worse than nothing).
//! Its secret ruleset is DATA loaded via [`Redactor::from_config`] (decision 4),
//! with a minimal safe [`Default`] for tests. Generic over `P`/`R`/`C`, never
//! `dyn` (the established port-composition discipline); tested against fakes,
//! fully offline (MASTER-SPEC section 9.4).

use chrono::DateTime;
use serde_json::Value;
use uuid::Uuid;

use crate::domain::strategy::CreatedBy;
use crate::domain::{
    Clock, CredentialSource, LlmCall, LlmCallId, LlmCallRepository, LlmConfig, LlmError,
    LlmProvider, LlmResponse, Message, PriceTable, TokenUsage, ToolCall, ToolDefinition,
};

/// The placeholder substituted for every redacted span in the persisted copy.
const REDACTED: &str = "«REDACTED»";

/// A scoped, data-driven secret scrubber for the PERSISTED copy of a prompt +
/// completion (NFR-6, README C7, audit ch1).
///
/// v1 strips exactly two things and nothing else:
/// - **API-key-shaped tokens** — a conservative structural pattern (a known key
///   prefix — `sk-`/`sk_`/`pk-`/`ghp_`/`gho_`/`xox`/`akia` — or a long
///   mixed-alphanumeric run), shared with the compose-time scrub via the
///   [`looks_like_secret_token`](crate::domain) kernel so the at-rest copy is
///   never weaker than compose-time (slice-close FIX C). This is detection LOGIC,
///   not secret DATA, so it lives in code.
/// - **Caller-declared tagged secrets** — the exact secret VALUES the caller
///   marks as sensitive (e.g. a live key value), supplied as DATA via
///   [`from_config`](Self::from_config) so no production secret is a public-Rust
///   literal (decision 4).
///
/// It deliberately does **not** strip bare numbers / balances / R-multiples: they
/// share no lexical shape, so a numeric regex would erase the very context the
/// coach (1.3.3) needs. Structural balance/account-ID redaction is deferred to
/// 1.3.2/1.3.3, done at prompt-composition time where the field is known.
#[derive(Clone, Default)]
pub struct Redactor {
    tagged_secrets: Vec<String>,
}

// Hand-written `Debug` that NEVER prints the tagged secret VALUES (they are raw
// secrets — e.g. the live GLM API key is tagged in via `from_config` at the CLI
// composition root). A derived `Debug` would dump them in the clear through any
// `dbg!`/`tracing::debug!(?redactor)`/Debug-deriving wrapper — defeating the whole
// leak-at-rest purpose (slice-close close-audit finding R1-2).
impl std::fmt::Debug for Redactor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Redactor")
            .field(
                "tagged_secrets",
                &format_args!("<{} redacted>", self.tagged_secrets.len()),
            )
            .finish()
    }
}

impl Redactor {
    /// Build a `Redactor` from loaded config DATA: the exact secret VALUES the
    /// caller has tagged as sensitive. API-key-shaped stripping is always on (it
    /// is structural, not data). Production loads the ruleset from the private
    /// config; a test passes a small explicit list. [`Default`] is the minimal
    /// safe redactor (API-key stripping only, no tagged secrets).
    #[must_use]
    pub fn from_config(mut tagged_secrets: Vec<String>) -> Self {
        // Longest-first: the sequential exact-substring replacement in `redact` is
        // order-sensitive, so a shorter tagged secret that is a substring of a
        // longer one must be scrubbed AFTER the longer one — otherwise the short
        // match eats part of the long secret and leaves its tail exposed in the
        // persisted copy (close-review Codex C4). Descending length guarantees the
        // longest match wins.
        tagged_secrets.sort_by_key(|s| std::cmp::Reverse(s.len()));
        Self { tagged_secrets }
    }

    /// Redact one string for the PERSISTED copy: every caller-tagged secret value
    /// and every API-key-shaped token is replaced with the redaction placeholder;
    /// all other text — including numbers/balances — is preserved verbatim.
    #[must_use]
    pub fn redact(&self, text: &str) -> String {
        // 1. Caller-tagged secret VALUES (exact substring) first.
        let mut scrubbed = text.to_owned();
        for secret in &self.tagged_secrets {
            if !secret.is_empty() {
                scrubbed = scrubbed.replace(secret.as_str(), REDACTED);
            }
        }
        // 2. Structural API-key-shaped tokens, preserving every separator.
        Self::redact_api_keys(&scrubbed)
    }

    /// Recursively redact every string leaf of a JSON [`Value`] for the PERSISTED
    /// copy (NFR-6, #81). Object/array structure, object KEYS, and non-string
    /// scalars (numbers/bools/null) are preserved verbatim; each string leaf is
    /// passed through [`redact`](Self::redact) (the tagged-secret + api-key-shape
    /// rules). Used to scrub `Assistant.tool_calls[i].arguments` — a
    /// `serde_json::Value` — before it lands in the ledger copy (defense-in-depth
    /// per audit F2: the composer's builder-tool args are non-secret strategy
    /// params, so this is a general backstop, not a live leak fix).
    #[must_use]
    pub fn redact_value(&self, value: &Value) -> Value {
        match value {
            Value::String(text) => Value::String(self.redact(text)),
            Value::Array(items) => {
                Value::Array(items.iter().map(|item| self.redact_value(item)).collect())
            }
            Value::Object(map) => Value::Object(
                map.iter()
                    .map(|(key, val)| (key.clone(), self.redact_value(val)))
                    .collect(),
            ),
            // Numbers / bools / null carry no string leaf — preserved verbatim (we
            // do NOT strip numbers; that would erase the coach's context).
            other => other.clone(),
        }
    }

    /// Replace API-key-shaped word tokens with the placeholder, keeping every
    /// separator (whitespace/punctuation) verbatim so formatting + numbers survive.
    fn redact_api_keys(text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut word = String::new();
        for ch in text.chars() {
            if Self::is_word_char(ch) {
                word.push(ch);
            } else {
                Self::flush_word(&mut out, &mut word);
                out.push(ch);
            }
        }
        Self::flush_word(&mut out, &mut word);
        out
    }

    /// Append `word` to `out` — as the placeholder if it is API-key-shaped, else
    /// verbatim — then clear it.
    fn flush_word(out: &mut String, word: &mut String) {
        if word.is_empty() {
            return;
        }
        if crate::domain::looks_like_secret_token(word) {
            out.push_str(REDACTED);
        } else {
            out.push_str(word);
        }
        word.clear();
    }

    /// A token character: ASCII alphanumeric plus `-`/`_` (the shape of `sk-…`
    /// keys and base64/hex tokens). A `.` is a separator, so `12345.67` stays two
    /// pure-digit words (never key-shaped) and survives.
    fn is_word_char(ch: char) -> bool {
        ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'
    }

    /// Redact a whole prompt: each message's text content is scrubbed while its
    /// structure (roles, tool-call ids) is preserved.
    fn redact_messages(&self, messages: &[Message]) -> Vec<Message> {
        messages.iter().map(|m| self.redact_message(m)).collect()
    }

    /// Redact one message's text content field(s).
    fn redact_message(&self, message: &Message) -> Message {
        match message {
            Message::System { content } => Message::System {
                content: self.redact(content),
            },
            Message::User { content } => Message::User {
                content: self.redact(content),
            },
            Message::Assistant {
                content,
                tool_calls,
            } => Message::Assistant {
                content: content.as_ref().map(|c| self.redact(c)),
                // #81: scrub every string leaf of each tool call's `arguments`
                // (id + name are non-secret structural identifiers — kept verbatim).
                tool_calls: tool_calls
                    .iter()
                    .map(|tc| ToolCall {
                        id: tc.id.clone(),
                        name: tc.name.clone(),
                        arguments: self.redact_value(&tc.arguments),
                    })
                    .collect(),
            },
            Message::ToolResult {
                tool_call_id,
                content,
            } => Message::ToolResult {
                tool_call_id: tool_call_id.clone(),
                content: self.redact(content),
            },
        }
    }
}

/// The redacting + cost-logging [`LlmProvider`] decorator (README C7).
///
/// Generic over the inner provider `P`, the ledger repo `R`, and the [`Clock`]
/// `C` (never `dyn`), so 1.05 wraps the concrete `GlmProvider` at zero cost. It
/// IS an [`LlmProvider`], so it substitutes transparently for the raw provider.
///
/// No `#[derive(Debug)]`: `C: Clock` carries no `Debug` bound (mirrors
/// `SqliteLlmCallRepo`).
pub struct RedactingLoggingProvider<P, R, C> {
    inner: P,
    repo: R,
    clock: C,
    redactor: Redactor,
    prices: PriceTable,
    created_by: CreatedBy,
    key_source: Option<CredentialSource>,
}

impl<P, R, C> RedactingLoggingProvider<P, R, C> {
    /// Wrap `inner` with redaction + cost-logging into `repo`, timestamping each
    /// [`LlmCall`] from `clock`. `redactor` supplies the NFR-6 secret ruleset;
    /// `prices` the README-C5 cost table. `created_by` defaults to
    /// [`CreatedBy::Human`] this slice (the composer/coach supply it in 1.3.2+).
    #[must_use]
    pub fn new(inner: P, repo: R, clock: C, redactor: Redactor, prices: PriceTable) -> Self {
        Self {
            inner,
            repo,
            clock,
            redactor,
            prices,
            created_by: CreatedBy::Human,
            key_source: None,
        }
    }

    /// Override the provenance actor stamped on every persisted [`LlmCall`]
    /// (default [`CreatedBy::Human`], set by [`new`](Self::new)).
    ///
    /// An agent-driven composition root MUST call this: `llm_call` is
    /// UPDATE/DELETE-trigger-immutable, so a row written under the wrong actor
    /// can never be corrected in place. The composer wires
    /// [`CreatedBy::ComposerLlm`] so the ledger agrees with the
    /// `StrategyVersion.created_by` it provenance-links.
    #[must_use]
    pub fn with_created_by(mut self, created_by: CreatedBy) -> Self {
        self.created_by = created_by;
        self
    }

    /// Record WHICH credential source supplied the API key on every persisted
    /// [`LlmCall`] (r1.s1.w2 — the risk gate's audit-trail control).
    ///
    /// A LABEL, never the key: the decorator is handed
    /// [`ApiKey::source()`](crate::domain::ApiKey::source), a value type that cannot
    /// carry the credential, so this seam is incapable of leaking one.
    ///
    /// A builder rather than a `new` parameter so every existing call site keeps
    /// compiling and defaults to `None` (provenance not recorded) — the same shape
    /// as [`with_created_by`](Self::with_created_by). The live composition root
    /// (`src/cli/compose.rs`) sets it; a test double that does not care may omit it.
    #[must_use]
    pub fn with_key_source(mut self, key_source: Option<CredentialSource>) -> Self {
        self.key_source = key_source;
        self
    }
}

impl<P, R, C> LlmProvider for RedactingLoggingProvider<P, R, C>
where
    P: LlmProvider + Send + Sync,
    R: LlmCallRepository + Send + Sync,
    C: Clock + Send + Sync,
{
    async fn chat(
        &self,
        messages: Vec<Message>,
        tools: &[ToolDefinition],
        config: &LlmConfig,
    ) -> Result<LlmResponse, LlmError> {
        // Preflight the price table BEFORE the billed inner call: an unknown-model
        // config error is knowable up front (zero usage → Ok(0) if priceable, else
        // Err::Config), so surface it here rather than let a real, BILLED provider
        // call succeed and then escape the FR-24 ledger when the post-call cost
        // lookup fails (close-review Codex C2).
        self.prices.cost(&config.model, &TokenUsage::default())?;

        // OQ-A: the inner provider gets the REAL, un-redacted prompt AND tools; we
        // keep a copy of the messages only to scrub the persisted record.
        let prompt_copy = messages.clone();
        let response = self.inner.chat(messages, tools, config).await?;

        // Cost from usage times the price table — model already validated above.
        let cost = self.prices.cost(&config.model, &response.usage)?;
        let cost_currency = self.prices.currency().to_owned();

        // Redact the STORED copy (prompt + completion) — never the sent bytes.
        let prompt_messages = self.redactor.redact_messages(&prompt_copy);
        let completion = response.content.as_ref().map(|c| self.redactor.redact(c));

        let now_ms = self.clock.now_ms();
        let created_at = DateTime::from_timestamp_millis(now_ms)
            .ok_or_else(|| LlmError::Provider(format!("clock.now_ms() {now_ms} out of range")))?;

        let call = LlmCall {
            id: LlmCallId::new(Uuid::new_v4().to_string()),
            backend: config.backend,
            model: config.model.clone(),
            prompt_messages,
            completion,
            input_tokens: response.usage.input_tokens,
            output_tokens: response.usage.output_tokens,
            cost,
            cost_currency,
            created_at,
            created_by: self.created_by,
            key_source: self.key_source,
            // r1.s2.w2: this decorator serves the composer, whose ledger rows record
            // no prompt version (audit C2). `w3` supplies the resolved-prompt content
            // hash on the coach's own calls.
            prompt_version: None,
        };

        self.repo
            .save_call(&call)
            .await
            .map_err(|e| LlmError::Provider(format!("llm_call persist failed: {e}")))?;

        Ok(response)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{REDACTED, RedactingLoggingProvider, Redactor};
    use crate::adapters::clock::FakeClock;
    use crate::domain::strategy::CreatedBy;
    use crate::domain::{
        DataError, LlmBackend, LlmCall, LlmCallId, LlmCallRepository, LlmConfig, LlmError,
        LlmProvider, LlmResponse, Message, ModelPrice, PriceTable, TokenUsage, ToolCall,
        ToolDefinition,
    };
    use rust_decimal::Decimal;
    use std::collections::HashMap;
    use std::future::Future;
    use std::sync::{Arc, Mutex};

    /// A canned inner provider that RECORDS the exact messages it received, so a
    /// test can assert the decorator forwarded the REAL, un-redacted prompt
    /// (OQ-A). Returns a fixed [`LlmResponse`] with a known [`TokenUsage`].
    struct FakeProvider {
        received: Arc<Mutex<Vec<Vec<Message>>>>,
        response: LlmResponse,
    }

    impl LlmProvider for FakeProvider {
        fn chat(
            &self,
            messages: Vec<Message>,
            _tools: &[ToolDefinition],
            _config: &LlmConfig,
        ) -> impl Future<Output = Result<LlmResponse, LlmError>> {
            self.received.lock().expect("received lock").push(messages);
            std::future::ready(Ok(self.response.clone()))
        }
    }

    /// An in-memory ledger repo that CAPTURES every saved [`LlmCall`] verbatim, so
    /// a test can inspect the PERSISTED (redacted) copy. Mirrors 1.02's private
    /// `FakeLlmCallRepo` but exposes the captured rows.
    struct RecordingRepo {
        saved: Arc<Mutex<Vec<LlmCall>>>,
    }

    impl LlmCallRepository for RecordingRepo {
        fn save_call(&self, call: &LlmCall) -> impl Future<Output = Result<LlmCallId, DataError>> {
            self.saved.lock().expect("saved lock").push(call.clone());
            std::future::ready(Ok(call.id.clone()))
        }

        fn get_call(
            &self,
            id: &LlmCallId,
        ) -> impl Future<Output = Result<Option<LlmCall>, DataError>> {
            std::future::ready(Ok(self
                .saved
                .lock()
                .expect("saved lock")
                .iter()
                .find(|c| c.id == *id)
                .cloned()))
        }
    }

    fn config() -> LlmConfig {
        LlmConfig {
            backend: LlmBackend::Ollama,
            model: "gpt-oss:120b".to_owned(),
            temperature: 0.2,
            max_tokens: 256,
        }
    }

    /// A minimal test price table keyed on `gpt-oss:120b` (README C5), CNY-native.
    /// These are TEST values, not the production moat data (decision 4).
    fn prices() -> PriceTable {
        let mut models = HashMap::new();
        models.insert(
            "gpt-oss:120b".to_owned(),
            ModelPrice {
                input_per_mtok: Decimal::from(2),
                output_per_mtok: Decimal::from(8),
            },
        );
        PriceTable::from_config("CNY", models)
    }

    fn response(content: &str, input_tokens: u32, output_tokens: u32) -> LlmResponse {
        LlmResponse {
            content: Some(content.to_owned()),
            tool_calls: Vec::new(),
            usage: TokenUsage {
                input_tokens,
                output_tokens,
            },
        }
    }

    struct Driven {
        saved: Vec<LlmCall>,
        received: Vec<Vec<Message>>,
        returned: LlmResponse,
    }

    /// Wire the decorator over the fakes + a `FakeClock`, run one `chat`, and hand
    /// back what was persisted, what the inner provider received, and what the
    /// caller got.
    async fn drive(
        prompt: Vec<Message>,
        redactor: Redactor,
        canned: LlmResponse,
        now_ms: i64,
    ) -> Driven {
        let received = Arc::new(Mutex::new(Vec::new()));
        let saved = Arc::new(Mutex::new(Vec::new()));
        let provider = FakeProvider {
            received: Arc::clone(&received),
            response: canned,
        };
        let repo = RecordingRepo {
            saved: Arc::clone(&saved),
        };
        let decorator = RedactingLoggingProvider::new(
            provider,
            repo,
            FakeClock::at(now_ms),
            redactor,
            prices(),
        );
        let returned = decorator
            .chat(prompt, &[], &config())
            .await
            .expect("decorator chat succeeds");
        let saved = saved.lock().expect("saved lock").clone();
        let received = received.lock().expect("received lock").clone();
        Driven {
            saved,
            received,
            returned,
        }
    }

    fn user_text(message: &Message) -> &str {
        match message {
            Message::User { content } => content,
            other => panic!("expected a User message, got {other:?}"),
        }
    }

    const FAKE_KEY: &str = "sk-ABCD1234efGH5678ijKL9012mnOP3456";
    const TAGGED_SECRET: &str = "ACCT-9F3K-SECRET";

    #[tokio::test]
    async fn redacts_api_key_from_persisted_prompt() {
        let prompt = vec![
            Message::system("be terse"),
            Message::user(format!("use my key {FAKE_KEY} now")),
        ];
        let canned = response(&format!("stored {FAKE_KEY} ok"), 10, 4);
        let driven = drive(prompt, Redactor::default(), canned, 1_700_000_000_000).await;

        // (i) the PERSISTED prompt has the key replaced ...
        let call = &driven.saved[0];
        let stored = user_text(&call.prompt_messages[1]);
        assert!(
            !stored.contains(FAKE_KEY),
            "stored prompt still leaks the key: {stored}"
        );
        assert!(
            stored.contains(REDACTED),
            "stored prompt not redacted: {stored}"
        );
        // ... surrounding words preserved.
        assert!(stored.contains("use my key"));
        assert!(stored.contains("now"));
        // completion redacted too.
        let completion = call.completion.as_deref().expect("completion present");
        assert!(!completion.contains(FAKE_KEY));
        assert!(completion.contains(REDACTED));

        // (ii) the inner provider received the UN-redacted messages (OQ-A).
        let sent = user_text(&driven.received[0][1]);
        assert!(
            sent.contains(FAKE_KEY),
            "inner provider must receive the real key, got {sent}"
        );
        assert!(!sent.contains(REDACTED));
        // caller got the real (un-redacted) response back.
        assert_eq!(
            driven.returned.content.as_deref(),
            Some(format!("stored {FAKE_KEY} ok").as_str())
        );
    }

    #[tokio::test]
    async fn redacts_tagged_secret_field_but_preserves_numbers() {
        let body = format!("token {TAGGED_SECRET} balance 12345.67 at 3.5R over 1000 trades");
        let prompt = vec![Message::user(body.clone())];
        let redactor = Redactor::from_config(vec![TAGGED_SECRET.to_owned()]);
        let driven = drive(prompt, redactor, response("ok", 5, 1), 1_700_000_000_000).await;

        let stored = user_text(&driven.saved[0].prompt_messages[0]);
        // tagged secret stripped ...
        assert!(
            !stored.contains(TAGGED_SECRET),
            "tagged secret leaked: {stored}"
        );
        assert!(
            stored.contains(REDACTED),
            "tagged secret not redacted: {stored}"
        );
        // ... but plain numbers / balances / R-multiples are PRESERVED (we did NOT
        // build a strip-any-number redactor).
        assert!(
            stored.contains("12345.67"),
            "balance wrongly stripped: {stored}"
        );
        assert!(
            stored.contains("3.5"),
            "R-multiple wrongly stripped: {stored}"
        );
        assert!(
            stored.contains("1000"),
            "trade count wrongly stripped: {stored}"
        );

        // OQ-A: the inner provider received the UN-redacted prompt.
        let sent = user_text(&driven.received[0][0]);
        assert_eq!(sent, body);
        assert!(sent.contains(TAGGED_SECRET));
    }

    /// FIX C: the at-rest scrubber shares the compose-time prefix heuristic, so a
    /// `xox`-prefixed (Slack-style) token is redacted from the PERSISTED copy even
    /// with NO tagged secrets. Before the unification the at-rest test only matched
    /// `sk-`/≥32-char, so a `xoxb-…` token LEAKED at rest — this guards the
    /// "never weaker than compose-time" property.
    ///
    /// The fixture carries a REAL Slack token shape: PR #93 review tightened the
    /// heuristic so a bare `xox` prefix no longer matches on its own (`xoxo` in a
    /// trader's prose must survive), which a toy 18-char fixture would not exercise.
    #[tokio::test]
    async fn at_rest_scrubber_redacts_prefixed_secret_tokens() {
        // Assembled from fragments so the literal never appears whole in source:
        // GitHub push protection matches the real `xox<type>-…` pattern and would
        // (correctly) block a verbatim fixture.
        const SLACK_TOKEN: &str = concat!("xoxb", "-2401234567-1234567890123-AbCdEfGhIjKlMnOp");
        let prompt = vec![Message::user(format!("token {SLACK_TOKEN} here"))];
        // Redactor::default() has NO tagged secrets — only the structural heuristic
        // can catch this, so the assertion proves the heuristic (not a tagged value).
        let driven = drive(
            prompt,
            Redactor::default(),
            response("ok", 3, 1),
            1_700_000_000_000,
        )
        .await;

        let stored = user_text(&driven.saved[0].prompt_messages[0]);
        assert!(
            !stored.contains(SLACK_TOKEN),
            "at-rest scrubber leaked the prefixed token: {stored}"
        );
        assert!(
            stored.contains(REDACTED),
            "token not redacted at rest: {stored}"
        );
        // surrounding words survive.
        assert!(stored.contains("token") && stored.contains("here"));
    }

    #[tokio::test]
    async fn persists_llm_call_with_cost_and_tokens() {
        let prompt = vec![Message::user("size a long")];
        let now_ms = 1_700_000_123_000;
        let driven = drive(
            prompt,
            Redactor::default(),
            response("done", 1500, 500),
            now_ms,
        )
        .await;

        assert_eq!(driven.saved.len(), 1, "exactly one ledger row persisted");
        let call = &driven.saved[0];
        assert_eq!(call.backend, LlmBackend::Ollama);
        assert_eq!(call.model, "gpt-oss:120b");
        assert_eq!(call.input_tokens, 1500);
        assert_eq!(call.output_tokens, 500);
        // cost is the price-table figure, in native currency (no silent FX).
        let expected = prices()
            .cost(
                "gpt-oss:120b",
                &TokenUsage {
                    input_tokens: 1500,
                    output_tokens: 500,
                },
            )
            .expect("cost");
        assert_eq!(call.cost, expected);
        assert!(call.cost > Decimal::ZERO);
        assert_eq!(call.cost_currency, "CNY");
        // created_at came from the injected FakeClock (deterministic).
        assert_eq!(call.created_at.timestamp_millis(), now_ms);
        // this slice's default provenance.
        assert_eq!(call.created_by, CreatedBy::Human);
    }

    #[tokio::test]
    async fn cost_computed_from_usage_and_price_table() {
        let usage = TokenUsage {
            input_tokens: 1500,
            output_tokens: 500,
        };
        let driven = drive(
            vec![Message::user("hi")],
            Redactor::default(),
            LlmResponse {
                content: Some("ok".to_owned()),
                tool_calls: Vec::new(),
                usage,
            },
            1_700_000_000_000,
        )
        .await;

        // 1500/1e6 * 2  +  500/1e6 * 8  =  0.003 + 0.004  =  0.007 CNY.
        let call = &driven.saved[0];
        assert_eq!(
            call.cost,
            prices().cost("gpt-oss:120b", &usage).expect("cost")
        );
        assert_eq!(call.cost.normalize(), Decimal::new(7, 3).normalize());
        assert_eq!(call.cost_currency, "CNY");
    }

    #[tokio::test]
    async fn redacts_overlapping_tagged_secrets_longest_first() {
        // Two tagged secrets where the shorter is a substring of the longer. With
        // naive in-list-order replacement the short one scrubs part of the long one
        // and leaves its tail exposed; longest-first ordering must strip both fully
        // (close-review Codex C4).
        let short = "9F3K";
        let long = "ACCT-9F3K-TOKEN-XYZ";
        // Pass the SHORT one FIRST — `from_config` must reorder to longest-first.
        let redactor = Redactor::from_config(vec![short.to_owned(), long.to_owned()]);
        let prompt = vec![Message::user(format!("audit {long} please"))];
        let driven = drive(prompt, redactor, response("ok", 3, 1), 1_700_000_000_000).await;

        let stored = user_text(&driven.saved[0].prompt_messages[0]);
        assert!(stored.contains(REDACTED), "not redacted: {stored}");
        for leak in [long, short, "ACCT-", "TOKEN-XYZ"] {
            assert!(
                !stored.contains(leak),
                "persisted prompt still leaks `{leak}`: {stored}"
            );
        }
    }

    #[tokio::test]
    async fn unknown_model_errors_before_billing_and_ledger() {
        // A model absent from the price table must fail BEFORE the billed inner
        // call, so no provider request is spent and no ledger row is written — the
        // config error is knowable up front (close-review Codex C2).
        let received = Arc::new(Mutex::new(Vec::new()));
        let saved = Arc::new(Mutex::new(Vec::new()));
        let provider = FakeProvider {
            received: Arc::clone(&received),
            response: response("unreached", 1, 1),
        };
        let repo = RecordingRepo {
            saved: Arc::clone(&saved),
        };
        let decorator = RedactingLoggingProvider::new(
            provider,
            repo,
            FakeClock::at(1_700_000_000_000),
            Redactor::default(),
            prices(), // keyed on "gpt-oss:120b" only
        );
        let cfg = LlmConfig {
            backend: LlmBackend::Ollama,
            model: "unpriced-model".to_owned(),
            temperature: 0.2,
            max_tokens: 64,
        };

        let err = decorator
            .chat(vec![Message::user("hi")], &[], &cfg)
            .await
            .expect_err("an unpriced model must error");
        assert!(
            matches!(err, LlmError::Config(_)),
            "expected Config, got {err:?}"
        );
        assert!(
            received.lock().expect("received lock").is_empty(),
            "inner provider must NOT be billed for an unpriced model"
        );
        assert!(
            saved.lock().expect("saved lock").is_empty(),
            "no ledger row may be written when the model is unpriced"
        );
    }

    /// Extract the first tool call's `arguments` `Value` from an `Assistant` message.
    fn assistant_tool_call_args(message: &Message) -> &serde_json::Value {
        match message {
            Message::Assistant { tool_calls, .. } => &tool_calls[0].arguments,
            other => panic!("expected an Assistant message, got {other:?}"),
        }
    }

    /// AC-8 (#81 proof): the decorator scrubs `Assistant.tool_calls[i].arguments`
    /// in the PERSISTED copy. A secret nested inside a JSON string leaf of a tool
    /// call's arguments is redacted at rest (via the recursive `redact_value`),
    /// while the inner provider still received the arguments in the CLEAR (OQ-A).
    /// Non-secret structural params (numbers) survive verbatim.
    #[tokio::test]
    async fn redacts_tool_call_arguments() {
        // An Assistant turn in the PROMPT carrying a tool call whose arguments embed
        // BOTH a structural api-key-shaped token AND a caller-tagged secret, NESTED
        // inside a JSON object (proves the redactor recurses into string leaves).
        let arguments = serde_json::json!({
            "config": {
                "note": format!("use {FAKE_KEY} and {TAGGED_SECRET}"),
            },
            "size": 1000,
        });
        let prompt = vec![Message::Assistant {
            content: Some("calling a tool".to_owned()),
            tool_calls: vec![ToolCall {
                id: "call-1".to_owned(),
                name: "set_risk".to_owned(),
                arguments: arguments.clone(),
            }],
        }];
        let redactor = Redactor::from_config(vec![TAGGED_SECRET.to_owned()]);
        let driven = drive(prompt, redactor, response("ok", 5, 1), 1_700_000_000_000).await;

        // (i) the PERSISTED tool-call arguments have BOTH secrets scrubbed ...
        let stored_args = assistant_tool_call_args(&driven.saved[0].prompt_messages[0]);
        let stored_str = stored_args.to_string();
        assert!(
            !stored_str.contains(FAKE_KEY),
            "persisted args leak the api key: {stored_str}"
        );
        assert!(
            !stored_str.contains(TAGGED_SECRET),
            "persisted args leak the tagged secret: {stored_str}"
        );
        assert!(
            stored_str.contains(REDACTED),
            "persisted args not redacted: {stored_str}"
        );
        // ... while a non-secret numeric param survives (we did NOT strip numbers).
        assert_eq!(stored_args["size"], serde_json::json!(1000));

        // (ii) OQ-A: the inner provider received the UN-redacted arguments in the clear.
        let sent_args = assistant_tool_call_args(&driven.received[0][0]);
        assert_eq!(sent_args, &arguments);
    }
}
