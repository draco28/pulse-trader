//! `pulse llm-check` — the VS-1.3.1 composition root + demo verb (FR-23 / FR-24 /
//! NFR-6, README the-full-composition).
//!
//! This is the ONE place the slice's concrete LLM types are assembled
//! (monomorphized), keeping every layer generic underneath. The live arm
//! ([`run_llm_check`]) wires:
//!
//! ```text
//! glm_api_key()  →  OpenAiCompatProvider::new(key)
//!                →  RedactingLoggingProvider::new(inner, repo, clock, redactor, prices)
//!                   where repo = SqliteLlmCallRepo over the opened Db
//!                →  .chat()  →  a redacted, cost-logged LlmCall row
//! ```
//!
//! and prints backend / model / tokens / cost+currency + the stored `LlmCall`
//! id, then the model's reply and the persisted (redacted) prompt so a human can
//! confirm the secret was stripped at rest.
//!
//! **Injectable core (audit C2, mirror `run_fetch_data`).**
//! [`run_llm_check_with`] takes the provider + repo + redactor + prices + clock by
//! value, so the offline auto-test (`tests/llm_roundtrip_cli.rs`) drives the SAME
//! composition with a FAKE provider + a tempfile-`Db` repo — never a live
//! `GlmProvider`, never the network/Keychain (MASTER-SPEC §9.4).
//!
//! **Single shared clock (1.04 deferral).** The live arm creates ONE
//! [`SystemClock`] and injects the SAME clock into BOTH the
//! [`RedactingLoggingProvider`] AND the [`SqliteLlmCallRepo`] — the repo's
//! `save_call` overrides `created_at` with its own clock, so a single shared
//! clock keeps the persisted timestamp single-sourced.
//!
//! **Nominal price (Z.AI coding plan).** The GLM coding plan is a FLAT-RATE
//! subscription, so the per-token figures the ledger records are a NOMINAL
//! estimate (config-tunable later), not a real per-Mtok rate. The values are DATA
//! fed through [`PriceTable::from_config`] (the moat seam), never a hardcoded
//! public-Rust price literal in the domain.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::adapters::clock::SystemClock;
use crate::adapters::db::{Db, SqliteLlmCallRepo};
use crate::adapters::llm::openai_compat::OpenAiCompatProvider;
use crate::adapters::llm::redacting_logging::{RedactingLoggingProvider, Redactor};
use crate::adapters::secrets::glm_api_key;
use crate::domain::{
    Clock, DataError, LlmBackend, LlmCall, LlmCallId, LlmCallRepository, LlmConfig, LlmProvider,
    LlmResponse, Message, ModelPrice, PriceTable,
};

use rust_decimal::Decimal;
use std::collections::HashMap;

/// The demo model id — `gpt-oss:120b` via Ollama Cloud (mirrors the
/// `openai_compat.rs` `OLLAMA_MODEL_ID` const + the price-table key so `cost`
/// resolves).
const DEMO_MODEL: &str = "gpt-oss:120b";

/// A conservative sampling temperature for the demo round-trip (wire-level `f32`,
/// never a determinism input — MASTER-SPEC §9.4 / the `LlmConfig` note).
const DEMO_TEMPERATURE: f32 = 0.2;

/// The response token cap for the demo round-trip. GLM 5.2 is a **reasoning**
/// model whose thinking tokens count against this cap BEFORE the final answer, so
/// a tight cap yields empty `content` (the live VS-1.3.1 close demo saw an empty
/// reply at 256 and a real one at ~343). Keep generous headroom past the reasoning.
const DEMO_MAX_TOKENS: u32 = 4096;

/// The fixed demo prompt used when the operator gives no prompt argument.
const DEMO_PROMPT: &str = "In one concise sentence, what is a liquidation in crypto futures?";

/// The native billing currency of the nominal GLM price table (Zhipu/Z.AI bills
/// RMB/CNY — audit ch3; no silent FX baked in).
const NOMINAL_CURRENCY: &str = "CNY";

