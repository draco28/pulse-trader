//! The coach turn (r1.s2.w3, ADR-0021) — one provider call in, one recorded
//! session out.
//!
//! **Never silence.** Every COACHING OUTCOME persists: a turn that produced one
//! ends by writing exactly one [`CoachingSession`] — a `Proposed` proposal, or one
//! of the SEVEN typed [`CoachFailure`]s (`TransportFailure` joined the taxonomy in
//! `r1.s2.w4`). Persistence is inside the turn rather than left to the caller on
//! purpose — a guarantee a caller can forget is not a guarantee.
//!
//! The exceptions are the typed caller, local and persistence faults below, which
//! are not coaching outcomes at all and write nothing. "Every path persists" would
//! be the tidier sentence and it is not the true one.
//!
//! The things that are NOT recorded outcomes are the ones that are not *coaching*
//! at all: a caller handing in inputs that do not belong together, a fault this
//! process raised on the call path (a failed ledger write, an unpriced model, a
//! ledger correlation that cannot be established), and a failure to write the
//! session row itself. Each surfaces as [`CoachTurnError`] at the CLI edge
//! (ADR-0017).
//!
//! That last category is the one place never-silence yields, and it does so
//! deliberately (PR #128, finding G1): when the turn cannot say WHICH ledger row it
//! produced, there is no honest row to write, and writing one anyway would put a
//! wrong `llm_call_id` — or a NULL implying no ledger row was produced — into the
//! audit trail. Refusing is the smaller lie.
//!
//! **One provider call, and every deviation is terminal** (grill L3). Zero tool
//! calls, several tool calls, unparseable arguments, an empty hypothesis, a
//! mutation that does not apply, a timeout, an oversized context — each ends the
//! turn as its own recorded reason. There are **no retries and no nudges**: unlike
//! the composer, which nudges because a half-built strategy is worth salvaging, a
//! coach turn either produced a proposal or did not, and re-asking is a human
//! gesture that costs nothing. Retrying a hidden-reasoning model silently is how
//! token spend disappears (#124).
//!
//! **The coach reads; it never recomputes.** It is handed a persisted
//! [`PersistedRun`] and its trades and projects them into the bounded
//! [`CoachContext`]; no backtest number is recalculated here, and no order-placing
//! path exists in this codebase for it to reach (ADR-0016).
//!
//! **Least privilege.** The only things that reach the model are the resolved
//! system prompt and [`CoachContext::render`] — no trade log, no equity curve, no
//! run config header, no credential. The provider handed in is the redacting
//! decorator, so the persisted copy of prompt and completion is scrubbed at rest.

use std::time::{Duration, Instant};

use crate::domain::strategy::StrategyVersion;
use crate::domain::strategy::VersionId;
use crate::domain::{
    BacktestRunId, CoachContext, CoachFailure, CoachingRepository, CoachingSession,
    CoachingSessionId, DataError, Disposition, Hypothesis, LlmCallId, LlmConfig, LlmError,
    LlmProvider, Message, Mutation, PersistedRun, Proposal, SessionOutcome, ToolDefinition, Trade,
    apply,
};

use crate::adapters::llm::redacting_logging::Redactor;

use super::composer::LlmCallCapture;
use super::tools::{PROPOSE_MUTATION_TOOL, ProposeMutationArgs, coach_tool_definitions};

