//! Offline end-to-end test for VS-1.3.1 work-1.05 — the `pulse llm-check`
//! composition root (FR-23 / FR-24 / NFR-6).
//!
//! Drives the injectable core [`run_llm_check_with`] with a **fake** inner
//! provider (no network, MASTER-SPEC §9.4) + a literal test key + a tempfile-`Db`
//! [`SqliteLlmCallRepo`] over a migrated scratch db (no Keychain), then asserts an
//! [`LlmCall`] is persisted with the prompt **redacted** and tokens/cost/currency
//! **populated**. This is the slice's `auto` demo criterion: "a GLM call
//! round-trips through `PulseHive` and logs an `LlmCall` … redaction strips the
//! secret", proven over a fake provider (the live `PulseHive` call is the `user:`
//! demo).
//!
//! **Single shared clock (1.04 deferral).** ONE [`FakeClock`] is injected into
//! BOTH the repo AND (via the core) the redacting decorator, so `created_at` is
//! single-sourced — the read-back row's `created_at` equals the injected instant.
//!
//! Offline (in-process `MIGRATOR` + committed `.sqlx/`), `TempDir`-isolated.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

use pulse::{
    Db, FakeClock, LlmBackend, LlmCallRepository, LlmCheckOutcome, LlmConfig, LlmError,
    LlmProvider, LlmResponse, MIGRATOR, Message, ModelPrice, PriceTable, Redactor,
    SqliteLlmCallRepo, TokenUsage, ToolDefinition, run_llm_check_with,
};
use rust_decimal::Decimal;
use tempfile::TempDir;

/// An API-key-shaped literal that the default [`Redactor`] must strip from the
/// PERSISTED copy (a `sk-` prefixed high-entropy tail). NOT a real key.
const FAKE_KEY: &str = "sk-TESTKEY1234abcd5678efgh9012ijkl3456";

/// The redaction placeholder the decorator substitutes (mirror
/// `redacting_logging.rs::REDACTED`, which is private — pinned here as the stable
/// contract the persisted copy carries).
const REDACTED: &str = "«REDACTED»";

/// A canned, offline inner provider: it RECORDS the exact messages it received
/// (so the test can prove the inner provider got the REAL, un-redacted prompt —
/// OQ-A) and returns a fixed [`LlmResponse`] with a known [`TokenUsage`]. No
/// network, no keychain.
struct FakeProvider {
    received: Arc<Mutex<Vec<Vec<Message>>>>,
    reply: String,
    usage: TokenUsage,
}

impl LlmProvider for FakeProvider {
    fn chat(
        &self,
        messages: Vec<Message>,
        _tools: &[ToolDefinition],
        _config: &LlmConfig,
    ) -> impl Future<Output = Result<LlmResponse, LlmError>> {
        self.received.lock().expect("received lock").push(messages);
        std::future::ready(Ok(LlmResponse {
            content: Some(self.reply.clone()),
            tool_calls: Vec::new(),
            usage: self.usage,
        }))
    }
}

/// A literal TEST price table keyed on the `DEMO_MODEL` the core drives
/// (`glm-5.3-flash`, README C5), CNY-native — so the computed `cost` is
/// populated + non-zero. These are TEST values, NOT production moat data (the real
/// subscription rate is a nominal estimate, config-tunable).
fn test_prices() -> PriceTable {
    let mut models = HashMap::new();
    models.insert(
        "glm-5.3-flash".to_owned(),
        ModelPrice {
            input_per_mtok: Decimal::from(2),
            output_per_mtok: Decimal::from(8),
        },
    );
    PriceTable::from_config("CNY", models)
}

/// A fresh `TempDir` + a migrated `pulse.db` [`Db`] over it (offline, in-process
/// `MIGRATOR`; the `TempDir` guard keeps the scratch db alive for the test body).
async fn migrated_db() -> (TempDir, Db) {
    let tmp = TempDir::new().expect("tempdir");
    let db = Db::with_path(&tmp.path().join("pulse.db"))
        .await
        .expect("open db");
    MIGRATOR.run(db.pool()).await.expect("run migrations");
    (tmp, db)
}

/// Pull the text out of a [`Message::User`], panicking on any other variant.
fn user_text(message: &Message) -> String {
    match message {
        Message::User { content } => content.clone(),
        other => panic!("expected a User message, got {other:?}"),
    }
}

