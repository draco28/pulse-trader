//! The `SQLite` adapter implementing the [`LlmCallRepository`] port (VS-1.3.1
//! work-1.02, FR-24 / NFR-2, README C6).
//!
//! This is the ONLY place `query!` macros for the `llm_call` table live (`sqlx` is
//! confined to `adapters::db`, mirror `backtest_run_repo.rs:1-6`); the committed
//! `.sqlx/` offline cache is keyed to the macros in this file (regenerate with
//! `cargo sqlx prepare` under sqlx-cli `=0.8.6` — #41: a floating install pulls
//! 0.9.0 and fails on the pinned rustc 1.92).
//!
//! **Typed projection, never a blob.** `save_call` writes EXPLICIT columns per
//! README C6 + the `schema_version` tag; read-back is independent of serde
//! field-presence.
//!
//! **Decimal-as-TEXT (D2 / NFR-2).** `cost` is stored via `.normalize().to_string()`
//! — the same canonicalization the domain uses — so a reloaded call carries the
//! byte-identical `Decimal`; it is a `TEXT` column, never an f64 one. `cost_currency`
//! is the price table's NATIVE billing currency (audit ch3 — GLM bills `CNY`; no
//! silent FX).
//!
//! **`created_at` from the injected `Clock` (D7).** The stored timestamp is sourced
//! from the adapter's [`Clock`], serialized RFC3339-millis — deterministic under a
//! `FakeClock`; every other `LlmCall` field is persisted verbatim.
//!
//! **`schema_version` read-reject (#68, mirror `RUN_SCHEMA_VERSION`).** An unknown
//! stored `schema_version` is a fail-closed [`DataError::Db`], not a silent partial.
//!
//! **Immutability is structural.** There is no `update_call`/`delete_call`; the
//! migration-`0004` `BEFORE UPDATE`/`BEFORE DELETE` triggers `RAISE(ABORT, ...)` on
//! any mutation, surfacing as a sqlx error → [`DataError::Db`].
//!
//! NO `#[derive(Debug)]` on the repo struct: the `C: Clock` carries no `Debug` bound
//! (mirror `SqliteBacktestRunRepo`).

use chrono::{DateTime, SecondsFormat, Utc};
use rust_decimal::Decimal;
use sqlx::SqlitePool;

use crate::adapters::clock::SystemClock;
use crate::domain::strategy::CreatedBy;
use crate::domain::{Clock, DataError, LlmBackend, LlmCall, LlmCallId, LlmCallRepository, Message};

/// The row-schema tag `save_call` writes into every `llm_call.schema_version` and
/// that every read ASSERTS (mirror `RUN_SCHEMA_VERSION`, #68). v1 reads only v1 and
/// rejects the rest with a real [`DataError::Db`] — a load-bearing read control
/// point, not a ceremonial column.
const LLM_CALL_SCHEMA_VERSION: i64 = 1;

/// The `SQLite` [`LlmCallRepository`](crate::domain::port::LlmCallRepository) adapter
/// over `pulse.db`.
///
/// Constructed from a [`SqlitePool`] (cloned from `Db::pool()`). Carries an injected
/// [`Clock`] (the `created_at` source, D7).
///
/// No `#[derive(Debug)]`: `C: Clock` carries no `Debug` bound (mirror
/// `SqliteBacktestRunRepo`).
pub struct SqliteLlmCallRepo<C: Clock> {
    pool: SqlitePool,
    clock: C,
}

impl SqliteLlmCallRepo<SystemClock> {
    /// The production constructor: the wall-clock [`SystemClock`].
    #[must_use]
    pub fn new(pool: SqlitePool) -> SqliteLlmCallRepo<SystemClock> {
        SqliteLlmCallRepo {
            pool,
            clock: SystemClock,
        }
    }
}

impl<C: Clock> SqliteLlmCallRepo<C> {
    /// The test/injection seam: supply a [`Clock`] so `created_at` is deterministic
    /// (mirror `SqliteBacktestRunRepo::with_deps`).
    #[must_use]
    pub fn with_deps(pool: SqlitePool, clock: C) -> SqliteLlmCallRepo<C> {
        SqliteLlmCallRepo { pool, clock }
    }

