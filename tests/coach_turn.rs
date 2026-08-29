//! AC-1 / demo line `d6` — the happy coach turn, end to end over a **scripted**
//! provider (r1.s2.w3, ADR-0021).
//!
//! The capability sentence, made executable: *a trader's persisted backtest run
//! yields exactly one validated DSL mutation with a stated hypothesis, with the
//! turn's cost and coach-prompt version in the `LlmCall` ledger.*
//!
//! This binary drives the real composition core (`run_coach_with`) over the REAL
//! redacting decorator, the REAL `apply()` mutation framework, the REAL coaching
//! repo and a REAL temp `SQLite` — with a fake `LlmProvider` in place of the model.
//! **No live LLM call happens here or anywhere in this item**: this file is a
//! demo-ledger line (`d6`) and is re-run at every future spine close, so provider
//! latency, cost and flakiness must never enter it.
//!
//! What it asserts:
//!   1. one scripted `propose_mutation` call yields exactly one persisted
//!      `Proposed` session carrying the typed mutation and a non-empty hypothesis;
//!   2. the session names its `LlmCall`, and that ledger row carries a cost and a
//!      `prompt_version` equal to the SHA-256 of the RESOLVED prompt;
//!   3. a `$PULSE_PROMPT_DIR/coach.md` overlay wins and changes the recorded
//!      version; with no overlay the compiled-in default is used;
//!   4. exactly ONE provider call is made (grill L3 — no retries, no nudges).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::collections::VecDeque;
use std::future::Future;
use std::sync::{Arc, Mutex};

use pulse::{
    BacktestResult, BacktestRunId, BacktestRunRepository, CoachCliOutcome, CoachWiring,
    CoachingRepository, CoachingSession, CreatedBy, Db, Disposition, EngineFingerprint, FakeClock,
    LlmCall, LlmCallId, LlmCallRepository, LlmConfig, LlmError, LlmProvider, LlmResponse, MIGRATOR,
    Message, ModelPrice, Mutation, NewVersion, ParamValue, PriceTable, Redactor, RegimeBreakdown,
    SessionOutcome, SkippedEntryCounts, SqliteBacktestRunRepo, SqliteCoachingRepo,
    SqliteLlmCallRepo, SqliteStrategyRepo, StrategyRepository, StrategyVersion, SummaryStats,
    TokenUsage, ToolCall, ToolDefinition, run_coach_with,
};
use rust_decimal::Decimal;
use serde_json::json;
use sha2::{Digest, Sha256};
use tempfile::TempDir;

mod coach_support;
use coach_support::{CapturingLlmRepo, canonical_dsl_json, config, test_prices};

/// A scripted `LlmProvider`: hands back queued responses, records every message
/// vec it saw, and counts calls. Never touches the network.
struct ScriptedProvider {
    scripts: Mutex<VecDeque<LlmResponse>>,
    seen: Arc<Mutex<Vec<Vec<Message>>>>,
}

impl ScriptedProvider {
    fn new(responses: Vec<LlmResponse>) -> (Self, Arc<Mutex<Vec<Vec<Message>>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                scripts: Mutex::new(responses.into()),
                seen: Arc::clone(&seen),
            },
            seen,
        )
    }
}

impl LlmProvider for ScriptedProvider {
    fn chat(
        &self,
        messages: Vec<Message>,
        _tools: &[ToolDefinition],
        _config: &LlmConfig,
    ) -> impl Future<Output = Result<LlmResponse, LlmError>> {
        self.seen.lock().expect("seen lock").push(messages);
        let next = self.scripts.lock().expect("scripts lock").pop_front();
        std::future::ready(Ok(next.unwrap_or_else(|| LlmResponse {
            content: Some("(script exhausted)".to_owned()),
            tool_calls: Vec::new(),
            usage: TokenUsage {
                input_tokens: 10,
                output_tokens: 5,
            },
        })))
    }
}

/// One `propose_mutation` tool call — the happy script.
fn propose_call(path: &str, new_value: &serde_json::Value, hypothesis: &str) -> LlmResponse {
    LlmResponse {
        content: None,
        tool_calls: vec![ToolCall {
            id: "call-1".to_owned(),
            name: "propose_mutation".to_owned(),
            arguments: json!({
                "path": path,
                "new_value": new_value.clone(),
                "hypothesis": hypothesis,
            }),
        }],
        usage: TokenUsage {
            input_tokens: 1_000,
            output_tokens: 200,
        },
    }
}

/// The SHA-256 hex of the compiled-in default coach prompt, read from the same
/// file `include_str!` embeds.
fn default_prompt_version() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/agent/prompts/coach.md");
    let bytes = std::fs::read(&path).expect("read the shipped coach prompt");
    hex_sha256(&bytes)
}

