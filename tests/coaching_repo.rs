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

    for (id, hash, by) in [
        ("ver-1", "hash-1", "human"),
        ("ver-2", "hash-2", "coach_llm"),
    ] {
        sqlx::query(
            "INSERT INTO strategy_version \
             (id, strategy_id, dsl_schema_version, dsl, dsl_original, version_hash, created_by, \
              creating_llm_call_ids, created_at) \
             VALUES (?1, 'strat-1', '1.0.0', '{}', '{}', ?2, ?3, '[]', '2026-08-29T00:00:00.000Z')",
        )
        .bind(id)
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

    let failures = vec![
        CoachFailure::ZeroCalls,
        CoachFailure::SeveralCalls { count: 3 },
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
    ];

    for (i, failure) in failures.into_iter().enumerate() {
        let id = format!("sess-fail-{i}");
        // A pre-call failure records no ledger row; the rest name their call.
        let call = match &failure {
            CoachFailure::ContextOverflow { .. } => None,
            _ => Some("call-1"),
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
    repo.record_disposition(&id, &Disposition::Modified)
        .await
        .expect("record modified");
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
            assert_eq!(proposal.disposition, Disposition::Modified);
        }
        SessionOutcome::Failed { .. } => panic!("still a proposal session"),
    }

    // Accepting persists the child version.
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