/// `pulse llm-check [PROMPT] [--db <path>]` — run a GLM 5.2 chat round-trip through
/// the redacting + cost-logging composition and print the persisted `LlmCall`.
///
/// The verb name derives from the [`Command::LlmCheck`](super::Command) variant
/// (clap kebab-cases it to `llm-check`), so the top-level `--help` lists `llm`.
#[derive(Debug, clap::Args)]
pub struct LlmArgs {
    /// The prompt to send (a fixed demo prompt is used when omitted).
    pub prompt: Option<String>,
    /// `pulse.db` path override (defaults to the platform Application Support db);
    /// `global = true` so it parses in any position (mirror `RunsArgs.db`).
    #[arg(long, global = true)]
    pub db: Option<PathBuf>,
}

/// The outcome of one demo round-trip: the persisted (redacted) [`LlmCall`] ledger
/// record and the un-redacted [`LlmResponse`] the caller received. The auto-test
/// asserts against `call`; the live arm prints from both.
pub struct LlmCheckOutcome {
    /// The persisted ledger record — prompt + completion REDACTED, tokens + cost +
    /// currency populated, `created_at` from the shared clock.
    pub call: LlmCall,
    /// The un-redacted response the model returned (OQ-A: the caller sees the real
    /// reply; only the stored copy is scrubbed).
    pub response: LlmResponse,
}

/// The nominal GLM 5.2 price table (README C5, the moat-in-data seam).
///
/// The Z.AI coding plan is a FLAT-RATE subscription, so these per-Mtok figures are
/// a NOMINAL estimate the ledger records for observability, NOT a real per-token
/// bill — they are config-tunable and deliberately not authoritative. Fed as DATA
/// through [`PriceTable::from_config`], never a hardcoded domain literal.
fn nominal_price_table() -> PriceTable {
    let mut models = HashMap::new();
    models.insert(
        DEMO_MODEL.to_owned(),
        ModelPrice {
            // Nominal CNY/Mtok estimates for the flat-rate coding plan.
            input_per_mtok: Decimal::new(2, 0),
            output_per_mtok: Decimal::new(8, 0),
        },
    );
    PriceTable::from_config(NOMINAL_CURRENCY, models)
}

/// The demo chat config (backend = Ollama, model = [`DEMO_MODEL`], nominal knobs).
fn demo_config() -> LlmConfig {
    LlmConfig {
        backend: LlmBackend::Ollama,
        model: DEMO_MODEL.to_owned(),
        temperature: DEMO_TEMPERATURE,
        max_tokens: DEMO_MAX_TOKENS,
    }
}

/// Build the demo prompt: a fixed system framing plus the operator's prompt (or
/// the fixed [`DEMO_PROMPT`] when none was given).
fn build_prompt(args: &LlmArgs) -> Vec<Message> {
    let user = args
        .prompt
        .clone()
        .unwrap_or_else(|| DEMO_PROMPT.to_owned());
    vec![
        Message::system(
            "You are PulseTrader's assistant. Answer concisely. This is not financial advice.",
        ),
        Message::user(user),
    ]
}

/// A capture side-channel over an inner [`LlmCallRepository`]: it forwards
/// `save_call` to the real repo (the actual persistence) and records a COPY of the
/// saved row, so the composition root can surface the persisted `LlmCall` (its id,
/// redacted prompt, tokens, cost) after the write.
///
/// The port has no "last saved" read and the [`RedactingLoggingProvider`] mints the
/// row id internally, so this thin wrapper is how the id is recovered generically —
/// for BOTH the live arm and the auto-test — without modifying 1.04's decorator.
struct CapturingRepo<R> {
    inner: R,
    captured: Arc<Mutex<Option<LlmCall>>>,
}

impl<R: LlmCallRepository + Send + Sync> LlmCallRepository for CapturingRepo<R> {
    async fn save_call(&self, call: &LlmCall) -> Result<LlmCallId, DataError> {
        // Persist through the real repo FIRST (its clock overrides `created_at`);
        // only capture the row once the write actually succeeded.
        let id = self.inner.save_call(call).await?;
        if let Ok(mut slot) = self.captured.lock() {
            *slot = Some(call.clone());
        }
        Ok(id)
    }

    async fn get_call(&self, id: &LlmCallId) -> Result<Option<LlmCall>, DataError> {
        self.inner.get_call(id).await
    }
}