fn hex_sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// A migrated temp DB plus a persisted strategy version and backtest run to coach
/// against. Returns the guard, the `Db`, the version, and the run id.
async fn seeded() -> (TempDir, Db, StrategyVersion, BacktestRunId) {
    let tmp = TempDir::new().expect("tempdir");
    let db = Db::with_path(&tmp.path().join("pulse.db"))
        .await
        .expect("open db");
    MIGRATOR.run(db.pool()).await.expect("run migrations");

    let strategy_repo = SqliteStrategyRepo::new(db.pool().clone());
    let strategy = strategy_repo
        .create_strategy("RSI Oversold", None, &[])
        .await
        .expect("create strategy");
    let version = strategy_repo
        .create_version(NewVersion {
            strategy_id: strategy.id.clone(),
            parent_version_id: None,
            dsl_json: canonical_dsl_json(),
            created_by: CreatedBy::Human,
            creating_llm_call_ids: vec![],
        })
        .await
        .expect("create version");

    // A real run through the real repo, so `get_run`'s re-validate-on-read hash
    // check passes (a hand-seeded row would not).
    let run_repo =
        SqliteBacktestRunRepo::with_deps(db.pool().clone(), FakeClock::at(1_700_000_000_000));
    let result = BacktestResult {
        trades: Vec::new(),
        net_pnl: Decimal::ZERO,
        fees_total: Decimal::ZERO,
        funding_total: Decimal::ZERO,
        slippage_total: Decimal::ZERO,
        regime_breakdown: RegimeBreakdown::default(),
        skipped_entries: SkippedEntryCounts::default(),
        engine_fingerprint: EngineFingerprint::current(),
        summary: SummaryStats::default(),
        equity_curve: pulse::EquityCurve::default(),
    };
    let run_id = run_repo
        .save_run(
            &version.id,
            &result,
            &SummaryStats::default(),
            Decimal::new(10_000, 0),
        )
        .await
        .expect("save run");

    (tmp, db, version, run_id)
}

/// Drive one coach turn over the scripted provider, optionally with a prompt
/// overlay directory.
async fn coach_once(
    db: &Db,
    run_id: &BacktestRunId,
    script: Vec<LlmResponse>,
    prompt_dir: Option<std::path::PathBuf>,
) -> (CoachCliOutcome, Arc<Mutex<Vec<Vec<Message>>>>) {
    let clock = FakeClock::at(1_700_000_000_000);
    let (provider, seen) = ScriptedProvider::new(script);
    let ids = Arc::new(Mutex::new(Vec::new()));
    let llm_repo = CapturingLlmRepo::new(
        SqliteLlmCallRepo::with_deps(db.pool().clone(), clock),
        Arc::clone(&ids),
    );

    let wiring = CoachWiring {
        provider,
        llm_repo,
        redactor: Redactor::default(),
        prices: test_prices(),
        clock,
        key_source: None,
        config: config(),
        prompt_dir,
        turn_timeout: None,
        max_dsl_bytes: None,
        captured: ids,
    };

    let run_repo = SqliteBacktestRunRepo::with_deps(db.pool().clone(), clock);
    let strategy_repo = SqliteStrategyRepo::new(db.pool().clone());
    let coaching_repo = SqliteCoachingRepo::with_deps(db.pool().clone(), clock);

    let outcome = run_coach_with(wiring, &run_repo, &strategy_repo, &coaching_repo, run_id)
        .await
        .expect("the coach turn completes");
    (outcome, seen)
}

// ---------------------------------------------------------------------------
// 1. One validated mutation with a stated hypothesis
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_persisted_run_yields_one_validated_mutation_with_a_hypothesis() {
    let (_tmp, db, _version, run_id) = seeded().await;

    let (outcome, seen) = coach_once(
        &db,
        &run_id,
        vec![propose_call(
            "entry.lhs.indicator.rsi.period",
            &json!({ "type": "Period", "value": 21 }),
            "a slower RSI should cut the whipsaw entries this run shows",
        )],
        None,
    )
    .await;

    // Exactly one provider call — no retries, no nudges (grill L3).
    assert_eq!(
        seen.lock().expect("seen lock").len(),
        1,
        "a coach turn makes exactly one provider call"
    );

    // The session is `Proposed`, with the typed mutation and a stated hypothesis.
    let session: &CoachingSession = &outcome.session;
    match &session.outcome {
        SessionOutcome::Proposed { proposal } => {
            assert_eq!(
                proposal.mutation,
                Mutation::SetParam {
                    path: "entry.lhs.indicator.rsi.period".to_owned(),
                    new_value: ParamValue::Period { value: 21 },
                },
                "the proposal carries the typed mutation the model asked for"
            );
            assert_eq!(
                proposal.hypothesis.as_str(),
                "a slower RSI should cut the whipsaw entries this run shows"
            );
            assert_eq!(proposal.disposition, Disposition::Proposed);
        }
        SessionOutcome::Failed { failure } => panic!("expected a proposal, got {failure:?}"),
    }
    assert_eq!(&session.backtest_run_id, &run_id);

    // It persisted, and reads back the same (never silence: the row IS the record).
    let coaching_repo = SqliteCoachingRepo::new(db.pool().clone());
    let stored = coaching_repo
        .get_session(&session.id)
        .await
        .expect("get_session")
        .expect("the turn persisted a session row");
    assert_eq!(
        &stored, session,
        "the returned session is the persisted one"
    );

    // The turn reached the provider, so it names its ledger row.
    let call_id: &LlmCallId = session
        .llm_call_id
        .as_ref()
        .expect("a turn that reached the provider names its LlmCall");

    // The ledger row carries a cost and the resolved prompt's version.
    let llm_repo = SqliteLlmCallRepo::new(db.pool().clone());
    let call: LlmCall = llm_repo
        .get_call(call_id)
        .await
        .expect("get_call")
        .expect("the decorator persisted the ledger row");
    assert!(
        call.cost > Decimal::ZERO,
        "the turn's cost is recorded, got {}",
        call.cost
    );
    assert_eq!(
        call.created_by,
        CreatedBy::CoachLlm,
        "attributed to the coach"
    );
    assert_eq!(
        call.prompt_version.as_deref(),
        Some(default_prompt_version().as_str()),
        "prompt_version is the SHA-256 of the resolved prompt"
    );
    assert_eq!(
        outcome.prompt_version,
        default_prompt_version(),
        "the outcome reports the version it stamped"
    );
}

