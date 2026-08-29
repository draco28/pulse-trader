//! AC-2 / demo line `d7` — every deviant coach turn ends as a typed recorded
//! failure, never silence (r1.s2.w3, grill L3 / ADR-0021 decision 6).
//!
//! The half of the capability sentence that is easy to skip and expensive to get
//! wrong: *zero calls, several calls, malformed arguments, an inapplicable
//! mutation, a timeout, context overflow — each ends as a recorded failed session.*
//! A coach that quietly returns nothing when the model misbehaves is the failure
//! mode this spine exists to remove, so each of the six is driven here through the
//! REAL turn against a scripted provider and asserted to leave a row behind.
//!
//! **No live LLM call happens here.** This binary is a demo-ledger line (`d7`)
//! re-run at every future spine close.
//!
//! Also asserted, once per case: **at most one provider call per turn** — no
//! retries and no nudges, unlike the composer.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pulse::{
    BacktestResult, BacktestRunId, BacktestRunRepository, CoachCliOutcome, CoachFailure,
    CoachWiring, CoachingRepository, CreatedBy, Db, EngineFingerprint, FakeClock, LlmConfig,
    LlmError, LlmProvider, LlmResponse, MIGRATOR, Message, MutationError, NewVersion, Redactor,
    RegimeBreakdown, SessionOutcome, SkippedEntryCounts, SqliteBacktestRunRepo, SqliteCoachingRepo,
    SqliteLlmCallRepo, SqliteStrategyRepo, StrategyRepository, SummaryStats, TokenUsage, ToolCall,
    ToolDefinition, run_coach_with,
};
use rust_decimal::Decimal;
use serde_json::json;
use tempfile::TempDir;

mod coach_support;
use coach_support::{CapturingLlmRepo, canonical_dsl_json, config, test_prices};

/// A scripted provider that counts its calls and can stall past a turn timeout.
struct ScriptedProvider {
    scripts: Mutex<VecDeque<LlmResponse>>,
    calls: Arc<AtomicUsize>,
    delay: Option<Duration>,
}

impl ScriptedProvider {
    fn new(responses: Vec<LlmResponse>) -> (Self, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        (
            Self {
                scripts: Mutex::new(responses.into()),
                calls: Arc::clone(&calls),
                delay: None,
            },
            calls,
        )
    }

    fn stalling(delay: Duration) -> (Self, Arc<AtomicUsize>) {
        let (mut provider, calls) = Self::new(vec![]);
        provider.delay = Some(delay);
        (provider, calls)
    }
}

impl LlmProvider for ScriptedProvider {
    async fn chat(
        &self,
        _messages: Vec<Message>,
        _tools: &[ToolDefinition],
        _config: &LlmConfig,
    ) -> Result<LlmResponse, LlmError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(delay) = self.delay {
            tokio::time::sleep(delay).await;
        }
        Ok(self
            .scripts
            .lock()
            .expect("scripts lock")
            .pop_front()
            .unwrap_or_else(|| text_only("(script exhausted)")))
    }
}

fn usage() -> TokenUsage {
    TokenUsage {
        input_tokens: 500,
        output_tokens: 50,
    }
}

/// A response with no tool call at all.
fn text_only(text: &str) -> LlmResponse {
    LlmResponse {
        content: Some(text.to_owned()),
        tool_calls: Vec::new(),
        usage: usage(),
    }
}

fn call(id: &str, name: &str, arguments: serde_json::Value) -> ToolCall {
    ToolCall {
        id: id.to_owned(),
        name: name.to_owned(),
        arguments,
    }
}

fn with_calls(tool_calls: Vec<ToolCall>) -> LlmResponse {
    LlmResponse {
        content: None,
        tool_calls,
        usage: usage(),
    }
}

fn good_args(path: &str) -> serde_json::Value {
    json!({
        "path": path,
        "new_value": { "type": "Period", "value": 21 },
        "hypothesis": "a slower RSI should cut whipsaw entries",
    })
}

/// A migrated temp DB with a persisted version + run to coach against.
async fn seeded() -> (TempDir, Db, BacktestRunId) {
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

    let clock = FakeClock::at(1_700_000_000_000);
    let run_repo = SqliteBacktestRunRepo::with_deps(db.pool().clone(), clock);
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
    (tmp, db, run_id)
}

