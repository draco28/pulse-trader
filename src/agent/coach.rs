//! The coach turn (r1.s2.w3, ADR-0021) — one provider call in, one recorded
//! session out.
//!
//! **Never silence.** Every path through [`Coach::run_turn`] ends by persisting
//! exactly one [`CoachingSession`]: a `Proposed` proposal, or one of the six typed
//! [`CoachFailure`]s. Persistence is inside the turn rather than left to the
//! caller on purpose — a guarantee a caller can forget is not a guarantee.
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

use std::time::Duration;

use crate::domain::strategy::StrategyVersion;
use crate::domain::{
    CoachContext, CoachFailure, CoachingRepository, CoachingSession, CoachingSessionId, DataError,
    Disposition, Hypothesis, LlmCallId, LlmConfig, LlmError, LlmProvider, Message, Mutation,
    PersistedRun, Proposal, SessionOutcome, ToolDefinition, Trade, apply,
};

use crate::adapters::llm::redacting_logging::Redactor;

use super::composer::LlmCallCapture;
use super::tools::{PROPOSE_MUTATION_TOOL, ProposeMutationArgs, coach_tool_definitions};

/// A turn that never happened: the provider transport failed before any coaching
/// outcome existed to record.
///
/// Deliberately NOT a [`CoachFailure`]. The six failure variants are the ways a
/// *coach turn* can deviate; an HTTP error is an infrastructure fault, and writing
/// it into the audit trail as (say) a timeout would put a false reason in the one
/// record `r1.s4` and the operator have to trust. It surfaces at the CLI edge
/// instead, preserved (ADR-0017).
#[derive(Debug, thiserror::Error)]
pub enum CoachTurnError {
    /// The provider transport failed — the turn did not happen.
    #[error("the coach's provider call failed before the turn began: {0}")]
    Provider(#[from] LlmError),
    /// The session could not be recorded. Fatal by design: an unrecordable turn is
    /// the silence this spine exists to prevent, so it is surfaced rather than
    /// swallowed.
    #[error("the coach turn could not be recorded: {0}")]
    Record(#[from] DataError),
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

    /// The default budget for the one variable-length context field, the DSL.
    ///
    /// Every other `CoachContext` field is fixed-size, so this single number is
    /// what turns "will the context fit?" into a pre-call checkable condition
    /// (grill L4). 32 KiB is far above any strategy the r1 grammar can express —
    /// the canonical fixture is well under 1 KiB — so in practice it fires only on
    /// a pathological document, which is exactly when the coach should refuse
    /// before spending a call.
    pub const DEFAULT_MAX_DSL_BYTES: usize = 32 * 1024;

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
    /// Returns the session it persisted. The only `Err`s are a provider transport
    /// failure (the turn never happened) and a persistence failure — see
    /// [`CoachTurnError`]. Every *coaching* outcome, success or deviation, is an
    /// `Ok` carrying its recorded session.
    ///
    /// # Errors
    ///
    /// [`CoachTurnError::Provider`] if the transport failed;
    /// [`CoachTurnError::Record`] if the session could not be written.
    pub async fn run_turn<R: CoachingRepository>(
        &self,
        sessions: &R,
        session_id: CoachingSessionId,
        run: &PersistedRun,
        trades: &[Trade],
        version: &StrategyVersion,
    ) -> Result<CoachingSession, CoachTurnError> {
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

        // 3. ONE call, under the wall-clock guard.
        let start = self.captured_len();
        let response = match tokio::time::timeout(
            self.turn_timeout,
            self.provider.chat(messages, &self.tools, &self.config),
        )
        .await
        {
            Err(_elapsed) => {
                let failure = CoachFailure::ProviderTimeout {
                    elapsed_ms: u64::try_from(self.turn_timeout.as_millis()).unwrap_or(u64::MAX),
                };
                // A timed-out call may still have produced a ledger row if the
                // decorator got far enough; name it if so.
                let call_id = self.captured_since(start);
                return self
                    .record(sessions, session_id, run, version, call_id, failure)
                    .await;
            }
            Ok(Err(source)) => return Err(CoachTurnError::Provider(source)),
            Ok(Ok(response)) => response,
        };
        let call_id = self.captured_since(start);

        // 4. Route the one response. No retries, no nudges (grill L3).
        let outcome = classify(&self.redactor, &version.dsl, response.tool_calls);
        match outcome {
            Ok(proposal) => {
                self.persist(
                    sessions,
                    session_id,
                    run,
                    version,
                    call_id,
                    SessionOutcome::Proposed { proposal },
                )
                .await
            }
            Err(failure) => {
                self.record(sessions, session_id, run, version, call_id, failure)
                    .await
            }
        }
    }

    /// Persist a failed turn — the never-silence path, used by every deviation.
    async fn record<R: CoachingRepository>(
        &self,
        sessions: &R,
        session_id: CoachingSessionId,
        run: &PersistedRun,
        version: &StrategyVersion,
        llm_call_id: Option<LlmCallId>,
        failure: CoachFailure,
    ) -> Result<CoachingSession, CoachTurnError> {
        self.persist(
            sessions,
            session_id,
            run,
            version,
            llm_call_id,
            SessionOutcome::Failed { failure },
        )
        .await
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
    ) -> Result<CoachingSession, CoachTurnError> {
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
            CoachTurnError::Record(DataError::Db(format!(
                "coaching session `{}` was written but did not read back",
                session_id.as_str()
            )))
        })
    }

    /// The current length of the capture buffer (the pre-call snapshot point).
    fn captured_len(&self) -> usize {
        self.captured
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    /// The id the decorator minted for this turn, if it minted one.
    fn captured_since(&self, start: usize) -> Option<LlmCallId> {
        let guard = self
            .captured
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.get(start..).and_then(|ids| ids.last().cloned())
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
            return Err(CoachFailure::SeveralCalls {
                count: u32::try_from(n).unwrap_or(u32::MAX),
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
