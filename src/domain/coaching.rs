//! The coaching session domain (r1.s2.w2, ADR-0021) — pure, zero-I/O value types.
//!
//! **Never silence.** A coach turn produces exactly one of two things: a
//! [`Proposal`] (one typed [`Mutation`] plus a stated hypothesis) or a typed
//! [`CoachFailure`]. That is a type-level property here, not a convention:
//! [`SessionOutcome`] is an enum, so a session carrying both or neither is not
//! representable. The session row is the audit trail (audit C3) — every turn
//! outcome persists, and `llm_call_id` is `None` precisely when no provider call
//! was made (a pre-call failure such as [`CoachFailure::ContextOverflow`]).
//!
//! **Validity is use-time, never stored (audit C4).** There is deliberately no
//! `validated` field on [`Proposal`] and no such column in migration `0005`.
//! Whether a stored mutation still applies is answered by calling
//! [`apply`](crate::domain::dsl::apply) at the moment of use — `r1.s4`'s
//! modify-then-accept path re-runs it after the trader's edit.
//!
//! **The disposition state machine.** `Proposed → Accepted | Rejected | Modified`,
//! with [`Disposition::Accepted`] carrying the child version id **as its payload**
//! rather than as a nullable field, so "a rejected proposal with a child version"
//! cannot be constructed. `Accepted` and `Rejected` are terminal; `Modified` is a
//! working state (`r1.s4` edits, then accepts); nothing returns to `Proposed`.
//! `w2` constructs only `Proposed` — the rest is dormant until `r1.s4`, which
//! exercises it without a second migration (grill L2).
//!
//! **The accept idempotency key is the session id.** `r1.s4`'s consistency model
//! keys one child version per proposal by session id, and the schema enforces at
//! most one proposal per session (`coaching_proposals.session_id UNIQUE`).

use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;

use super::backtest::BacktestRunId;
use super::dsl::{Mutation, MutationError};
use super::llm_call::LlmCallId;
use super::strategy::VersionId;

/// Identifier of a [`CoachingSession`] — a `#[serde(transparent)]` `String`
/// newtype, matching [`LlmCallId`] and
/// [`StrategyId`](crate::domain::strategy::StrategyId): an opaque adapter-minted
/// string serialized as a bare JSON string (matching the `TEXT` primary key).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CoachingSessionId(String);

impl CoachingSessionId {
    /// Wrap a raw (adapter-generated) id string.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Borrow the underlying id string (for SQL binding / map keys).
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Everything that can go wrong in the coaching domain itself (as distinct from
/// [`MutationError`], which is the DSL's).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CoachingError {
    /// A hypothesis was empty or whitespace-only. The capability sentence promises
    /// a mutation **with a stated hypothesis**; an empty string is silence wearing
    /// a proposal's clothes.
    #[error("a proposal's hypothesis must not be empty or whitespace-only")]
    EmptyHypothesis,
    /// A disposition transition the state machine does not allow.
    #[error("illegal disposition transition {from} -> {to}")]
    IllegalTransition {
        /// The state transitioned from.
        from: DispositionKind,
        /// The state transitioned to.
        to: DispositionKind,
    },
}

/// A proposal's stated hypothesis — guaranteed non-empty after trimming.
///
/// Constructible only through [`Hypothesis::new`], and `#[serde(try_from)]` so the
/// invariant survives the **read** path too: a hand-edited row or a proposal
/// written by an older binary cannot smuggle an empty hypothesis past the
/// constructor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Hypothesis(String);

impl Hypothesis {
    /// Build a hypothesis, trimming surrounding whitespace.
    ///
    /// # Errors
    ///
    /// Returns [`CoachingError::EmptyHypothesis`] when the text is empty or
    /// whitespace-only.
    pub fn new(text: impl Into<String>) -> Result<Self, CoachingError> {
        let text = text.into();
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Err(CoachingError::EmptyHypothesis);
        }
        Ok(Self(trimmed.to_owned()))
    }

    /// Borrow the hypothesis text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Hypothesis {
    type Error = CoachingError;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::new(text)
    }
}

impl From<Hypothesis> for String {
    fn from(hypothesis: Hypothesis) -> Self {
        hypothesis.0
    }
}

/// The disposition state, without its payload — the tag a column stores, an error
/// names, and a `CHECK` constraint enumerates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DispositionKind {
    /// The coach proposed it; nobody has acted yet.
    Proposed,
    /// Accepted — `r1.s4` mints the child version.
    Accepted,
    /// Rejected — recorded, no version.
    Rejected,
    /// The trader edited the proposed mutation's parameters.
    Modified,
}

impl fmt::Display for DispositionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Proposed => "proposed",
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
            Self::Modified => "modified",
        })
    }
}

/// What has become of a [`Proposal`].
///
/// [`Disposition::Accepted`] carries the child version id as its **payload**, so
/// `child_version_id` exists only on the state that can have one — the reason the
/// `0005` column is nullable but the domain has no nullable field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Disposition {
    /// The initial state — the only one `w2` ever constructs.
    Proposed,
    /// Accepted, naming the child `StrategyVersion` `r1.s4` minted (ADR-0010:
    /// this crate creates no child version in `r1.s2`).
    Accepted {
        /// The child version the accept produced.
        child_version_id: VersionId,
    },
    /// Rejected and recorded; no version was created.
    Rejected,
    /// The trader edited the proposed mutation's parameters; still open.
    Modified,
}

impl Disposition {
    /// This disposition's tag, without its payload.
    #[must_use]
    pub fn kind(&self) -> DispositionKind {
        match self {
            Self::Proposed => DispositionKind::Proposed,
            Self::Accepted { .. } => DispositionKind::Accepted,
            Self::Rejected => DispositionKind::Rejected,
            Self::Modified => DispositionKind::Modified,
        }
    }