/// AC-3: the offline e2e — a fake-provider round-trip persists an `LlmCall` whose
/// prompt is REDACTED (no key leak) with tokens/cost/currency populated, while the
/// inner provider still received the REAL prompt (OQ-A). NO network, NO keychain.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn logs_redacted_llm_call_over_fake_provider() {
    let (_tmp, db) = migrated_db().await;
    let now_ms = 1_700_000_000_000;

    // The SINGLE SHARED CLOCK (1.04 deferral): ONE FakeClock injected into BOTH the
    // repo AND (via the core) the redacting decorator, so `created_at` is
    // single-sourced. FakeClock is Copy, so the same instant reaches both seams.
    let clock = FakeClock::at(now_ms);
    let repo = SqliteLlmCallRepo::with_deps(db.pool().clone(), clock);

    let received = Arc::new(Mutex::new(Vec::new()));
    let provider = FakeProvider {
        received: Arc::clone(&received),
        reply: format!("I stored your key {FAKE_KEY} — done."),
        usage: TokenUsage {
            input_tokens: 1500,
            output_tokens: 500,
        },
    };

    // A prompt carrying a fake API-key-shaped token that redaction MUST strip from
    // the persisted copy (but NOT from the bytes the inner provider receives).
    let prompt = vec![
        Message::system("be terse"),
        Message::user(format!("use my api key {FAKE_KEY} to trade")),
    ];

    let outcome: LlmCheckOutcome = run_llm_check_with(
        provider,
        repo,
        Redactor::default(),
        test_prices(),
        clock,
        prompt,
    )
    .await
    .expect("llm round-trip succeeds");

    let call = &outcome.call;

    // (i) the PERSISTED prompt is REDACTED — the key is gone, the placeholder is
    // present, and the surrounding words survive.
    let stored_user = user_text(&call.prompt_messages[1]);
    assert!(
        !stored_user.contains(FAKE_KEY),
        "persisted prompt still leaks the key: {stored_user}"
    );
    assert!(
        stored_user.contains(REDACTED),
        "persisted prompt not redacted: {stored_user}"
    );
    assert!(stored_user.contains("use my api key"));
    assert!(stored_user.contains("to trade"));
    // completion redacted too (the reply echoed the key back).
    let completion = call.completion.as_deref().expect("completion present");
    assert!(
        !completion.contains(FAKE_KEY),
        "persisted completion leaks the key: {completion}"
    );
    assert!(completion.contains(REDACTED));

    // (ii) tokens + cost + currency populated (cost NON-ZERO from the test table:
    // 1500/1e6*2 + 500/1e6*8 = 0.003 + 0.004 = 0.007 CNY).
    assert_eq!(call.input_tokens, 1500);
    assert_eq!(call.output_tokens, 500);
    assert!(
        call.cost > Decimal::ZERO,
        "cost must be populated non-zero, was {}",
        call.cost
    );
    assert_eq!(call.cost.normalize(), Decimal::new(7, 3).normalize());
    assert_eq!(call.cost_currency, "CNY");
    assert_eq!(call.backend, LlmBackend::Ollama);
    assert_eq!(call.model, "glm-5.3-flash");

    // (iii) OQ-A: the inner provider received the REAL, un-redacted prompt. Scope
    // the guard so it drops before the later read-back await (no lock held across
    // `.await`).
    {
        let sent = received.lock().expect("received lock");
        let sent_user = user_text(&sent[0][1]);
        assert!(
            sent_user.contains(FAKE_KEY),
            "inner provider must receive the real key, got {sent_user}"
        );
        assert!(!sent_user.contains(REDACTED));
    }
    // the caller got the real (un-redacted) reply back.
    assert_eq!(
        outcome.response.content.as_deref(),
        Some(format!("I stored your key {FAKE_KEY} — done.").as_str())
    );

    // (iv) TRUE persistence: read the row back through the repo's `get_call` path
    // over the same pool. The single shared clock ⇒ the stored `created_at` equals
    // the injected instant (created_at single-sourced).
    let reader = SqliteLlmCallRepo::with_deps(db.pool().clone(), clock);
    let stored = reader
        .get_call(&call.id)
        .await
        .expect("get_call")
        .expect("the persisted row is fetchable");
    assert_eq!(stored.id, call.id);
    assert_eq!(stored.input_tokens, 1500);
    assert_eq!(stored.output_tokens, 500);
    assert!(stored.cost > Decimal::ZERO);
    assert_eq!(stored.cost_currency, "CNY");
    let stored_user = user_text(&stored.prompt_messages[1]);
    assert!(
        !stored_user.contains(FAKE_KEY),
        "the DB row leaks the key: {stored_user}"
    );
    assert_eq!(
        stored.created_at.timestamp_millis(),
        now_ms,
        "single shared clock ⇒ created_at is single-sourced from the injected instant"
    );
}