/// A turn that produced no record — the faults that are not coaching outcomes.
///
/// Deliberately NOT [`CoachFailure`]s. The seven failure variants are the ways a
/// *coach turn* can deviate, and each of them is written to the audit trail. What
/// is left here is what a session row would have to lie about, in three
/// categories: inputs that do not belong together, a fault raised inside this
/// process on the call path (an unpriced model, a failed ledger write, a ledger
/// correlation that cannot be established), and a failure to write the row at all.
/// Every one of them surfaces at the CLI edge, preserved (ADR-0017).
#[derive(Debug, thiserror::Error)]
pub enum CoachTurnError {
    /// The `run` and the `version` handed in do not belong together — `version` is
    /// not the version `run` was produced against.
    ///
    /// A CALLER fault, deliberately on this side of the line (PR #128, finding F3).
    /// The coach would otherwise prompt on one version's DSL about another
    /// version's result and then persist a session whose two foreign keys are each
    /// individually valid and jointly false: an audit row asserting a coaching turn
    /// that related a run to a version it never touched. `run_coach_with` loads the
    /// version FROM the run and cannot build the pair, but [`Coach`] is exported, so
    /// a direct caller can — which is why the check lives in the turn rather than at
    /// the CLI edge.
    #[error(
        "backtest run `{}` was produced against strategy version `{}`, not `{}`",
        .run.as_str(), .run_version.as_str(), .offered.as_str()
    )]
    RunVersionMismatch {
        /// The run whose ownership the caller contradicted.
        run: BacktestRunId,
        /// The version that run names.
        run_version: VersionId,
        /// The version the caller offered instead.
        offered: VersionId,
    },
    /// The turn never happened because THIS process faulted on the provider call
    /// path — the decorator's ledger insert failed, the configured model has no
    /// price-table entry, the clock is out of range.
    ///
    /// A true *transport* fault is NOT here: it is recorded as
    /// [`CoachFailure::TransportFailure`] (r1.s2.w4) and the CLI still exits
    /// non-zero for it. The split is what keeps "the coach's provider call failed"
    /// from being written into the audit trail for a fault that never left this
    /// binary (PR #128, finding 5).
    #[error("the coach turn could not run: {0}")]
    LocalFault(#[from] LlmError),
    /// A response came back and NO ledger id appeared for this turn.
    ///
    /// `llm_call_id = NULL` says no ledger row was correlated to the turn, and on a
    /// turn that got a usable response that is a false record: the call happened, was
    /// billed, and minted a row this session then fails to name. (NULL is honest on
    /// the pre-call and timeout/transport paths, where there is no row to name.) It
    /// means the provider is not the capturing ledger decorator, or
    /// the capture handle is not the one that decorator writes through —
    /// [`Coach::new`] takes the two independently, so the pairing is a caller's
    /// obligation and is checked here rather than assumed (PR #128, finding G1).
    #[error(
        "the coach turn reached the provider but captured no ledger row: the provider or the \
         capture handle is not the one the ledger decorator writes through"
    )]
    LedgerRowMissing,
    /// Several ledger ids appeared for one turn.
    ///
    /// One turn is one call is one row. Several ids mean the buffer is shared with
    /// another turn or the decorator wrote more than once, and no choice among them
    /// is honest: taking the newest — what this code did before PR #128 (finding
    /// G1) — can name a different turn's row in the audit trail. Detected and
    /// refused instead of resolved.
    #[error("the coach turn captured {seen} ledger rows; one turn is one call is one row")]
    LedgerRowsAmbiguous {
        /// How many ids appeared since the pre-call snapshot.
        seen: usize,
    },
    /// The proposal could not be recorded. Fatal by design: an unrecordable turn is
    /// the silence this spine exists to prevent, so it is surfaced rather than
    /// swallowed.
    #[error("the coach turn could not be recorded: {source}")]
    Record {
        /// Why the write failed.
        #[source]
        source: DataError,
    },
    /// The turn deviated AND the deviation could not be recorded — the double
    /// fault.
    ///
    /// Both halves travel together on purpose (PR #128, finding 6). Reporting only
    /// the write error leaves the operator with "the session could not be written"
    /// and no way to learn what the turn actually did — which, on the paths that
    /// reach here after a timeout or a transport fault, is the entire content of
    /// the incident.
    #[error("the coach turn failed ({failure}) and the failure could not be recorded: {source}")]
    RecordFailed {
        /// What the turn actually did — the reason that never reached the row.
        ///
        /// Boxed to keep this error small: it rides in the `Err` of every
        /// `run_turn`, and a `CoachFailure` inlined here (it carries a `Mutation`
        /// and a `MutationError`) makes every `Ok` pay for the rarest failure.
        failure: Box<CoachFailure>,
        /// Why it could not be written.
        #[source]
        source: DataError,
    },
}

/// The coach: drives one provider turn over the `propose_mutation` tool and
/// records the outcome.
///
/// Consumed generically (`<P: LlmProvider>`, never `dyn`) — the established port
/// style. `provider` is expected to be the redacting + cost-logging decorator, so
/// the ledger row (with its cost and `prompt_version`) is written as a side effect
/// of the call and its id arrives in `captured`.
pub struct Coach<P: LlmProvider> {
    provider: P,
    prompt: String,
    config: LlmConfig,
    tools: Vec<ToolDefinition>,
    turn_timeout: Duration,
    max_dsl_bytes: usize,
    redactor: Redactor,
    captured: LlmCallCapture,
}

