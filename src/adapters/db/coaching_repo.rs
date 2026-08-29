//! The `SQLite` adapter implementing the [`CoachingRepository`] port (r1.s2.w2,
//! ADR-0021 / audit C3).
//!
//! This is the ONLY place `query!` macros for the `coaching_sessions` /
//! `coaching_proposals` tables live (`sqlx` is confined to `adapters::db`, mirror
//! `llm_call_repo.rs:1-9`); the committed `.sqlx/` offline cache is keyed to the
//! macros in this file (regenerate with `just prepare` under sqlx-cli `=0.8.6` —
//! #41: a floating install can drift out of sync with the pinned `sqlx` crate and
//! desync the cache).
//!
//! **A turn is one transaction.** A proposal turn writes two rows — the session and
//! its proposal — and a half-written turn would be exactly the silence the
//! capability sentence forbids, so `save_session` commits both or neither.
//!
//! **Typed projection, never a blob.** The session's identity, its run/version
//! references and its outcome are explicit columns; only the two payloads that are
//! themselves typed domain values (the `Mutation` and the `CoachFailure`) are
//! stored as serde JSON, so the coach's proposal and its failure reason come back
//! as the types they went in as rather than as prose.
//!
//! **`created_at` from the injected `Clock`** (mirror `SqliteLlmCallRepo`): the
//! stored timestamp is the adapter's, serialized RFC3339-millis, deterministic
//! under a `FakeClock`.
//!
//! **`schema_version` read-reject (#68, mirror `LLM_CALL_SCHEMA_VERSION`).** An
//! unknown stored tag is a fail-closed [`DataError::Db`], never a silent partial.
//!
//! **No `validated` column, and none is read or written here** (audit C4): a
//! mutation's applicability is re-established by
//! [`apply`](crate::domain::apply) at use time.
//!
//! NO `#[derive(Debug)]` on the repo struct: the `C: Clock` carries no `Debug`
//! bound (mirror `SqliteLlmCallRepo`).

use chrono::{DateTime, SecondsFormat};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::adapters::clock::SystemClock;
use crate::domain::backtest::BacktestRunId;
use crate::domain::strategy::VersionId;
use crate::domain::{
    Clock, CoachFailure, CoachingRepository, CoachingSession, CoachingSessionId, DataError,
    Disposition, Hypothesis, LlmCallId, Mutation, Proposal, SessionOutcome,
};

/// The row-schema tag `save_session` writes into every
/// `coaching_sessions.schema_version` and that every read ASSERTS (mirror
/// `LLM_CALL_SCHEMA_VERSION`, #68). v1 reads only v1 and rejects the rest with a
/// real [`DataError::Db`].
const COACHING_SCHEMA_VERSION: i64 = 1;

/// The `outcome` column's two values, matching migration `0005`'s `CHECK`.
const OUTCOME_PROPOSED: &str = "proposed";
const OUTCOME_FAILED: &str = "failed";

/// The `SQLite` [`CoachingRepository`](crate::domain::port::CoachingRepository)
/// adapter over `pulse.db`.
///
/// Constructed from a [`SqlitePool`] (cloned from `Db::pool()`). Carries an
/// injected [`Clock`] (the `created_at` source).
pub struct SqliteCoachingRepo<C: Clock> {
    pool: SqlitePool,
    clock: C,
}

impl SqliteCoachingRepo<SystemClock> {
    /// The production constructor: the wall-clock [`SystemClock`].
    #[must_use]
    pub fn new(pool: SqlitePool) -> SqliteCoachingRepo<SystemClock> {
        SqliteCoachingRepo {
            pool,
            clock: SystemClock,
        }
    }
}

impl<C: Clock> SqliteCoachingRepo<C> {
    /// The test/injection seam: supply a [`Clock`] so `created_at` is
    /// deterministic (mirror `SqliteLlmCallRepo::with_deps`).
    #[must_use]
    pub fn with_deps(pool: SqlitePool, clock: C) -> SqliteCoachingRepo<C> {
        SqliteCoachingRepo { pool, clock }
    }