/// Run one turn over `provider`, with optional timeout / DSL-budget overrides.
async fn turn_with(
    db: &Db,
    run_id: &BacktestRunId,
    provider: ScriptedProvider,
    turn_timeout: Option<Duration>,
    max_dsl_bytes: Option<usize>,
) -> CoachCliOutcome {
    let clock = FakeClock::at(1_700_000_000_000);
    let ids = Arc::new(Mutex::new(Vec::new()));
    let wiring = CoachWiring {
        provider,
        llm_repo: CapturingLlmRepo::new(
            SqliteLlmCallRepo::with_deps(db.pool().clone(), clock),
            Arc::clone(&ids),
        ),
        redactor: Redactor::default(),
        prices: test_prices(),
        clock,
        key_source: None,
        config: config(),
        prompt_dir: None,
        turn_timeout,
        max_dsl_bytes,
        captured: ids,
    };
    let run_repo = SqliteBacktestRunRepo::with_deps(db.pool().clone(), clock);
    let strategy_repo = SqliteStrategyRepo::new(db.pool().clone());
    let coaching_repo = SqliteCoachingRepo::with_deps(db.pool().clone(), clock);
    run_coach_with(wiring, &run_repo, &strategy_repo, &coaching_repo, run_id)
        .await
        .expect("a deviant turn is still a completed turn")
}

/// The recorded failure of a turn — panicking if it produced a proposal instead.
fn failure_of(outcome: &CoachCliOutcome) -> &CoachFailure {
    match &outcome.outcome_ref() {
        SessionOutcome::Failed { failure } => failure,
        SessionOutcome::Proposed { .. } => panic!("expected a recorded failure, got a proposal"),
    }
}

/// Assert the session really is on disk — never silence means a ROW, not a return
/// value.
async fn assert_persisted(db: &Db, outcome: &CoachCliOutcome) {
    let repo = SqliteCoachingRepo::new(db.pool().clone());
    let stored = repo
        .get_session(&outcome.session.id)
        .await
        .expect("get_session")
        .expect("the failed turn left a session row behind");
    assert_eq!(&stored, &outcome.session, "the row is the returned session");
}

trait OutcomeRef {
    fn outcome_ref(&self) -> &SessionOutcome;
}
impl OutcomeRef for CoachCliOutcome {
    fn outcome_ref(&self) -> &SessionOutcome {
        &self.session.outcome
    }
}