impl<P: LlmProvider> Coach<P> {
    /// The per-turn wall-clock guard (audit C5 — the composer's NFR-1 mechanism
    /// and value, reused rather than re-invented).
    pub const DEFAULT_TURN_TIMEOUT: Duration = Duration::from_secs(120);

    /// The default budget for the one variable-length CONTEXT field, the DSL.
    ///
    /// Every other `CoachContext` field is fixed-size, so this single number is
    /// what turns "will the *context* fit?" into a pre-call checkable condition
    /// (grill L4). 32 KiB is far above any strategy the r1 grammar can express —
    /// the canonical fixture is well under 1 KiB — so in practice it fires only on
    /// a pathological document, which is exactly when the coach should refuse
    /// before spending a call.
    ///
    /// It is a SUB-budget, and the question it answers is deliberately the smaller
    /// one: the resolved system prompt is not a `CoachContext` field, so this
    /// number cannot see it. The whole turn is bounded by
    /// [`Self::DEFAULT_MAX_TURN_BYTES`].
    pub const DEFAULT_MAX_DSL_BYTES: usize = 32 * 1024;

    /// The budget for the WHOLE turn — every deterministic byte this process
    /// decides to send, not just the one context field.
    ///
    /// The other operator-owned input is the resolved system prompt, which
    /// `$PULSE_PROMPT_DIR/coach.md` owns and can make arbitrarily large. Before
    /// PR #128 (finding C1) the pre-call check measured a part and let the whole
    /// through: an oversized overlay reached the provider instead of being recorded
    /// as [`CoachFailure::ContextOverflow`]. Twice the DSL sub-budget, so that
    /// sub-budget stays the binding constraint on a strategy document and this
    /// ceiling fires only on what the sub-budget cannot see.
    ///
    /// **Bytes, not tokens.** A conservative LOCAL POLICY proxy: the serialized
    /// size of the exact [`Message`] and [`ToolDefinition`] values handed to
    /// [`LlmProvider::chat`], which counts the role tags, field names, delimiters
    /// and tool schemas that travel with the text. It is deliberately NOT the
    /// provider's token count and NOT the `PulseHive` wire envelope the adapter
    /// builds from these values (ADR-0012 keeps that shape on the far side of the
    /// port). It exists to refuse the pathological turn before it costs a call, not
    /// to predict a context window.
    ///
    /// A fixed policy ceiling rather than a per-turn knob: nothing needs to tune it,
    /// and the tunable budget is the sub-budget above.
    pub const DEFAULT_MAX_TURN_BYTES: usize = 64 * 1024;

    /// Build a coach over `provider`, framed by the resolved `prompt`.
    ///
    /// `captured` is the shared buffer the capturing ledger repo pushes each
    /// minted `LlmCallId` into; the coach reads it back to name the turn's ledger
    /// row on the session (the composer's provenance mechanism).
    #[must_use]
    pub fn new(provider: P, prompt: String, config: LlmConfig, captured: LlmCallCapture) -> Self {
        Self {
            provider,
            prompt,
            config,
            tools: coach_tool_definitions(),
            turn_timeout: Self::DEFAULT_TURN_TIMEOUT,
            max_dsl_bytes: Self::DEFAULT_MAX_DSL_BYTES,
            // Structural api-key-shaped stripping only until a composition root
            // tags the live key (see `with_redactor`).
            redactor: Redactor::default(),
            captured,
        }
    }

    /// Override the per-turn wall-clock guard (audit C5).
    #[must_use]
    pub fn with_turn_timeout(mut self, turn_timeout: Duration) -> Self {
        self.turn_timeout = turn_timeout;
        self
    }

    /// Override the pre-call DSL size budget.
    #[must_use]
    pub fn with_max_dsl_bytes(mut self, max_dsl_bytes: usize) -> Self {
        self.max_dsl_bytes = max_dsl_bytes;
        self
    }