    /// The current `created_at`, sourced from the injected [`Clock`] (D7), serialized
    /// as an RFC3339 millisecond UTC string for the `TEXT` column.
    fn now_rfc3339(&self) -> Result<String, DataError> {
        let now_ms = self.clock.now_ms();
        let dt = DateTime::from_timestamp_millis(now_ms).ok_or_else(|| {
            DataError::Db(format!("clock.now_ms() {now_ms} is out of DateTime range"))
        })?;
        Ok(dt.to_rfc3339_opts(SecondsFormat::Millis, true))
    }
}

/// Canonicalize a `Decimal` for storage as `.normalize().to_string()` (D2 / NFR-2)
/// so a reloaded call carries a byte-identical value; `Decimal` has no
/// `-0`/`NaN`/`Inf`, so this is total.
fn decimal_text(value: Decimal) -> String {
    value.normalize().to_string()
}

/// Parse a `Decimal` `TEXT` column back, fail-closed on a malformed value.
fn parse_decimal(column: &str, s: &str) -> Result<Decimal, DataError> {
    s.parse::<Decimal>()
        .map_err(|e| DataError::Db(format!("malformed Decimal in `{column}` = `{s}`: {e}")))
}

/// The bare `snake_case` token an enum serializes to, for a `TEXT` column (strips the
/// JSON quotes `serde_json::to_string` adds) — mirror `backtest_run_repo::enum_token`.
fn enum_token<T: serde::Serialize>(value: &T) -> Result<String, DataError> {
    let quoted = serde_json::to_string(value).map_err(|e| DataError::Db(e.to_string()))?;
    Ok(quoted.trim_matches('"').to_owned())
}

/// Wrap a bare `snake_case` enum token in JSON quotes so `serde_json` can decode it
/// (mirror `backtest_run_repo::json_token`).
fn json_token(s: &str) -> String {
    format!("\"{s}\"")
}

/// Parse a JSON `TEXT` column into a deserializable value, fail-closed on a malformed
/// payload.
fn parse_json<T: serde::de::DeserializeOwned>(column: &str, s: &str) -> Result<T, DataError> {
    serde_json::from_str(s)
        .map_err(|e| DataError::Db(format!("malformed JSON in `{column}` = `{s}`: {e}")))
}

impl<C: Clock + Send + Sync> LlmCallRepository for SqliteLlmCallRepo<C> {
    async fn save_call(&self, call: &LlmCall) -> Result<LlmCallId, DataError> {
        let id = call.id.as_str().to_owned();
        let backend = enum_token(&call.backend)?;
        let model = call.model.clone();
        let prompt_messages = serde_json::to_string(&call.prompt_messages)
            .map_err(|e| DataError::Db(e.to_string()))?;
        let completion = call.completion.clone();
        let input_tokens = i64::from(call.input_tokens);
        let output_tokens = i64::from(call.output_tokens);
        let cost = decimal_text(call.cost);
        let cost_currency = call.cost_currency.clone();
        // `created_at` from the injected Clock (D7), NOT the in-memory value.
        let created_at = self.now_rfc3339()?;
        let created_by = enum_token(&call.created_by)?;
        let schema_version = LLM_CALL_SCHEMA_VERSION;

        sqlx::query!(
            "INSERT INTO llm_call \
             (id, backend, model, prompt_messages, completion, input_tokens, output_tokens, \
              cost, cost_currency, created_at, created_by, schema_version) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            id,
            backend,
            model,
            prompt_messages,
            completion,
            input_tokens,
            output_tokens,
            cost,
            cost_currency,
            created_at,
            created_by,
            schema_version,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| DataError::Db(e.to_string()))?;

        Ok(LlmCallId::new(id))
    }