// ---------------------------------------------------------------------------
// The six typed failures
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_turn_with_no_tool_call_is_recorded_as_zero_calls() {
    let (_tmp, db, run_id) = seeded().await;
    let (provider, calls) =
        ScriptedProvider::new(vec![text_only("I think you should try RSI 21.")]);

    let outcome = turn_with(&db, &run_id, provider, None, None).await;

    assert!(matches!(failure_of(&outcome), CoachFailure::ZeroCalls));
    assert_eq!(calls.load(Ordering::SeqCst), 1, "exactly one provider call");
    assert!(
        outcome.session.llm_call_id.is_some(),
        "the turn reached the provider, so it names its ledger row"
    );
    assert_persisted(&db, &outcome).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_turn_with_several_tool_calls_is_recorded_as_several_calls() {
    let (_tmp, db, run_id) = seeded().await;
    let (provider, calls) = ScriptedProvider::new(vec![with_calls(vec![
        call(
            "c1",
            "propose_mutation",
            good_args("entry.lhs.indicator.rsi.period"),
        ),
        call("c2", "propose_mutation", good_args("exits[0].distance_pct")),
    ])]);

    let outcome = turn_with(&db, &run_id, provider, None, None).await;

    match failure_of(&outcome) {
        CoachFailure::SeveralCalls { count } => assert_eq!(*count, 2),
        other => panic!("expected SeveralCalls, got {other:?}"),
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1, "still ONE provider call");
    assert_persisted(&db, &outcome).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unparseable_arguments_are_recorded_as_malformed() {
    let (_tmp, db, run_id) = seeded().await;
    let (provider, calls) = ScriptedProvider::new(vec![with_calls(vec![call(
        "c1",
        "propose_mutation",
        // `new_value` is not the tagged shape the tool declares.
        json!({ "path": "entry.lhs.indicator.rsi.period", "new_value": 21, "hypothesis": "faster" }),
    )])]);

    let outcome = turn_with(&db, &run_id, provider, None, None).await;

    match failure_of(&outcome) {
        CoachFailure::MalformedArguments { detail } => assert!(
            detail.contains("propose_mutation"),
            "the recorded reason names what failed: {detail}"
        ),
        other => panic!("expected MalformedArguments, got {other:?}"),
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_persisted(&db, &outcome).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_empty_hypothesis_is_recorded_as_malformed() {
    let (_tmp, db, run_id) = seeded().await;
    let (provider, _calls) = ScriptedProvider::new(vec![with_calls(vec![call(
        "c1",
        "propose_mutation",
        json!({
            "path": "entry.lhs.indicator.rsi.period",
            "new_value": { "type": "Period", "value": 21 },
            "hypothesis": "   ",
        }),
    )])]);

    let outcome = turn_with(&db, &run_id, provider, None, None).await;

    match failure_of(&outcome) {
        CoachFailure::MalformedArguments { detail } => assert!(
            detail.contains("hypothesis"),
            "a mutation without a stated hypothesis is not a proposal: {detail}"
        ),
        other => panic!("expected MalformedArguments, got {other:?}"),
    }
    assert_persisted(&db, &outcome).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_mutation_that_does_not_apply_is_recorded_inapplicable() {
    let (_tmp, db, run_id) = seeded().await;
    let (provider, _calls) = ScriptedProvider::new(vec![with_calls(vec![call(
        "c1",
        "propose_mutation",
        json!({
            // A path that addresses nothing in this strategy — the real `apply()`
            // decides, not a lookalike check here.
            "path": "entry.lhs.indicator.ema.period",
            "new_value": { "type": "Period", "value": 21 },
            "hypothesis": "an EMA would be steadier",
        }),
    )])]);

    let outcome = turn_with(&db, &run_id, provider, None, None).await;

    match failure_of(&outcome) {
        CoachFailure::InapplicableMutation { mutation, error } => {
            assert!(
                matches!(error, MutationError::UnknownPath { .. }),
                "the w1 MutationError is carried verbatim: {error:?}"
            );
            assert!(
                format!("{mutation:?}").contains("ema.period"),
                "the rejected mutation is recorded too, so the turn is reconstructable"
            );
        }
        other => panic!("expected InapplicableMutation, got {other:?}"),
    }
    assert_persisted(&db, &outcome).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_provider_that_outlives_the_guard_is_recorded_as_a_timeout() {
    let (_tmp, db, run_id) = seeded().await;
    let (provider, calls) = ScriptedProvider::stalling(Duration::from_secs(30));

    // A test-shortened guard: the mechanism is the composer's `tokio::time::timeout`
    // (audit C5); only the value moves.
    let outcome = turn_with(
        &db,
        &run_id,
        provider,
        Some(Duration::from_millis(20)),
        None,
    )
    .await;

    match failure_of(&outcome) {
        CoachFailure::ProviderTimeout { elapsed_ms } => assert_eq!(*elapsed_ms, 20),
        other => panic!("expected ProviderTimeout, got {other:?}"),
    }
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "the timed-out call is not retried"
    );
    assert_persisted(&db, &outcome).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_oversized_dsl_is_recorded_as_context_overflow_before_any_call() {
    let (_tmp, db, run_id) = seeded().await;
    let (provider, calls) = ScriptedProvider::new(vec![with_calls(vec![call(
        "c1",
        "propose_mutation",
        good_args("entry.lhs.indicator.rsi.period"),
    )])]);

    // A budget the canonical strategy cannot fit under.
    let outcome = turn_with(&db, &run_id, provider, None, Some(10)).await;

    match failure_of(&outcome) {
        CoachFailure::ContextOverflow { detail } => assert!(
            detail.contains("budget"),
            "the recorded reason states the measured overflow: {detail}"
        ),
        other => panic!("expected ContextOverflow, got {other:?}"),
    }

    // The point of a PRE-call check: it costs nothing.
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "context overflow is detected before the provider is called"
    );
    assert!(
        outcome.session.llm_call_id.is_none(),
        "a pre-call failure records no ledger row (audit C3)"
    );
    assert_persisted(&db, &outcome).await;
}

// ---------------------------------------------------------------------------
// The taxonomy, as a set
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unknown_tool_is_recorded_rather_than_ignored() {
    let (_tmp, db, run_id) = seeded().await;
    let (provider, _calls) = ScriptedProvider::new(vec![with_calls(vec![call(
        "c1",
        "finalize_strategy",
        json!({}),
    )])]);

    let outcome = turn_with(&db, &run_id, provider, None, None).await;

    match failure_of(&outcome) {
        CoachFailure::MalformedArguments { detail } => assert!(
            detail.contains("finalize_strategy"),
            "the recorded reason names the tool that was called: {detail}"
        ),
        other => panic!("expected MalformedArguments, got {other:?}"),
    }
    assert_persisted(&db, &outcome).await;
}