    /// The current `created_at`, sourced from the injected [`Clock`], serialized
    /// as an RFC3339 millisecond UTC string for the `TEXT` column.
    fn now_rfc3339(&self) -> Result<String, DataError> {
        let now_ms = self.clock.now_ms();
        let dt = DateTime::from_timestamp_millis(now_ms).ok_or_else(|| {
            DataError::Db(format!("clock.now_ms() {now_ms} is out of DateTime range"))
        })?;
        Ok(dt.to_rfc3339_opts(SecondsFormat::Millis, true))
    }
}

/// The `failure_kind` column value for a failure — the `snake_case` tag migration
/// `0005`'s `CHECK` enumerates. Written explicitly rather than derived from serde
/// so the column's vocabulary and the schema's `CHECK` cannot drift apart silently:
/// adding a `CoachFailure` variant fails to compile here until both are updated.
fn failure_kind(failure: &CoachFailure) -> &'static str {
    match failure {
        CoachFailure::ZeroCalls => "zero_calls",
        CoachFailure::SeveralCalls { .. } => "several_calls",
        CoachFailure::MalformedArguments { .. } => "malformed_arguments",
        CoachFailure::InapplicableMutation { .. } => "inapplicable_mutation",
        CoachFailure::ProviderTimeout { .. } => "provider_timeout",
        CoachFailure::ContextOverflow { .. } => "context_overflow",
    }
}

/// The `disposition` column value — the tag migration `0005`'s `CHECK` enumerates.
fn disposition_tag(disposition: &Disposition) -> &'static str {
    match disposition {
        Disposition::Proposed => "proposed",
        Disposition::Accepted { .. } => "accepted",
        Disposition::Rejected => "rejected",
        Disposition::Modified => "modified",
    }
}

/// Parse a JSON `TEXT` column into a deserializable value, fail-closed on a
/// malformed payload (mirror `llm_call_repo::parse_json`).
fn parse_json<T: serde::de::DeserializeOwned>(column: &str, s: &str) -> Result<T, DataError> {
    serde_json::from_str(s)
        .map_err(|e| DataError::Db(format!("malformed JSON in `{column}` = `{s}`: {e}")))
}

/// Rebuild a [`Disposition`] from its stored tag + nullable child version.
///
/// Fail-closed: the `0005` `CHECK` already guarantees `accepted` iff a child
/// version, so a row that violates it is corrupt and must not read back as a
/// plausible state.
fn parse_disposition(
    tag: &str,
    child_version_id: Option<String>,
) -> Result<Disposition, DataError> {
    match (tag, child_version_id) {
        ("proposed", None) => Ok(Disposition::Proposed),
        ("rejected", None) => Ok(Disposition::Rejected),
        ("modified", None) => Ok(Disposition::Modified),
        ("accepted", Some(child)) => Ok(Disposition::Accepted {
            child_version_id: VersionId::new(child),
        }),
        ("accepted", None) => Err(DataError::Db(
            "coaching_proposals: an accepted proposal has no child_version_id".to_owned(),
        )),
        (tag @ ("proposed" | "rejected" | "modified"), Some(child)) => Err(DataError::Db(format!(
            "coaching_proposals: disposition `{tag}` carries child_version_id `{child}`"
        ))),
        (other, _) => Err(DataError::Db(format!(
            "coaching_proposals: unknown disposition `{other}`"
        ))),
    }
}