    /// The child version this disposition names — `Some` only for
    /// [`Disposition::Accepted`].
    #[must_use]
    pub fn child_version_id(&self) -> Option<&VersionId> {
        match self {
            Self::Accepted { child_version_id } => Some(child_version_id),
            Self::Proposed | Self::Rejected | Self::Modified => None,
        }
    }
}

/// The coach's single proposal for a turn: one typed [`Mutation`], the hypothesis
/// it rests on, and its disposition.
///
/// No `validated` field (audit C4) — applicability is re-established by
/// [`apply`](crate::domain::dsl::apply) at use time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Proposal {
    /// The mutation the coach proposes, stored typed so `r1.s4`'s modify path can
    /// edit its parameters and re-apply it.
    pub mutation: Mutation,
    /// Why the coach believes this mutation helps — never empty.
    pub hypothesis: Hypothesis,
    /// Where the proposal stands.
    pub disposition: Disposition,
}

impl Proposal {
    /// Move this proposal to `next`, returning the transitioned copy.
    ///
    /// The state machine: `Accepted` and `Rejected` are terminal; `Modified` is a
    /// working state that may still be accepted or rejected; nothing may return to
    /// `Proposed`.
    ///
    /// # Errors
    ///
    /// Returns [`CoachingError::IllegalTransition`] when the move is not allowed.
    pub fn transition(&self, next: Disposition) -> Result<Self, CoachingError> {
        let from = self.disposition.kind();
        let to = next.kind();

        let legal = match (from, to) {
            // `Proposed` is an initial state and nothing returns to it; and a
            // settled proposal — accepted or rejected — is settled.
            (_, DispositionKind::Proposed)
            | (DispositionKind::Accepted | DispositionKind::Rejected, _) => false,
            // From `Proposed` or the working state `Modified`, the rest is open.
            (DispositionKind::Proposed | DispositionKind::Modified, _) => true,
        };
        if !legal {
            return Err(CoachingError::IllegalTransition { from, to });
        }

        Ok(Self {
            disposition: next,
            ..self.clone()
        })
    }
}

/// The typed failure taxonomy for a coach turn (grill L3).
///
/// One variant per deviation the capability sentence enumerates. Every one
/// `Display`s with its context, because each has to read back later as a recorded
/// failure reason rather than as "something went wrong". **No provider call is
/// retried** — a deviation is terminal for the turn, and re-asking is a human
/// gesture (ADR-0021 decision 6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Error)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CoachFailure {
    /// The model answered without calling `propose_mutation` at all.
    #[error("the coach turn ended with no propose_mutation call")]
    ZeroCalls,
    /// The model called `propose_mutation` more than once; the first well-formed
    /// call ends a turn.
    #[error("the coach turn made {count} propose_mutation calls; exactly one ends a turn")]
    SeveralCalls {
        /// How many calls the turn made.
        count: u32,
    },
    /// The tool call arrived but its arguments did not parse into a [`Mutation`].
    #[error("the coach's propose_mutation arguments were malformed: {detail}")]
    MalformedArguments {
        /// What was wrong with the arguments.
        detail: String,
    },
    /// The proposed mutation does not apply to the version's DSL — the w1
    /// [`MutationError`] is carried **verbatim**, not flattened to a string, so the
    /// typed reason survives into the record and back out of it.
    #[error("the proposed mutation does not apply: {error}")]
    InapplicableMutation {
        /// What the coach proposed.
        mutation: Mutation,
        /// Why it does not apply.
        #[source]
        error: MutationError,
    },
    /// The provider did not answer inside the turn's wall-clock budget.
    #[error("the provider did not answer within {elapsed_ms} ms")]
    ProviderTimeout {
        /// How long the turn waited.
        elapsed_ms: u64,
    },
    /// The bounded `CoachContext` did not fit the model's window — a **pre-call**
    /// failure, so the session records `llm_call_id = None` (audit C3).
    #[error("the coach context does not fit the model's window: {detail}")]
    ContextOverflow {
        /// The measured overflow.
        detail: String,
    },
}

/// Exactly one outcome per coach turn — a proposal or a typed failure.
///
/// An enum rather than two `Option` fields: "both" and "neither" are the two
/// states the never-silence guarantee forbids, and neither is representable here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum SessionOutcome {
    /// The turn produced a proposal.
    Proposed {
        /// The coach's single proposal.
        proposal: Proposal,
    },
    /// The turn failed, and the reason is recorded.
    Failed {
        /// Why the turn produced no proposal.
        failure: CoachFailure,
    },
}

/// One coach turn, recorded.
///
/// The session row **is** the audit trail (audit C3): it exists whether the turn
/// succeeded or failed, and `llm_call_id` is `Some` exactly when a provider call
/// was actually made — a pre-call failure records the session with `None`.
///
/// `created_at` is an RFC3339 UTC string minted by the adapter's injected `Clock`,
/// mirroring [`PersistedRun`](crate::domain::PersistedRun) rather than parsing to
/// `DateTime` in the domain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoachingSession {
    /// The session's opaque id — also `r1.s4`'s accept idempotency key.
    pub id: CoachingSessionId,
    /// The persisted backtest run the coach read (and never recomputed).
    pub backtest_run_id: BacktestRunId,
    /// The strategy version whose DSL the proposal mutates.
    pub strategy_version_id: VersionId,
    /// Adapter-minted RFC3339 UTC creation timestamp.
    pub created_at: String,
    /// The provider call this turn made, if any. `None` for a pre-call failure.
    pub llm_call_id: Option<LlmCallId>,
    /// What the turn produced — a proposal or a typed failure, never neither.
    pub outcome: SessionOutcome,
}