/// The injectable, fixture-doubleable core (audit C2, mirror `run_fetch_data`):
/// assemble the redacting + cost-logging decorator over the injected `provider` /
/// `repo` / `redactor` / `prices` / `clock`, run ONE `chat()` over `prompt`, and
/// return the persisted (redacted) [`LlmCall`] plus the un-redacted response.
///
/// The auto-test drives THIS with a FAKE provider + a tempfile-`Db` repo — never a
/// live [`GlmProvider`], never the network/Keychain (MASTER-SPEC §9.4). The same
/// `clock` value should be injected here AND into the `SqliteLlmCallRepo` so
/// `created_at` is single-sourced (the 1.04 deferral).
///
/// # Errors
///
/// Returns an [`anyhow::Error`] if the provider round-trip fails, the model has no
/// price-table entry (fail-closed cost), the ledger persist fails, or (defensively)
/// the saved row was not captured.
pub async fn run_llm_check_with<P, R, C>(
    provider: P,
    repo: R,
    redactor: Redactor,
    prices: PriceTable,
    clock: C,
    prompt: Vec<Message>,
) -> anyhow::Result<LlmCheckOutcome>
where
    P: LlmProvider + Send + Sync,
    R: LlmCallRepository + Send + Sync,
    C: Clock + Send + Sync,
{
    let captured: Arc<Mutex<Option<LlmCall>>> = Arc::new(Mutex::new(None));
    let capturing = CapturingRepo {
        inner: repo,
        captured: Arc::clone(&captured),
    };

    // The composition root: wrap the (already-selected) provider in the redacting +
    // cost-logging decorator over the capturing repo, sharing the single `clock`.
    let decorator = RedactingLoggingProvider::new(provider, capturing, clock, redactor, prices);
    let config = demo_config();

    // No-tool back-compat: the `llm-check` demo advertises no tools (composer tools
    // are 2.04); an empty slice reproduces the VS-1.3.1 behavior exactly.
    let response = decorator
        .chat(prompt, &[], &config)
        .await
        .map_err(|e| anyhow::anyhow!("llm chat round-trip failed: {e}"))?;

    // Recover the persisted row (with its adapter-minted id) from the capture slot.
    let call = {
        let slot = captured
            .lock()
            .map_err(|_| anyhow::anyhow!("internal: llm_call capture lock poisoned"))?;
        slot.clone()
    }
    .ok_or_else(|| anyhow::anyhow!("internal: no llm_call was persisted by the round-trip"))?;

    Ok(LlmCheckOutcome { call, response })
}

/// The LIVE arm (composition root): source the GLM key from the Keychain, build the
/// `GlmProvider` → `RedactingLoggingProvider` → `SqliteLlmCallRepo` composition over
/// the opened `db`, run the round-trip via [`run_llm_check_with`], and print the
/// result. This is the ONLY place the concrete GLM types are assembled.
///
/// `db` is `Some` for this verb (the dispatcher opens a migrated `pulse.db` — the
/// ledger write needs it); it is `Option<&Db>` to mirror the sibling CLI arms.
///
/// # Errors
///
/// Returns an [`anyhow::Error`] on an absent db, a missing/unreadable Keychain key
/// (pointing at `pulse setup-keys`), a provider/transport failure, or a ledger
/// persist failure — every path a clear message + non-zero exit, never a panic.
pub async fn run_llm_check(db: Option<&Db>, args: &LlmArgs) -> anyhow::Result<()> {
    let db = db.ok_or_else(|| anyhow::anyhow!("internal: llm-check requires an open db"))?;

    // Source the key from the macOS Keychain (READ only — the seed path is
    // VS-1.3.4's `pulse setup-keys`). A missing entry is a clear error, not a panic.
    let key = glm_api_key().map_err(|e| anyhow::anyhow!("read GLM API key: {e}"))?;

    // Tag the live key value as a secret so an accidental echo of it in the prompt
    // is scrubbed from the STORED copy too (structural sk-shaped stripping is always
    // on). Clone before moving the key into the provider ctor.
    let redactor = Redactor::from_config(vec![key.clone()]);
    let provider = OpenAiCompatProvider::new(key);

    // SINGLE SHARED CLOCK (1.04 deferral): ONE SystemClock injected into BOTH the
    // repo AND the decorator (via the core), so `created_at` is single-sourced.
    let clock = SystemClock;
    let repo = SqliteLlmCallRepo::with_deps(db.pool().clone(), clock);

    let prices = nominal_price_table();
    let prompt = build_prompt(args);

    let outcome = run_llm_check_with(provider, repo, redactor, prices, clock, prompt).await?;
    print_outcome(&outcome);
    Ok(())
}