impl<C: Clock + Send + Sync> CoachingRepository for SqliteCoachingRepo<C> {
    async fn save_session(
        &self,
        session: &CoachingSession,
    ) -> Result<CoachingSessionId, DataError> {
        let id = session.id.as_str().to_owned();
        let backtest_run_id = session.backtest_run_id.as_str().to_owned();
        let strategy_version_id = session.strategy_version_id.as_str().to_owned();
        // `created_at` from the injected Clock, NOT the in-memory value.
        let created_at = self.now_rfc3339()?;
        let llm_call_id = session.llm_call_id.as_ref().map(|c| c.as_str().to_owned());
        let schema_version = COACHING_SCHEMA_VERSION;

        let (outcome, failure_kind_col, failure_detail) = match &session.outcome {
            SessionOutcome::Proposed { .. } => (OUTCOME_PROPOSED, None, None),
            SessionOutcome::Failed { failure } => {
                let detail =
                    serde_json::to_string(failure).map_err(|e| DataError::Db(e.to_string()))?;
                (OUTCOME_FAILED, Some(failure_kind(failure)), Some(detail))
            }
        };

        // One turn, one transaction: a session without its proposal would be the
        // half-written record the never-silence guarantee exists to prevent.
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| DataError::Db(e.to_string()))?;

        sqlx::query!(
            "INSERT INTO coaching_sessions \
             (id, backtest_run_id, strategy_version_id, created_at, llm_call_id, outcome, \
              failure_kind, failure_detail, schema_version) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            id,
            backtest_run_id,
            strategy_version_id,
            created_at,
            llm_call_id,
            outcome,
            failure_kind_col,
            failure_detail,
            schema_version,
        )
        .execute(&mut *tx)
        .await
        .map_err(|e| DataError::Db(e.to_string()))?;

        if let SessionOutcome::Proposed { proposal } = &session.outcome {
            let proposal_id = Uuid::new_v4().to_string();
            let mutation = serde_json::to_string(&proposal.mutation)
                .map_err(|e| DataError::Db(e.to_string()))?;
            let hypothesis = proposal.hypothesis.as_str().to_owned();
            let disposition = disposition_tag(&proposal.disposition);
            let child_version_id = proposal
                .disposition
                .child_version_id()
                .map(|v| v.as_str().to_owned());

            sqlx::query!(
                "INSERT INTO coaching_proposals \
                 (id, session_id, mutation, hypothesis, disposition, child_version_id) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                proposal_id,
                id,
                mutation,
                hypothesis,
                disposition,
                child_version_id,
            )
            .execute(&mut *tx)
            .await
            .map_err(|e| DataError::Db(e.to_string()))?;
        }

        tx.commit()
            .await
            .map_err(|e| DataError::Db(e.to_string()))?;