    /// Supply the redactor used to scrub the model's TOOL ARGUMENTS before they
    /// become stored domain values (the no-secret-in-log control).
    ///
    /// This is not the decorator's job and cannot be: the decorator scrubs the
    /// prompt and completion it persists to the `LlmCall` ledger, but a tool
    /// argument travels a different road — it becomes the proposal's `hypothesis`
    /// and the recorded failure's `detail`, which land in `coaching_proposals` and
    /// `coaching_sessions`. A model that echoes a credential into its hypothesis
    /// would otherwise write it, unscrubbed, into the audit trail.
    ///
    /// The composition root passes the SAME `Redactor` it gives the decorator, so
    /// both roads are scrubbed against the same tagged secrets. (`Redactor` is an
    /// adapter value type rather than a domain one; the coach holds it as data and
    /// still speaks only to the `LlmProvider` PORT for anything I/O-shaped.)
    #[must_use]
    pub fn with_redactor(mut self, redactor: Redactor) -> Self {
        self.redactor = redactor;
        self
    }

    /// Run one coach turn against `run` / `version` and record it.
    ///
    /// Returns the session it persisted. The only `Err`s are a local fault on the
    /// call path (the turn never happened) and a persistence failure — see
    /// [`CoachTurnError`]. Every *coaching* outcome, success or deviation — a
    /// transport fault included — is an `Ok` carrying its recorded session.
    ///
    /// # Exclusivity
    ///
    /// Turns on ONE coach are serialized by the receiver, not by a lock: the
    /// capture buffer is append-only and shared with the ledger decorator, so the
    /// snapshot → call → id-extraction sequence must not interleave with another
    /// turn's, or a session names a ledger row it did not produce. `&mut self`
    /// makes the interleaving unwritable rather than unlikely — the borrow checker
    /// rejects a second turn while the first is alive:
    ///
    /// ```compile_fail
    /// # use pulse::{
    /// #     Coach, CoachingRepository, CoachingSessionId, LlmProvider, PersistedRun,
    /// #     StrategyVersion, Trade,
    /// # };
    /// # async fn overlapping_turns<P: LlmProvider, R: CoachingRepository>(
    /// #     coach: &mut Coach<P>,
    /// #     sessions: &R,
    /// #     first_id: CoachingSessionId,
    /// #     second_id: CoachingSessionId,
    /// #     run: &PersistedRun,
    /// #     trades: &[Trade],
    /// #     version: &StrategyVersion,
    /// # ) {
    /// let first = coach.run_turn(sessions, first_id, run, trades, version);
    /// let second = coach.run_turn(sessions, second_id, run, trades, version);
    /// let _ = (first.await, second.await);
    /// # }
    /// ```
    ///
    /// The buffer itself is a WIRING obligation rather than something this type can
    /// enforce: the production composition root mints one per invocation and hands
    /// it to exactly one capturing repo and one coach (`src/cli/coach.rs`), and a
    /// direct [`Coach::new`] caller owes the same. What the turn can do is refuse to
    /// guess — zero or several ids for one turn fail closed
    /// ([`CoachTurnError::LedgerRowMissing`],
    /// [`CoachTurnError::LedgerRowsAmbiguous`]). That is detection, not proof of
    /// origin: a shared buffer in which only one of the two providers captures an id
    /// still yields exactly one, and nothing here can tell whose it is.
    ///
    /// # Errors
    ///
    /// [`CoachTurnError::RunVersionMismatch`] if `version` is not the version `run`
    /// was produced against; [`CoachTurnError::LocalFault`] if this process faulted
    /// on the call path; [`CoachTurnError::LedgerRowMissing`] /
    /// [`CoachTurnError::LedgerRowsAmbiguous`] if the turn cannot name exactly the
    /// ledger row it produced; [`CoachTurnError::Record`] /
    /// [`CoachTurnError::RecordFailed`] if the session could not be written.
    pub async fn run_turn<R: CoachingRepository>(
        &mut self,
        sessions: &R,
        session_id: CoachingSessionId,
        run: &PersistedRun,
        trades: &[Trade],
        version: &StrategyVersion,
    ) -> Result<CoachingSession, CoachTurnError> {
        // 0. Ownership, before the projection, the budgets, the call and any write
        // (PR #128, finding F3). A run coached against someone else's version is a
        // caller mistake, and the session it would leave behind is worse than no
        // session: individually valid FKs asserting a relationship that never
        // existed.
        if run.strategy_version_id != version.id {
            return Err(CoachTurnError::RunVersionMismatch {
                run: run.id.clone(),
                run_version: run.strategy_version_id.clone(),
                offered: version.id.clone(),
            });
        }

        // 1. The bounded projection, refused pre-call when the DSL does not fit.
        let context = match CoachContext::build(run, trades, &version.dsl, self.max_dsl_bytes) {
            Ok(context) => context,
            Err(failure) => {
                // Pre-call: no provider call was made, so no ledger row exists and
                // `llm_call_id` is NULL (audit C3).
                return self
                    .record(sessions, session_id, run, version, None, failure)
                    .await;
            }
        };

        // 2. Exactly two messages: the resolved system prompt and the projection.
        let messages = vec![
            Message::system(self.prompt.clone()),
            Message::user(context.render()),
        ];

        // 2b. The WHOLE-TURN budget (PR #128, finding C1). `max_dsl_bytes` bounds
        // the projection's one variable-length field and cannot see the resolved
        // system prompt, so the two refusals live in the same place for the same
        // reason: pre-call, recorded with `llm_call_id = None` (audit C3), free.
        if let Err(failure) = Self::check_turn_budget(&messages, &self.tools) {
            return self
                .record(sessions, session_id, run, version, None, failure)
                .await;
        }

        // 3. ONE call, under the wall-clock guard.
        let start = self.captured_len();
        let started = Instant::now();
        let response = match tokio::time::timeout(
            self.turn_timeout,
            self.provider.chat(messages, &self.tools, &self.config),
        )
        .await
        {
            Err(_elapsed) => {
                // The MEASURED wait, not the configured budget (PR #128, finding
                // 10). They are close but not equal, and the recorded number is
                // read as evidence of what happened — "the provider did not answer
                // within 120000 ms" restating the setting tells an auditor nothing
                // the config file did not already say.
                let failure = CoachFailure::ProviderTimeout {
                    elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                };
                // A timed-out call may still have produced a ledger row if the
                // decorator got far enough; name it if so. ZERO is legitimate here
                // and does NOT mean no attempt was made — the call went out and the
                // answer did not come back inside the guard. SEVERAL is legitimate
                // nowhere.
                let call_id = self.captured_at_most_one(start)?;
                return self
                    .record(sessions, session_id, run, version, call_id, failure)
                    .await;
            }
            Ok(Err(LlmError::Provider(detail))) => {
                // r1.s2.w4: a TRANSPORT fault is a RECORDED outcome, not an early
                // return. The error text is scrubbed on the way in — an error body
                // can echo the request that produced it, the same road hazard
                // `classify()` handles for tool arguments.
                let failure = CoachFailure::TransportFailure {
                    detail: self.redactor.redact(&detail),
                };
                // A transport fault may still have minted a ledger row if the
                // decorator wrote one before failing; name it if so rather than
                // asserting NULL. Zero is the common case here — no usable exchange,
                // no priced row — and it records an ATTEMPT that happened, which is
                // why `llm_call_id = NULL` cannot be read as "no call was made".
                let call_id = self.captured_at_most_one(start)?;
                let recorded = self
                    .record(sessions, session_id, run, version, call_id, failure)
                    .await?;
                // Recorded AND loud: the CLI still preserves the provider error at
                // the edge (ADR-0017); the session is what makes it non-silent.
                return Ok(recorded);
            }
            Ok(Err(local)) => {
                // NOT a coaching outcome (PR #128, finding 5). An unpriced model or
                // a failed ledger insert is this process faulting, and recording it
                // as `TransportFailure` would write "the coach's provider call
                // failed" into the audit trail for something the provider never
                // did. Surfaced at the edge instead, with nothing persisted.
                return Err(CoachTurnError::LocalFault(local));
            }
            Ok(Ok(response)) => response,
        };
        // A turn that REACHED the provider names exactly one ledger row, or it is a
        // wiring fault and not a coaching outcome (PR #128, finding G1).
        let call_id = Some(self.captured_exactly_one(start)?);

        // 4. Route the one response. No retries, no nudges (grill L3).
        let outcome = classify(&self.redactor, &version.dsl, response.tool_calls);
        match outcome {
            Ok(proposal) => self
                .persist(
                    sessions,
                    session_id,
                    run,
                    version,
                    call_id,
                    SessionOutcome::Proposed { proposal },
                )
                .await
                .map_err(|source| CoachTurnError::Record { source }),
            Err(failure) => {
                self.record(sessions, session_id, run, version, call_id, failure)
                    .await
            }
        }
    }