/// Print the round-trip result: the ledger header (backend / model / tokens /
/// cost+currency / stored id), the model's un-redacted reply, and the persisted
/// (redacted) prompt so a human can confirm the secret was stripped at rest.
fn print_outcome(outcome: &LlmCheckOutcome) {
    let call = &outcome.call;
    println!(
        "llm-check\tbackend={}\tmodel={}\tinput_tokens={}\toutput_tokens={}\tcost={} {}\tllm_call_id={}",
        backend_label(call.backend),
        call.model,
        call.input_tokens,
        call.output_tokens,
        call.cost.normalize(),
        call.cost_currency,
        call.id.as_str(),
    );
    if let Some(content) = &outcome.response.content {
        println!("response\t{content}");
    }
    println!("persisted_prompt (redacted — confirm no secret leaks at rest):");
    for message in &call.prompt_messages {
        println!("  {}", render_message(message));
    }
}

/// The bare backend tag for display (e.g. `ollama`).
fn backend_label(backend: LlmBackend) -> &'static str {
    match backend {
        LlmBackend::Ollama => "ollama",
    }
}

/// Render one persisted message as `role: content` for the redaction readout.
fn render_message(message: &Message) -> String {
    match message {
        Message::System { content } => format!("system: {content}"),
        Message::User { content } => format!("user: {content}"),
        Message::Assistant { content, .. } => {
            format!("assistant: {}", content.as_deref().unwrap_or(""))
        }
        Message::ToolResult {
            tool_call_id,
            content,
        } => format!("tool[{tool_call_id}]: {content}"),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{
        DEMO_MODEL, LlmArgs, build_prompt, demo_config, nominal_price_table, render_message,
    };
    use crate::cli::{Cli, Command};
    use crate::domain::{LlmBackend, Message, TokenUsage};
    use clap::Parser;
    use rust_decimal::Decimal;

    #[test]
    fn parses_llm_check_with_positional_prompt() {
        let cli = Cli::try_parse_from(["pulse", "llm-check", "hello there"]).expect("parse");
        let Command::LlmCheck(args) = cli.command else {
            panic!("expected an llm-check command");
        };
        assert_eq!(args.prompt.as_deref(), Some("hello there"));
    }

    #[test]
    fn parses_llm_check_db_override_globally() {
        let cli =
            Cli::try_parse_from(["pulse", "llm-check", "hi", "--db", "/tmp/x.db"]).expect("parse");
        let Command::LlmCheck(args) = cli.command else {
            panic!("expected an llm-check command");
        };
        assert_eq!(
            args.db.as_deref().and_then(std::path::Path::to_str),
            Some("/tmp/x.db")
        );
    }

    #[test]
    fn build_prompt_uses_demo_prompt_when_absent() {
        let args = LlmArgs {
            prompt: None,
            db: None,
        };
        let prompt = build_prompt(&args);
        assert_eq!(prompt.len(), 2);
        match &prompt[0] {
            Message::System { .. } => {}
            other => panic!("expected a system framing, got {other:?}"),
        }
        // The user turn carries the fixed demo prompt (non-empty).
        match &prompt[1] {
            Message::User { content } => assert!(!content.is_empty()),
            other => panic!("expected a user turn, got {other:?}"),
        }
    }

    #[test]
    fn demo_config_targets_ollama_model_matching_the_price_table() {
        let config = demo_config();
        assert_eq!(config.backend, LlmBackend::Ollama);
        assert_eq!(config.model, DEMO_MODEL);
        // The nominal table has a price for the demo model, so cost resolves + is
        // non-zero for a non-trivial usage (fail-closed otherwise).
        let cost = nominal_price_table()
            .cost(
                DEMO_MODEL,
                &TokenUsage {
                    input_tokens: 1000,
                    output_tokens: 1000,
                },
            )
            .expect("nominal table prices the demo model");
        assert!(
            cost > Decimal::ZERO,
            "nominal cost must be non-zero: {cost}"
        );
    }

    #[test]
    fn render_message_labels_each_role() {
        assert_eq!(render_message(&Message::system("s")), "system: s");
        assert_eq!(render_message(&Message::user("u")), "user: u");
        assert_eq!(render_message(&Message::assistant("a")), "assistant: a");
    }
}
