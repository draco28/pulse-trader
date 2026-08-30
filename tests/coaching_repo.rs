//! AC-3 — `SqliteCoachingRepo`: every coach turn survives a round trip, typed
//! (r1.s2.w2, ADR-0021 / audit C3).
//!
//! The session row is the audit trail, so "it persisted" is not enough: what comes
//! back has to be the same turn. This binary drives the real adapter against a real
//! migrated database and asserts:
//!
//!   1. a proposal session round-trips with its **typed `Mutation`** intact — not a
//!      string, not a lossy projection (`r1.s4`'s modify path edits it);
//!   2. every `CoachFailure` variant round-trips, including
//!      `InapplicableMutation`, which carries a w1 `MutationError` verbatim;
//!   3. `llm_call_id` persists NULL and non-NULL, and NULL means exactly "no
//!      provider call was made" (audit C3);
//!   4. recording a disposition persists `child_version_id` only for `Accepted`;
//!   5. at most one proposal per session, end to end — the accept idempotency key.
//!
//! Offline (`SQLX_OFFLINE=true` + the committed `.sqlx/` + the in-process
//! `MIGRATOR`), `TempDir`-isolated, and deterministic through a `FakeClock`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use chrono::{DateTime, SecondsFormat};
use pulse::{
    BacktestRunId, CoachFailure, CoachingRepository, CoachingSession, CoachingSessionId,
    Comparator, Condition, Db, Direction, Disposition, ExitRule, FakeClock, Hypothesis,
    IndicatorSpec, LlmCallId, MIGRATOR, Mutation, MutationError, ParamValue, Proposal, RiskParams,
    SchemaVersion, SessionOutcome, SqliteCoachingRepo, StrategyDsl, SweepableValue, ValueSource,
    VersionId, apply,
};
use rust_decimal::Decimal;
use sqlx::SqlitePool;
use tempfile::TempDir;

const NOW_MS: i64 = 1_756_425_600_000; // 2026-08-29T00:00:00Z