    /// The deterministic bytes this turn would send, against
    /// [`Self::DEFAULT_MAX_TURN_BYTES`].
    ///
    /// Measured over a serialization of the EXACT `messages` and `tools` values
    /// handed to [`LlmProvider::chat`], not a hand-sum of the text inside them:
    /// role tags, field names, delimiters and the tool schemas are all content the
    /// turn sends, and a hand-sum silently under-counts the moment either type
    /// gains a field.
    ///
    /// A serialization failure is unreachable — every field is a `String`, a `u32`
    /// or an already-valid `serde_json::Value` — but it is mapped to the same
    /// pre-call refusal rather than unwrapped: a turn whose size cannot be
    /// established is a turn that must not be sent.
    fn check_turn_budget(
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> Result<(), CoachFailure> {
        let messages_bytes = serde_json::to_string(messages)
            .map_err(|e| CoachFailure::ContextOverflow {
                detail: format!("the turn's messages could not be measured: {e}"),
            })?
            .len();
        let tools_bytes = serde_json::to_string(tools)
            .map_err(|e| CoachFailure::ContextOverflow {
                detail: format!("the turn's tool schemas could not be measured: {e}"),
            })?
            .len();

        let total = messages_bytes + tools_bytes;
        let budget = Self::DEFAULT_MAX_TURN_BYTES;
        if total > budget {
            return Err(CoachFailure::ContextOverflow {
                detail: format!(
                    "the turn would send {total} deterministic bytes \
                     ({messages_bytes} of messages, {tools_bytes} of tool schemas) \
                     against a {budget}-byte budget"
                ),
            });
        }
        Ok(())
    }

    /// Persist a failed turn — the never-silence path, used by every deviation.
    ///
    /// On the double fault — the deviation could not be written — the returned
    /// error carries BOTH the write error and the reason that never reached the
    /// row ([`CoachTurnError::RecordFailed`], PR #128 finding 6). That reason is
    /// otherwise unrecoverable: it exists only in this frame.
    async fn record<R: CoachingRepository>(
        &self,
        sessions: &R,
        session_id: CoachingSessionId,
        run: &PersistedRun,
        version: &StrategyVersion,
        llm_call_id: Option<LlmCallId>,
        failure: CoachFailure,
    ) -> Result<CoachingSession, CoachTurnError> {
        let reason = failure.clone();
        self.persist(
            sessions,
            session_id,
            run,
            version,
            llm_call_id,
            SessionOutcome::Failed { failure },
        )
        .await
        .map_err(|source| CoachTurnError::RecordFailed {
            failure: Box::new(reason),
            source,
        })
    }

    /// Write the turn and return **what was recorded**, not what was submitted.
    ///
    /// The read-back is not ceremony: the repo's injected clock owns `created_at`
    /// (#82), so the value this function constructs is never the value that lands.
    /// Returning the submitted struct would hand every caller — the CLI's printout,
    /// `r1.s4`'s rail — a session that disagrees with the audit trail in exactly
    /// the field an auditor reads first.
    async fn persist<R: CoachingRepository>(
        &self,
        sessions: &R,
        session_id: CoachingSessionId,
        run: &PersistedRun,
        version: &StrategyVersion,
        llm_call_id: Option<LlmCallId>,
        outcome: SessionOutcome,
    ) -> Result<CoachingSession, DataError> {
        let session = CoachingSession {
            id: session_id.clone(),
            backtest_run_id: run.id.clone(),
            strategy_version_id: version.id.clone(),
            // Overwritten by the repo's clock on write; see the read-back below.
            created_at: String::new(),
            llm_call_id,
            outcome,
        };
        sessions.save_session(&session).await?;
        sessions.get_session(&session_id).await?.ok_or_else(|| {
            DataError::Db(format!(
                "coaching session `{}` was written but did not read back",
                session_id.as_str()
            ))
        })
    }

    /// The current length of the capture buffer (the pre-call snapshot point).
    fn captured_len(&self) -> usize {
        self.captured
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    /// The ids that appeared since the pre-call snapshot, in order.
    fn ids_since(&self, start: usize) -> Vec<LlmCallId> {
        let guard = self
            .captured
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.get(start..).unwrap_or_default().to_vec()
    }

    /// The one ledger row a SUCCESSFUL turn must have minted.
    ///
    /// Exactly one: the call happened, so a row exists, and the session names it.
    fn captured_exactly_one(&self, start: usize) -> Result<LlmCallId, CoachTurnError> {
        let mut ids = self.ids_since(start);
        match ids.len() {
            1 => Ok(ids.remove(0)),
            0 => Err(CoachTurnError::LedgerRowMissing),
            seen => Err(CoachTurnError::LedgerRowsAmbiguous { seen }),
        }
    }

    /// The ledger row a DEVIANT turn may or may not have minted.
    ///
    /// Zero is legitimate here and only here: a timeout can strike before the
    /// decorator writes, and a transport fault produces no usable exchange to price
    /// at all. Several is legitimate nowhere.
    fn captured_at_most_one(&self, start: usize) -> Result<Option<LlmCallId>, CoachTurnError> {
        let mut ids = self.ids_since(start);
        match ids.len() {
            0 => Ok(None),
            1 => Ok(Some(ids.remove(0))),
            seen => Err(CoachTurnError::LedgerRowsAmbiguous { seen }),
        }
    }
}

/// Turn one model response's tool calls into a proposal or a typed failure.
///
/// A free function, so the single-call contract and the failure taxonomy are
/// testable without a provider, and so the routing reads as the decision table it
/// is.
fn classify(
    redactor: &Redactor,
    dsl: &crate::domain::StrategyDsl,
    tool_calls: Vec<crate::domain::ToolCall>,
) -> Result<Proposal, CoachFailure> {
    // A3: exactly one call ends the turn. Zero and several are both terminal.
    let call = match tool_calls.len() {
        0 => return Err(CoachFailure::ZeroCalls),
        1 => {
            let mut calls = tool_calls;
            calls.remove(0)
        }
        n => {
            // Count what was actually asked for, not just how many calls arrived
            // (PR #128, finding 7): "two propose_mutation calls" and "one
            // propose_mutation plus one call to a tool the coach does not have"
            // are different mistakes, and the recorded reason has to be able to
            // say which one happened.
            let proposals = tool_calls
                .iter()
                .filter(|c| c.name == PROPOSE_MUTATION_TOOL)
                .count();
            return Err(CoachFailure::SeveralCalls {
                count: u32::try_from(n).unwrap_or(u32::MAX),
                propose_mutation_count: u32::try_from(proposals).unwrap_or(u32::MAX),
            });
        }
    };

    if call.name != PROPOSE_MUTATION_TOOL {
        return Err(CoachFailure::MalformedArguments {
            detail: redactor.redact(&format!(
                "the turn called `{}`; the coach's only tool is `{PROPOSE_MUTATION_TOOL}`",
                call.name
            )),
        });
    }

    // SCRUB BEFORE PARSE. Everything downstream — the hypothesis that becomes a
    // stored domain value, the path, and any serde error text quoting the input —
    // is derived from this value, so scrubbing here covers all of them at once.
    let arguments = redact_json(redactor, call.arguments);

    let args: ProposeMutationArgs =
        serde_json::from_value(arguments).map_err(|source| CoachFailure::MalformedArguments {
            detail: redactor.redact(&format!(
                "could not parse propose_mutation arguments: {source}"
            )),
        })?;

    // An empty hypothesis is a malformed proposal, not a proposal: the capability
    // sentence promises a mutation WITH a stated hypothesis.
    let hypothesis =
        Hypothesis::new(args.hypothesis).map_err(|source| CoachFailure::MalformedArguments {
            detail: format!("propose_mutation: {source}"),
        })?;

    let mutation = Mutation::SetParam {
        path: args.path,
        new_value: args.new_value,
    };

    // The w1 framework decides applicability — validated by `apply()` at use time,
    // never a stored fact (audit C4). The candidate itself is discarded: `r1.s4`
    // re-runs `apply()` at accept.
    match apply(dsl, &mutation) {
        Ok(_candidate) => Ok(Proposal {
            mutation,
            hypothesis,
            disposition: Disposition::Proposed,
        }),
        Err(error) => Err(CoachFailure::InapplicableMutation { mutation, error }),
    }
}

/// Recursively scrub every string leaf of a tool-argument value.
///
/// Numbers are never touched (the VS-1.3.1 rule: a "strip any number" rule nukes
/// the context that makes a proposal readable), and object keys are structural, so
/// only the values a model actually writes prose into are rewritten.
fn redact_json(redactor: &Redactor, value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::String(text) => serde_json::Value::String(redactor.redact(&text)),
        serde_json::Value::Array(items) => serde_json::Value::Array(
            items
                .into_iter()
                .map(|item| redact_json(redactor, item))
                .collect(),
        ),
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.into_iter()
                .map(|(key, val)| (key, redact_json(redactor, val)))
                .collect(),
        ),
        other => other,
    }
}