        Ok(CoachingSessionId::new(id))
    }

    async fn get_session(
        &self,
        id: &CoachingSessionId,
    ) -> Result<Option<CoachingSession>, DataError> {
        let id_str = id.as_str();
        let row = sqlx::query!(
            r#"SELECT
                 id                  AS "id!: String",
                 backtest_run_id     AS "backtest_run_id!: String",
                 strategy_version_id AS "strategy_version_id!: String",
                 created_at          AS "created_at!: String",
                 llm_call_id         AS "llm_call_id?: String",
                 outcome             AS "outcome!: String",
                 failure_detail      AS "failure_detail?: String",
                 schema_version      AS "schema_version!: i64"
               FROM coaching_sessions WHERE id = ?1"#,
            id_str,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| DataError::Db(e.to_string()))?;

        let Some(r) = row else { return Ok(None) };

        // The stored schema_version is load-bearing — reject an unsupported tag
        // hard (fail-closed, mirror `get_call`).
        if r.schema_version != COACHING_SCHEMA_VERSION {
            return Err(DataError::Db(format!(
                "unsupported coaching_sessions schema version {}",
                r.schema_version
            )));
        }

        let outcome = match r.outcome.as_str() {
            OUTCOME_PROPOSED => {
                let proposal = self.fetch_proposal(&r.id).await?.ok_or_else(|| {
                    DataError::Db(format!(
                        "coaching_sessions `{}` records a proposal but has no proposal row",
                        r.id
                    ))
                })?;
                SessionOutcome::Proposed { proposal }
            }
            OUTCOME_FAILED => {
                let detail = r.failure_detail.ok_or_else(|| {
                    DataError::Db(format!(
                        "coaching_sessions `{}` records a failure with no failure_detail",
                        r.id
                    ))
                })?;
                let failure: CoachFailure =
                    parse_json("coaching_sessions.failure_detail", &detail)?;
                SessionOutcome::Failed { failure }
            }
            other => {
                return Err(DataError::Db(format!(
                    "coaching_sessions: unknown outcome `{other}`"
                )));
            }
        };

        Ok(Some(CoachingSession {
            id: CoachingSessionId::new(r.id),
            backtest_run_id: BacktestRunId::new(r.backtest_run_id),
            strategy_version_id: VersionId::new(r.strategy_version_id),
            created_at: r.created_at,
            llm_call_id: r.llm_call_id.map(LlmCallId::new),
            outcome,
        }))
    }

    async fn list_sessions_for_run(
        &self,
        run_id: &BacktestRunId,
    ) -> Result<Vec<CoachingSession>, DataError> {
        let run = run_id.as_str();
        // Ids first, then the full read per id: `get_session` is the one place the
        // fail-closed decoding lives, and a list that decoded rows a second way is
        // how the two drift apart. The index `idx_coaching_sessions_run` serves the
        // ordering; a run's turn count is small by construction (one per coach ask).
        let ids: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM coaching_sessions WHERE backtest_run_id = ?1 \
             ORDER BY created_at, id",
        )
        .bind(run)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DataError::Db(e.to_string()))?;

        let mut sessions = Vec::with_capacity(ids.len());
        for id in ids {
            let session_id = CoachingSessionId::new(id);
            // Fail-closed: a row listed and then unreadable is a corrupt record, not
            // a gap to skip (the `get_run` posture, not the catalog one).
            let session = self.get_session(&session_id).await?.ok_or_else(|| {
                DataError::Db(format!(
                    "coaching_sessions `{}` vanished between listing and read",
                    session_id.as_str()
                ))
            })?;
            sessions.push(session);
        }
        Ok(sessions)
    }

    async fn record_disposition(
        &self,
        id: &CoachingSessionId,
        disposition: &Disposition,
    ) -> Result<(), DataError> {
        let id_str = id.as_str();
        let tag = disposition_tag(disposition);
        let child_version_id = disposition
            .child_version_id()
            .map(|v| v.as_str().to_owned());

        let affected = sqlx::query!(
            "UPDATE coaching_proposals SET disposition = ?1, child_version_id = ?2 \
             WHERE session_id = ?3",
            tag,
            child_version_id,
            id_str,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| DataError::Db(e.to_string()))?
        .rows_affected();

        if affected == 0 {
            return Err(DataError::Db(format!(
                "no proposal to disposition for coaching session `{id_str}` \
                 (absent session, or a turn that failed)"
            )));
        }
        Ok(())
    }
}

impl<C: Clock> SqliteCoachingRepo<C> {
    /// The proposal row for a session, decoded into the domain type.
    async fn fetch_proposal(&self, session_id: &str) -> Result<Option<Proposal>, DataError> {
        let row = sqlx::query!(
            r#"SELECT
                 mutation         AS "mutation!: String",
                 hypothesis       AS "hypothesis!: String",
                 disposition      AS "disposition!: String",
                 child_version_id AS "child_version_id?: String"
               FROM coaching_proposals WHERE session_id = ?1"#,
            session_id,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| DataError::Db(e.to_string()))?;

        let Some(r) = row else { return Ok(None) };

        let mutation: Mutation = parse_json("coaching_proposals.mutation", &r.mutation)?;
        // A stored hypothesis that is blank is a corrupt row, not an empty one: the
        // domain type refuses it and so does the `0005` CHECK.
        let hypothesis = Hypothesis::new(r.hypothesis)
            .map_err(|e| DataError::Db(format!("coaching_proposals.hypothesis: {e}")))?;
        let disposition = parse_disposition(&r.disposition, r.child_version_id)?;

        Ok(Some(Proposal {
            mutation,
            hypothesis,
            disposition,
        }))
    }
}