/// The RFC3339-millis rendering of [`NOW_MS`] — what the adapter's injected clock
/// writes, so a sample built with it round-trips EQUAL.
fn now_rfc3339() -> String {
    DateTime::from_timestamp_millis(NOW_MS)
        .expect("clock in range")
        .to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// A `(repo, pool, tempdir)` triple over a fresh migrated tempfile DB with the FK
/// parents seeded and a deterministic clock.
async fn repo() -> (SqliteCoachingRepo<FakeClock>, SqlitePool, TempDir) {
    let tmp = TempDir::new().expect("tempdir");
    let db = Db::with_path(&tmp.path().join("pulse.db"))
        .await
        .expect("open db");
    MIGRATOR.run(db.pool()).await.expect("run migrations");
    let pool = db.pool().clone();
    seed_parents(&pool).await;
    (
        SqliteCoachingRepo::with_deps(pool.clone(), FakeClock::at(NOW_MS)),
        pool,
        tmp,
    )
}

/// The FK parents a coaching session needs: a strategy, two versions (parent +
/// a child for an accept), two runs, and an `llm_call`.
async fn seed_parents(pool: &SqlitePool) {
    sqlx::query(
        "INSERT INTO strategy (id, name, tags, archived, created_at) \
         VALUES ('strat-1', 'RSI Oversold', '[]', 0, '2026-08-29T00:00:00.000Z')",
    )
    .execute(pool)
    .await
    .expect("seed strategy");

    // A SECOND strategy, so a cross-strategy child is expressible.
    sqlx::query(
        "INSERT INTO strategy (id, name, tags, archived, created_at) \
         VALUES ('strat-2', 'Another Strategy', '[]', 0, '2026-08-29T00:00:00.000Z')",
    )
    .execute(pool)
    .await
    .expect("seed second strategy");

    // The version TREE the accept path has to reason about (PR #128, finding G2).
    // `ver-1` is what the seeded runs — and therefore the sessions — were produced
    // against; `ver-2` and `ver-3` are its real children, the shape an accept mints.
    // The rest are the wrong shapes, each expressible only because the schema's FK
    // says "some version", not "this one's child".
    for (id, strategy, parent, hash, by) in [
        ("ver-1", "strat-1", None, "hash-1", "human"),
        ("ver-2", "strat-1", Some("ver-1"), "hash-2", "coach_llm"),
        ("ver-3", "strat-1", Some("ver-1"), "hash-3", "coach_llm"),
        ("ver-root", "strat-1", None, "hash-root", "human"),
        (
            "ver-sibling",
            "strat-1",
            Some("ver-root"),
            "hash-sib",
            "coach_llm",
        ),
        (
            "ver-foreign",
            "strat-2",
            Some("ver-1"),
            "hash-foreign",
            "coach_llm",
        ),
    ] {
        sqlx::query(
            "INSERT INTO strategy_version \
             (id, strategy_id, parent_version_id, dsl_schema_version, dsl, dsl_original, \
              version_hash, created_by, creating_llm_call_ids, created_at) \
             VALUES (?1, ?2, ?3, '1.0.0', '{}', '{}', ?4, ?5, '[]', \
                     '2026-08-29T00:00:00.000Z')",
        )
        .bind(id)
        .bind(strategy)
        .bind(parent)
        .bind(hash)
        .bind(by)
        .execute(pool)
        .await
        .expect("seed strategy_version");
    }

    for run in ["run-1", "run-2"] {
        sqlx::query(
            "INSERT INTO backtest_run \
             (id, strategy_version_id, schema_version, created_at, engine_fingerprint, \
              engine_target, result_content_hash, starting_equity, net_pnl, fees_total, \
              funding_total, slippage_total) \
             VALUES (?1, 'ver-1', '1', '2026-08-29T00:00:00.000Z', 'fp-1', 'test-target', \
                     'rch-1', '10000', '0', '0', '0', '0')",
        )
        .bind(run)
        .execute(pool)
        .await
        .expect("seed backtest_run");
    }

    sqlx::query(
        "INSERT INTO llm_call \
         (id, backend, model, prompt_messages, completion, input_tokens, output_tokens, cost, \
          cost_currency, created_at, created_by, schema_version) \
         VALUES ('call-1', 'ollama', 'glm-5.3-flash', '[]', NULL, 1, 1, '0', 'CNY', \
                 '2026-08-29T00:00:00.000Z', 'coach_llm', 1)",
    )
    .execute(pool)
    .await
    .expect("seed llm_call");
}

fn rsi_oversold_strategy() -> StrategyDsl {
    StrategyDsl {
        schema_version: SchemaVersion::CURRENT,
        name: "RSI Oversold".to_owned(),
        direction: Direction::Long,
        entry: Condition::Compare {
            lhs: ValueSource::Indicator {
                spec: IndicatorSpec::Rsi {
                    period: SweepableValue::Fixed(14),
                },
            },
            op: Comparator::Lt,
            rhs: ValueSource::Constant {
                value: Decimal::new(30, 0),
            },
        },
        filters: vec![],
        exits: vec![
            ExitRule::StopLoss {
                distance_pct: SweepableValue::Fixed(Decimal::new(5, 2)),
            },
            ExitRule::TakeProfit {
                target_r: SweepableValue::Fixed(Decimal::new(2, 0)),
            },
        ],
        risk: RiskParams {
            risk_per_trade_pct: SweepableValue::Fixed(Decimal::new(1, 2)),
            max_leverage: SweepableValue::Fixed(Decimal::new(3, 0)),
        },
    }
}

fn set_period(path: &str, value: u32) -> Mutation {
    Mutation::SetParam {
        path: path.to_owned(),
        new_value: ParamValue::Period { value },
    }
}

/// A REAL `MutationError`, driven out of `apply()` rather than hand-built.
fn a_real_mutation_error() -> MutationError {
    apply(
        &rsi_oversold_strategy(),
        &set_period("entry.lhs.indicator.rsi.period", 0),
    )
    .expect_err("an RSI period of 0 fails validation")
}

fn session(id: &str, run: &str, outcome: SessionOutcome, call: Option<&str>) -> CoachingSession {
    CoachingSession {
        id: CoachingSessionId::new(id),
        backtest_run_id: BacktestRunId::new(run),
        strategy_version_id: VersionId::new("ver-1"),
        created_at: now_rfc3339(),
        llm_call_id: call.map(LlmCallId::new),
        outcome,
    }
}

fn a_proposal() -> Proposal {
    Proposal {
        mutation: set_period("entry.lhs.indicator.rsi.period", 21),
        hypothesis: Hypothesis::new("a slower RSI should cut the whipsaw entries")
            .expect("non-empty"),
        disposition: Disposition::Proposed,
    }
}

/// One instance of EVERY [`CoachFailure`] variant.
///
/// The trailing match is exhaustive with no `_` arm, so an eighth failure kind
/// stops this file compiling until it is added to the list above it. A bare
/// `vec![...]` cannot hold that line — it silently covered six of seven until
/// `TransportFailure` was caught in review.
fn every_failure_variant() -> Vec<CoachFailure> {
    let all = vec![
        CoachFailure::ZeroCalls,
        CoachFailure::SeveralCalls {
            count: 3,
            propose_mutation_count: 2,
        },
        CoachFailure::MalformedArguments {
            detail: "`path` was not a string".to_owned(),
        },
        CoachFailure::InapplicableMutation {
            mutation: set_period("entry.lhs.indicator.rsi.period", 0),
            error: a_real_mutation_error(),
        },
        CoachFailure::ProviderTimeout { elapsed_ms: 30_000 },
        CoachFailure::ContextOverflow {
            detail: "42kB of DSL against a 32kB window".to_owned(),
        },
        CoachFailure::TransportFailure {
            detail: "HTTP 503 from upstream".to_owned(),
        },
    ];

    let mut tags: Vec<&'static str> = all
        .iter()
        .map(|f| match f {
            CoachFailure::ZeroCalls => "zero_calls",
            CoachFailure::SeveralCalls { .. } => "several_calls",
            CoachFailure::MalformedArguments { .. } => "malformed_arguments",
            CoachFailure::InapplicableMutation { .. } => "inapplicable_mutation",
            CoachFailure::ProviderTimeout { .. } => "provider_timeout",
            CoachFailure::ContextOverflow { .. } => "context_overflow",
            CoachFailure::TransportFailure { .. } => "transport_failure",
        })
        .collect();
    tags.sort_unstable();
    tags.dedup();
    assert_eq!(tags.len(), all.len(), "one instance per variant: {tags:?}");
    all
}

// ---------------------------------------------------------------------------
// 1. A proposal session round-trips, typed
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_proposal_session_round_trips_with_its_typed_mutation() {
    let (repo, _pool, _tmp) = repo().await;
    let saved = session(
        "sess-1",
        "run-1",
        SessionOutcome::Proposed {
            proposal: a_proposal(),
        },
        Some("call-1"),
    );

    let id = repo.save_session(&saved).await.expect("save_session");
    assert_eq!(id, CoachingSessionId::new("sess-1"));

    let got = repo
        .get_session(&id)
        .await
        .expect("get_session")
        .expect("row present");
    assert_eq!(got, saved, "the turn round-trips verbatim");

    // The typed Mutation specifically — r1.s4's modify path edits it in place.
    match &got.outcome {
        SessionOutcome::Proposed { proposal } => {
            assert_eq!(
                proposal.mutation,
                set_period("entry.lhs.indicator.rsi.period", 21),
                "the typed Mutation survives storage"
            );
            assert_eq!(
                proposal.hypothesis.as_str(),
                "a slower RSI should cut the whipsaw entries"
            );
            assert_eq!(proposal.disposition, Disposition::Proposed);
        }
        SessionOutcome::Failed { failure } => panic!("expected a proposal, got {failure:?}"),
    }

    // An absent id is Ok(None), not an error.
    assert!(
        repo.get_session(&CoachingSessionId::new("nope"))
            .await
            .expect("get_session absent")
            .is_none()
    );
}

// ---------------------------------------------------------------------------
// 2. Every failure variant round-trips
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_failure_variant_round_trips() {
    let (repo, _pool, _tmp) = repo().await;

    let failures = every_failure_variant();
    assert_eq!(failures.len(), 7, "every failure kind must round-trip");

    for (i, failure) in failures.into_iter().enumerate() {
        let id = format!("sess-fail-{i}");
        // Which failures leave a ledger row, by EXHAUSTIVE match (no `_` arm): a
        // pre-call refusal never reached the provider, and a transport fault
        // produced no usable exchange to bill, so both record `llm_call_id` NULL
        // (audit C3). An eighth variant has to answer this question before this
        // file compiles.
        let call = match &failure {
            CoachFailure::ContextOverflow { .. } | CoachFailure::TransportFailure { .. } => None,
            CoachFailure::ZeroCalls
            | CoachFailure::SeveralCalls { .. }
            | CoachFailure::MalformedArguments { .. }
            | CoachFailure::InapplicableMutation { .. }
            | CoachFailure::ProviderTimeout { .. } => Some("call-1"),
        };
        let saved = session(
            &id,
            "run-1",
            SessionOutcome::Failed {
                failure: failure.clone(),
            },
            call,
        );

        repo.save_session(&saved).await.expect("save a failed turn");
        let got = repo
            .get_session(&CoachingSessionId::new(&id))
            .await
            .expect("get_session")
            .expect("row present");

        assert_eq!(got, saved, "{failure:?} must round-trip verbatim");
        match got.outcome {
            SessionOutcome::Failed { failure: back } => assert_eq!(back, failure),
            SessionOutcome::Proposed { .. } => panic!("a failed turn must read back failed"),
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_inapplicable_mutation_keeps_its_typed_error_through_the_database() {
    let (repo, _pool, _tmp) = repo().await;
    let error = a_real_mutation_error();
    let saved = session(
        "sess-1",
        "run-1",
        SessionOutcome::Failed {
            failure: CoachFailure::InapplicableMutation {
                mutation: set_period("entry.lhs.indicator.rsi.period", 0),
                error: error.clone(),
            },
        },
        Some("call-1"),
    );

    repo.save_session(&saved).await.expect("save");
    let got = repo
        .get_session(&CoachingSessionId::new("sess-1"))
        .await
        .expect("get")
        .expect("present");

    match got.outcome {
        SessionOutcome::Failed {
            failure: CoachFailure::InapplicableMutation { error: back, .. },
        } => {
            assert_eq!(
                back, error,
                "the w1 MutationError survives the database verbatim"
            );
            assert!(matches!(back, MutationError::ValidationFailed { .. }));
        }
        other => panic!("expected an InapplicableMutation failure, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// 3. llm_call_id — NULL means "no provider call was made" (audit C3)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn llm_call_id_persists_null_and_non_null() {
    let (repo, pool, _tmp) = repo().await;

    repo.save_session(&session(
        "sess-called",
        "run-1",
        SessionOutcome::Proposed {
            proposal: a_proposal(),
        },
        Some("call-1"),
    ))
    .await
    .expect("save a turn that reached the provider");

    repo.save_session(&session(
        "sess-precall",
        "run-1",
        SessionOutcome::Failed {
            failure: CoachFailure::ContextOverflow {
                detail: "the DSL does not fit".to_owned(),
            },
        },
        None,
    ))
    .await
    .expect("save a pre-call failure");

    let called = repo
        .get_session(&CoachingSessionId::new("sess-called"))
        .await
        .expect("get")
        .expect("present");
    assert_eq!(called.llm_call_id, Some(LlmCallId::new("call-1")));

    let precall = repo
        .get_session(&CoachingSessionId::new("sess-precall"))
        .await
        .expect("get")
        .expect("present");
    assert!(
        precall.llm_call_id.is_none(),
        "a pre-call failure records no ledger row"
    );

    // And the column really is NULL, not an empty string.
    let raw: Option<String> =
        sqlx::query_scalar("SELECT llm_call_id FROM coaching_sessions WHERE id = 'sess-precall'")
            .fetch_one(&pool)
            .await
            .expect("read llm_call_id");
    assert!(raw.is_none(), "the stored column is SQL NULL, got {raw:?}");
}

// ---------------------------------------------------------------------------
// 4. Disposition — dormant until r1.s4, exercised here
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recording_a_disposition_persists_the_child_version_only_when_accepted() {
    let (repo, pool, _tmp) = repo().await;
    let id = CoachingSessionId::new("sess-1");
    repo.save_session(&session(
        "sess-1",
        "run-1",
        SessionOutcome::Proposed {
            proposal: a_proposal(),
        },
        Some("call-1"),
    ))
    .await
    .expect("save");

    // A non-accepting disposition leaves the child version NULL.
    repo.record_disposition(&id, &Disposition::Rejected)
        .await
        .expect("record rejected");
    let child: Option<String> =
        sqlx::query_scalar("SELECT child_version_id FROM coaching_proposals WHERE session_id = ?1")
            .bind("sess-1")
            .fetch_one(&pool)
            .await
            .expect("read child_version_id");
    assert!(
        child.is_none(),
        "only an accepted proposal names a child version, got {child:?}"
    );
    let got = repo.get_session(&id).await.expect("get").expect("present");
    match &got.outcome {
        SessionOutcome::Proposed { proposal } => {
            assert_eq!(proposal.disposition, Disposition::Rejected);
        }
        SessionOutcome::Failed { .. } => panic!("still a proposal session"),
    }

    // Accepting persists the child version — from a FRESH proposal, because
    // `Rejected` is terminal (see the transition tests below).
    let id = CoachingSessionId::new("sess-2");
    repo.save_session(&session(
        "sess-2",
        "run-1",
        SessionOutcome::Proposed {
            proposal: a_proposal(),
        },
        Some("call-1"),
    ))
    .await
    .expect("save");
    repo.record_disposition(
        &id,
        &Disposition::Accepted {
            child_version_id: VersionId::new("ver-2"),
        },
    )
    .await
    .expect("record accepted");
    let got = repo.get_session(&id).await.expect("get").expect("present");
    match &got.outcome {
        SessionOutcome::Proposed { proposal } => {
            assert_eq!(
                proposal.disposition,
                Disposition::Accepted {
                    child_version_id: VersionId::new("ver-2"),
                },
                "the accepted disposition reads back with its child version"
            );
        }
        SessionOutcome::Failed { .. } => panic!("still a proposal session"),
    }

    // Recording a disposition on a FAILED session has no proposal to move.
    repo.save_session(&session(
        "sess-failed",
        "run-1",
        SessionOutcome::Failed {
            failure: CoachFailure::ZeroCalls,
        },
        Some("call-1"),
    ))
    .await
    .expect("save a failed turn");
    assert!(
        repo.record_disposition(
            &CoachingSessionId::new("sess-failed"),
            &Disposition::Rejected
        )
        .await
        .is_err(),
        "a failed turn has no proposal to disposition"
    );
}

/// Seed one `proposed` session and return its id.
async fn a_proposed_session(repo: &SqliteCoachingRepo<FakeClock>, id: &str) -> CoachingSessionId {
    repo.save_session(&session(
        id,
        "run-1",
        SessionOutcome::Proposed {
            proposal: a_proposal(),
        },
        Some("call-1"),
    ))
    .await
    .expect("save");
    CoachingSessionId::new(id)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_settled_proposal_cannot_be_re_dispositioned() {
    let (repo, pool, _tmp) = repo().await;
    let id = a_proposed_session(&repo, "sess-1").await;

    repo.record_disposition(
        &id,
        &Disposition::Accepted {
            child_version_id: VersionId::new("ver-2"),
        },
    )
    .await
    .expect("accept an open proposal");

    // `Accepted` is terminal (`Proposal::transition`). An unconditional UPDATE
    // would re-point this row at a second child version and report success — the
    // exact thing the session-id accept key exists to prevent.
    let second_child = repo
        .record_disposition(
            &id,
            &Disposition::Accepted {
                child_version_id: VersionId::new("ver-3"),
            },
        )
        .await;
    assert!(
        second_child.is_err(),
        "an accepted proposal must not gain a second child version"
    );
    assert!(
        repo.record_disposition(&id, &Disposition::Rejected)
            .await
            .is_err(),
        "an accepted proposal must not be re-recorded as rejected"
    );

    // The row is untouched by the refused writes.
    let child: Option<String> =
        sqlx::query_scalar("SELECT child_version_id FROM coaching_proposals WHERE session_id = ?1")
            .bind("sess-1")
            .fetch_one(&pool)
            .await
            .expect("read child_version_id");
    assert_eq!(
        child.as_deref(),
        Some("ver-2"),
        "the refused writes changed nothing"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replaying_the_same_accept_is_a_no_op() {
    let (repo, _pool, _tmp) = repo().await;
    let id = a_proposed_session(&repo, "sess-1").await;
    let accept = Disposition::Accepted {
        child_version_id: VersionId::new("ver-2"),
    };

    repo.record_disposition(&id, &accept)
        .await
        .expect("the accept");
    // The session id IS the accept idempotency key: a retry of the SAME accept has
    // to succeed, or a client that lost the response can never safely retry.
    repo.record_disposition(&id, &accept)
        .await
        .expect("replaying the identical accept is idempotent");

    let got = repo.get_session(&id).await.expect("get").expect("present");
    match &got.outcome {
        SessionOutcome::Proposed { proposal } => assert_eq!(proposal.disposition, accept),
        SessionOutcome::Failed { .. } => panic!("still a proposal session"),
    }
}

/// An accept names the child version the mutation MINTED, and the schema can only
/// say "some version exists" (PR #128, finding G2). Three shapes are individually
/// legal rows and jointly a false provenance claim: a root version (no parent at
/// all), a version parented elsewhere, and a version belonging to another strategy.
/// Each is refused inside the same transaction, before the state update.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accepting_a_root_version_as_a_child_is_refused() {
    let (repo, pool, _tmp) = repo().await;
    let id = a_proposed_session(&repo, "sess-1").await;

    let outcome = repo
        .record_disposition(
            &id,
            &Disposition::Accepted {
                child_version_id: VersionId::new("ver-root"),
            },
        )
        .await;

    assert!(
        outcome.is_err(),
        "a version with no parent cannot be this proposal's child"
    );
    assert_untouched(&pool, "sess-1").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accepting_a_version_parented_elsewhere_is_refused() {
    let (repo, pool, _tmp) = repo().await;
    let id = a_proposed_session(&repo, "sess-1").await;

    // `ver-sibling` descends from `ver-root`, not from the version this session
    // coached — a real row, and the wrong lineage.
    let outcome = repo
        .record_disposition(
            &id,
            &Disposition::Accepted {
                child_version_id: VersionId::new("ver-sibling"),
            },
        )
        .await;

    assert!(
        outcome.is_err(),
        "only a DIRECT child of the coached version may be accepted"
    );
    assert_untouched(&pool, "sess-1").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accepting_a_child_from_another_strategy_is_refused() {
    let (repo, pool, _tmp) = repo().await;
    let id = a_proposed_session(&repo, "sess-1").await;

    // `ver-foreign` names `ver-1` as its parent and still belongs to `strat-2`, so
    // the parent check alone would pass it — the lineage check is what does not.
    let outcome = repo
        .record_disposition(
            &id,
            &Disposition::Accepted {
                child_version_id: VersionId::new("ver-foreign"),
            },
        )
        .await;

    assert!(
        outcome.is_err(),
        "an accept may not move a proposal into another strategy's tree"
    );
    assert_untouched(&pool, "sess-1").await;
}

/// The proposal row is still open and unsettled after a refused accept.
async fn assert_untouched(pool: &SqlitePool, session_id: &str) {
    let (disposition, child): (String, Option<String>) = sqlx::query_as(
        "SELECT disposition, child_version_id FROM coaching_proposals WHERE session_id = ?1",
    )
    .bind(session_id)
    .fetch_one(pool)
    .await
    .expect("read the proposal row");

    assert_eq!(disposition, "proposed", "a refused accept settles nothing");
    assert!(
        child.is_none(),
        "a refused accept names no child, got {child:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn modified_and_proposed_are_refused_as_targets() {
    let (repo, _pool, _tmp) = repo().await;
    let id = a_proposed_session(&repo, "sess-1").await;

    // A modify is an EDIT: it replaces the proposal's stored mutation. Recording
    // the state alone would leave a row that says "edited" while carrying the
    // un-edited mutation, so this operation refuses it rather than half-writing it.
    assert!(
        repo.record_disposition(&id, &Disposition::Modified)
            .await
            .is_err(),
        "a `modified` disposition must be written with the edited mutation, not alone"
    );
    // Nothing returns to the initial state.
    assert!(
        repo.record_disposition(&id, &Disposition::Proposed)
            .await
            .is_err(),
        "nothing returns to `proposed`"
    );

    let got = repo.get_session(&id).await.expect("get").expect("present");
    match &got.outcome {
        SessionOutcome::Proposed { proposal } => {
            assert_eq!(
                proposal.disposition,
                Disposition::Proposed,
                "the refused writes left the proposal open"
            );
        }
        SessionOutcome::Failed { .. } => panic!("still a proposal session"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failure_kind_that_disagrees_with_its_detail_is_refused_on_read() {
    let (repo, pool, _tmp) = repo().await;
    repo.save_session(&session(
        "sess-1",
        "run-1",
        SessionOutcome::Failed {
            failure: CoachFailure::ZeroCalls,
        },
        Some("call-1"),
    ))
    .await
    .expect("save a failed turn");

    // `failure_kind` is the QUERYABLE column — an audit scan for every
    // `provider_timeout` reads it, not the JSON. A row whose tag disagrees with its
    // detail would answer that scan wrongly, which is worse than answering with an
    // error.
    sqlx::query(
        "UPDATE coaching_sessions SET failure_kind = 'provider_timeout' WHERE id = 'sess-1'",
    )
    .execute(&pool)
    .await
    .expect("desync the kind from the detail");

    let err = repo
        .get_session(&CoachingSessionId::new("sess-1"))
        .await
        .expect_err("a disagreeing failure_kind must fail closed");
    let message = err.to_string();
    assert!(
        message.contains("provider_timeout") && message.contains("zero_calls"),
        "the error must name both sides of the disagreement: {message}"
    );
}

// ---------------------------------------------------------------------------
// 5. One proposal per session, end to end
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_session_carries_at_most_one_proposal() {
    let (repo, pool, _tmp) = repo().await;
    let first = session(
        "sess-1",
        "run-1",
        SessionOutcome::Proposed {
            proposal: a_proposal(),
        },
        Some("call-1"),
    );
    repo.save_session(&first).await.expect("the first save");

    // Re-saving the same session id is the retry the accept idempotency key exists
    // to make safe: it must be refused, not silently doubled.
    let again = repo.save_session(&first).await;
    assert!(
        again.is_err(),
        "a second proposal for the same session must be refused"
    );

    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM coaching_proposals WHERE session_id = 'sess-1'")
            .fetch_one(&pool)
            .await
            .expect("count proposals");
    assert_eq!(count, 1, "exactly one proposal row survives the retry");
}

// ---------------------------------------------------------------------------
// 6. Listing a run's turns
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn list_sessions_for_run_returns_that_runs_turns() {
    let (repo, _pool, _tmp) = repo().await;

    repo.save_session(&session(
        "sess-1",
        "run-1",
        SessionOutcome::Proposed {
            proposal: a_proposal(),
        },
        Some("call-1"),
    ))
    .await
    .expect("save");
    repo.save_session(&session(
        "sess-2",
        "run-1",
        SessionOutcome::Failed {
            failure: CoachFailure::ZeroCalls,
        },
        Some("call-1"),
    ))
    .await
    .expect("save");
    repo.save_session(&session(
        "sess-3",
        "run-2",
        SessionOutcome::Failed {
            failure: CoachFailure::ProviderTimeout { elapsed_ms: 1_000 },
        },
        Some("call-1"),
    ))
    .await
    .expect("save");

    let run_1 = repo
        .list_sessions_for_run(&BacktestRunId::new("run-1"))
        .await
        .expect("list run-1");
    let ids: Vec<&str> = run_1.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["sess-1", "sess-2"],
        "both of run-1's turns, and only those"
    );

    // Both outcome kinds come back — a failed turn is a turn.
    assert!(matches!(run_1[0].outcome, SessionOutcome::Proposed { .. }));
    assert!(matches!(run_1[1].outcome, SessionOutcome::Failed { .. }));

    let none = repo
        .list_sessions_for_run(&BacktestRunId::new("run-absent"))
        .await
        .expect("list an unknown run");
    assert!(none.is_empty(), "an unknown run has no turns");
}