    async fn get_call(&self, id: &LlmCallId) -> Result<Option<LlmCall>, DataError> {
        let id_str = id.as_str();
        let row = sqlx::query!(
            r#"SELECT
                 id               AS "id!: String",
                 backend          AS "backend!: String",
                 model            AS "model!: String",
                 prompt_messages  AS "prompt_messages!: String",
                 completion       AS "completion?: String",
                 input_tokens     AS "input_tokens!: i64",
                 output_tokens    AS "output_tokens!: i64",
                 cost             AS "cost!: String",
                 cost_currency    AS "cost_currency!: String",
                 created_at       AS "created_at!: String",
                 created_by       AS "created_by!: String",
                 schema_version   AS "schema_version!: i64"
               FROM llm_call WHERE id = ?1"#,
            id_str,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| DataError::Db(e.to_string()))?;

        let Some(r) = row else { return Ok(None) };

        // The stored schema_version is load-bearing — reject an unsupported tag hard
        // (fail-closed, mirror `get_run`).
        if r.schema_version != LLM_CALL_SCHEMA_VERSION {
            return Err(DataError::Db(format!(
                "unsupported llm_call schema version {}",
                r.schema_version
            )));
        }

        let backend: LlmBackend = parse_json("llm_call.backend", &json_token(&r.backend))?;
        let prompt_messages: Vec<Message> =
            parse_json("llm_call.prompt_messages", &r.prompt_messages)?;
        let created_by: CreatedBy = parse_json("llm_call.created_by", &json_token(&r.created_by))?;
        let cost = parse_decimal("llm_call.cost", &r.cost)?;
        let created_at = DateTime::parse_from_rfc3339(&r.created_at)
            .map_err(|e| DataError::Db(format!("malformed created_at `{}`: {e}", r.created_at)))?
            .with_timezone(&Utc);
        let input_tokens = u32::try_from(r.input_tokens).map_err(|e| {
            DataError::Db(format!(
                "input_tokens {} out of u32 range: {e}",
                r.input_tokens
            ))
        })?;
        let output_tokens = u32::try_from(r.output_tokens).map_err(|e| {
            DataError::Db(format!(
                "output_tokens {} out of u32 range: {e}",
                r.output_tokens
            ))
        })?;

        Ok(Some(LlmCall {
            id: LlmCallId::new(r.id),
            backend,
            model: r.model,
            prompt_messages,
            completion: r.completion,
            input_tokens,
            output_tokens,
            cost,
            cost_currency: r.cost_currency,
            created_at,
            created_by,
        }))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{LLM_CALL_SCHEMA_VERSION, SqliteLlmCallRepo};
    use crate::adapters::clock::FakeClock;
    use crate::adapters::db::{Db, MIGRATOR};
    use crate::domain::strategy::CreatedBy;
    use crate::domain::{DataError, LlmBackend, LlmCall, LlmCallId, LlmCallRepository, Message};
    use chrono::DateTime;
    use rust_decimal::Decimal;
    use sqlx::SqlitePool;
    use tempfile::TempDir;

    /// A `(repo, pool, tempdir)` triple over a fresh migrated tempfile DB with a
    /// deterministic [`FakeClock`] pinned at `now_ms`. The `TempDir` guard keeps the
    /// scratch DB alive for the test body.
    async fn repo_at(now_ms: i64) -> (SqliteLlmCallRepo<FakeClock>, SqlitePool, TempDir) {
        let tmp = TempDir::new().expect("tempdir");
        let db = Db::with_path(&tmp.path().join("pulse.db"))
            .await
            .expect("open db");
        MIGRATOR.run(db.pool()).await.expect("run migrations");
        let pool = db.pool().clone();
        (
            SqliteLlmCallRepo::with_deps(pool.clone(), FakeClock::at(now_ms)),
            pool,
            tmp,
        )
    }

    /// A sample `LlmCall` whose `created_at` matches the `FakeClock` instant `now_ms`
    /// (so the clock-sourced stored timestamp round-trips to an EQUAL value — the
    /// adapter overrides `created_at` from the injected clock, D7).
    fn sample_call_at(now_ms: i64, id: &str) -> LlmCall {
        LlmCall {
            id: LlmCallId::new(id),
            backend: LlmBackend::Ollama,
            model: "glm-5.1".to_owned(),
            prompt_messages: vec![
                Message::system("be terse"),
                Message::user("size a BTC scalp"),
            ],
            completion: Some("done".to_owned()),
            input_tokens: 128,
            output_tokens: 42,
            cost: Decimal::new(1234, 4), // 0.1234
            cost_currency: "CNY".to_owned(),
            created_at: DateTime::from_timestamp_millis(now_ms).expect("clock in range"),
            created_by: CreatedBy::ComposerLlm,
        }
    }

    // ---- AC-7: save → get round-trips the ledger record verbatim -----------------
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn llm_call_save_get_roundtrip() {
        let now_ms = 1_700_000_000_000;
        let (repo, pool, _tmp) = repo_at(now_ms).await;
        let call = sample_call_at(now_ms, "call-1");

        let id = repo.save_call(&call).await.expect("save_call");
        assert_eq!(id, LlmCallId::new("call-1"), "save_call returns the row id");

        let got = repo
            .get_call(&id)
            .await
            .expect("get_call")
            .expect("row present");

        // Full verbatim round-trip (backend + prompt + native-currency Decimal cost).
        assert_eq!(got, call, "the ledger record round-trips verbatim");
        assert_eq!(got.cost, Decimal::new(1234, 4));
        assert_eq!(got.cost_currency, "CNY", "native billing currency, no FX");
        assert_eq!(got.backend, LlmBackend::Ollama);
        assert_eq!(got.prompt_messages.len(), 2);

        // NFR-2 / the `user:` demo criterion: `cost` is canonical decimal TEXT, not a
        // float — the raw stored column is the `.normalize()`d string, exactly.
        let raw_cost: String = sqlx::query_scalar("SELECT cost FROM llm_call WHERE id = ?1")
            .bind("call-1")
            .fetch_one(&pool)
            .await
            .expect("read raw cost");
        assert_eq!(raw_cost, "0.1234", "cost stored as canonical Decimal TEXT");

        // An absent id is `Ok(None)`, not an error.
        assert!(
            repo.get_call(&LlmCallId::new("nope"))
                .await
                .expect("get_call absent")
                .is_none()
        );
    }

    // ---- AC-6: the immutability trigger rejects an UPDATE ------------------------
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn llm_call_immutable_update_rejected() {
        let now_ms = 1_700_000_000_000;
        let (repo, pool, _tmp) = repo_at(now_ms).await;
        repo.save_call(&sample_call_at(now_ms, "call-1"))
            .await
            .expect("save_call");

        // The BEFORE UPDATE trigger RAISE(ABORT)s → a sqlx error.
        let res = sqlx::query("UPDATE llm_call SET model = 'tampered' WHERE id = ?1")
            .bind("call-1")
            .execute(&pool)
            .await;
        assert!(
            res.is_err(),
            "an UPDATE on llm_call must be rejected by the immutability trigger"
        );
        let msg = format!("{}", res.unwrap_err());
        assert!(
            msg.contains("llm_call is immutable"),
            "the trigger's RAISE message surfaces: {msg}"
        );

        // The row is unchanged.
        let got = repo
            .get_call(&LlmCallId::new("call-1"))
            .await
            .expect("get_call")
            .expect("row present");
        assert_eq!(got.model, "glm-5.1", "the row is untouched");
    }

    // ---- AC-6: the immutability trigger rejects a DELETE ------------------------
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn llm_call_immutable_delete_rejected() {
        let now_ms = 1_700_000_000_000;
        let (repo, pool, _tmp) = repo_at(now_ms).await;
        repo.save_call(&sample_call_at(now_ms, "call-1"))
            .await
            .expect("save_call");

        let res = sqlx::query("DELETE FROM llm_call WHERE id = ?1")
            .bind("call-1")
            .execute(&pool)
            .await;
        assert!(
            res.is_err(),
            "a DELETE on llm_call must be rejected by the immutability trigger"
        );
        assert!(
            format!("{}", res.unwrap_err()).contains("llm_call is immutable"),
            "the trigger's RAISE message surfaces"
        );

        // The row survives.
        assert!(
            repo.get_call(&LlmCallId::new("call-1"))
                .await
                .expect("get_call")
                .is_some(),
            "the row survives the rejected DELETE"
        );
    }

    // ---- get_call fail-closes on an unsupported stored schema_version -----------
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn llm_call_get_rejects_unsupported_schema_version() {
        let now_ms = 1_700_000_000_000;
        let (repo, pool, _tmp) = repo_at(now_ms).await;

        // Seed a row directly with a NEWER/unknown schema_version (2). get_call must
        // reject it, not silently decode it.
        sqlx::query(
            "INSERT INTO llm_call \
             (id, backend, model, prompt_messages, completion, input_tokens, output_tokens, \
              cost, cost_currency, created_at, created_by, schema_version) \
             VALUES ('call-newschema', 'glm', 'glm-5.1', '[]', NULL, 1, 1, '0', 'CNY', \
                     '2026-06-30T00:00:00.000Z', 'composer_llm', 2)",
        )
        .execute(&pool)
        .await
        .expect("seed newer-schema row");

        match repo.get_call(&LlmCallId::new("call-newschema")).await {
            Err(DataError::Db(msg)) => assert!(
                msg.contains("unsupported llm_call schema version 2"),
                "must reject the unsupported schema_version; got: {msg}"
            ),
            other => panic!("expected DataError::Db unsupported-schema, got {other:?}"),
        }

        // Sanity: the supported tag is exactly v1 (guards against a silent bump).
        assert_eq!(LLM_CALL_SCHEMA_VERSION, 1);
    }
}