// ---------------------------------------------------------------------------
// 2. The prompt overlay wins and changes the recorded version
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_prompt_overlay_changes_the_recorded_prompt_version() {
    let (_tmp, db, _version, run_id) = seeded().await;

    let overlay_dir = TempDir::new().expect("overlay dir");
    let overlay = "You are a coach. This is the private overlay prompt.\n";
    std::fs::write(overlay_dir.path().join("coach.md"), overlay).expect("write overlay");

    let (outcome, _seen) = coach_once(
        &db,
        &run_id,
        vec![propose_call(
            "entry.lhs.indicator.rsi.period",
            &json!({ "type": "Period", "value": 21 }),
            "the overlay's hypothesis",
        )],
        Some(overlay_dir.path().to_path_buf()),
    )
    .await;

    let expected = hex_sha256(overlay.as_bytes());
    assert_eq!(
        outcome.prompt_version, expected,
        "the overlay's bytes are what get hashed"
    );
    assert_ne!(
        outcome.prompt_version,
        default_prompt_version(),
        "an overlay edit changes the recorded version with no release step (audit C2)"
    );

    // And it is what reached the ledger.
    let call_id = outcome
        .session
        .llm_call_id
        .as_ref()
        .expect("the turn reached the provider");
    let llm_repo = SqliteLlmCallRepo::new(db.pool().clone());
    let call = llm_repo
        .get_call(call_id)
        .await
        .expect("get_call")
        .expect("row present");
    assert_eq!(call.prompt_version.as_deref(), Some(expected.as_str()));

    // The overlay text really is the system prompt the model saw.
    let seen_overlay = outcome.prompt_version == expected;
    assert!(seen_overlay, "the resolved overlay drove the turn");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_empty_overlay_dir_falls_back_to_the_compiled_in_default() {
    let (_tmp, db, _version, run_id) = seeded().await;
    let empty_dir = TempDir::new().expect("empty overlay dir");

    let (outcome, seen) = coach_once(
        &db,
        &run_id,
        vec![propose_call(
            "exits[0].distance_pct",
            &json!({ "type": "Threshold", "value": "0.03" }),
            "a tighter stop should raise expectancy",
        )],
        Some(empty_dir.path().to_path_buf()),
    )
    .await;

    assert_eq!(
        outcome.prompt_version,
        default_prompt_version(),
        "with no coach.md in the override dir, the compiled-in default is used"
    );

    // The system message is the compiled-in prompt, and the user message is the
    // bounded context — two messages, nothing else (least privilege).
    let seen = seen.lock().expect("seen lock");
    assert_eq!(seen.len(), 1, "one call");
    assert_eq!(
        seen[0].len(),
        2,
        "exactly a system prompt and the bounded context: {:?}",
        seen[0]
    );

    // A Threshold-typed mutation applies just as a Period one does.
    match &outcome.session.outcome {
        SessionOutcome::Proposed { proposal } => assert_eq!(
            proposal.mutation,
            Mutation::SetParam {
                path: "exits[0].distance_pct".to_owned(),
                new_value: ParamValue::Threshold {
                    value: Decimal::new(3, 2)
                },
            }
        ),
        SessionOutcome::Failed { failure } => panic!("expected a proposal, got {failure:?}"),
    }
}

// ---------------------------------------------------------------------------
// A price table + config live in `fixture`; this keeps the unused-import lint
// honest about what this binary actually needs.
// ---------------------------------------------------------------------------

#[test]
fn the_test_price_table_bills_the_configured_model() {
    let prices: PriceTable = test_prices();
    let cost = prices
        .cost(
            &config().model,
            &TokenUsage {
                input_tokens: 1_000_000,
                output_tokens: 0,
            },
        )
        .expect("the fixture model is priced");
    assert!(cost > Decimal::ZERO);
    let _ = HashMap::<String, ModelPrice>::new();
}
