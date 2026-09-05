//! The coach's MODEL-FACING half (r1.s2.w3, ADR-0021; resealed by r1.s4.w1) —
//! the turn's deterministic budgets and the routing of one model response into one
//! outcome.
//!
//! **The turn itself moved.** Until r1.s4.w1 this module also owned the sequence:
//! a `pub` `Coach` a caller built from a provider, a prompt, a config and a capture
//! handle, and a `run_turn` that took a run, a trade vector and a version. That
//! surface admitted six audit rows that are each individually valid and jointly
//! false (`pulseai-labs/pulse-trader#132`) — a session naming a run and a version
//! that never met, a session naming another turn's ledger row, a turn written after
//! the call with no claim before it. It is now
//! [`crate::application::coach::run_coach_turn`], which takes IDENTIFIERS and ports
//! and loads everything else itself, so none of those rows is constructible.
//!
//! What stays here is what is genuinely about talking to a model:
//!
//! - the deterministic **budgets** ([`DEFAULT_MAX_DSL_BYTES`],
//!   [`DEFAULT_MAX_TURN_BYTES`]) and the per-turn wall-clock guard
//!   ([`DEFAULT_TURN_TIMEOUT`]), with [`check_turn_budget`] measuring the exact
//!   values the turn would send;
//! - [`classify`], the decision table that turns one response's tool calls into a
//!   [`Proposal`], a recorded structural-advice answer, or one typed
//!   [`CoachFailure`];
//! - the tool-argument scrubber [`redact_json`], which runs BEFORE parsing so every
//!   value derived from the arguments is scrubbed at once.
//!
//! **One provider call, and every deviation is terminal** (grill L3). Zero tool
//! calls, several tool calls, unparseable arguments, an empty hypothesis, a mutation
//! that does not apply, a timeout, an oversized context — each ends the turn as its
//! own recorded reason. There are **no retries and no nudges**: unlike the composer,
//! which nudges because a half-built strategy is worth salvaging, a coach turn
//! either produced an answer or did not, and re-asking is a human gesture that costs
//! nothing. Retrying a hidden-reasoning model silently is how token spend disappears
//! (#124).
//!
//! **Two mutually exclusive tools** since r1.s4.w1 (#131): `propose_mutation` and
//! `record_inapplicable`. Exactly one call still ends the turn — the second tool
//! widens what a turn can honestly SAY, not how many calls it may make.

use std::time::Duration;

use crate::domain::{
    CoachFailure, Disposition, Hypothesis, Message, Mutation, Proposal, ToolDefinition, apply,
};

use crate::domain::Redactor;

use super::tools::{
    PROPOSE_MUTATION_TOOL, ProposeMutationArgs, RECORD_INAPPLICABLE_TOOL, RecordInapplicableArgs,
};

/// The per-turn wall-clock guard (audit C5 — the composer's NFR-1 mechanism and
/// value, reused rather than re-invented).
pub(crate) const DEFAULT_TURN_TIMEOUT: Duration = Duration::from_secs(120);

/// The default budget for the one variable-length CONTEXT field, the DSL.
///
/// Every other `CoachContext` field is fixed-size, so this single number is what
/// turns "will the *context* fit?" into a pre-call checkable condition (grill L4).
/// 32 KiB is far above any strategy the r1 grammar can express — the canonical
/// fixture is well under 1 KiB — so in practice it fires only on a pathological
/// document, which is exactly when the coach should refuse before spending a call.
///
/// It is a SUB-budget, and the question it answers is deliberately the smaller one:
/// the resolved system prompt is not a `CoachContext` field, so this number cannot
/// see it. The whole turn is bounded by [`DEFAULT_MAX_TURN_BYTES`].
pub(crate) const DEFAULT_MAX_DSL_BYTES: usize = 32 * 1024;

/// The budget for the WHOLE turn — every deterministic byte this process decides to
/// send, not just the one context field.
///
/// The other operator-owned input is the resolved system prompt, which
/// `$PULSE_PROMPT_DIR/coach.md` owns and can make arbitrarily large. Before PR #128
/// (finding C1) the pre-call check measured a part and let the whole through: an
/// oversized overlay reached the provider instead of being recorded as
/// [`CoachFailure::ContextOverflow`]. Twice the DSL sub-budget, so that sub-budget
/// stays the binding constraint on a strategy document and this ceiling fires only
/// on what the sub-budget cannot see.
///
/// **Bytes, not tokens.** A conservative LOCAL POLICY proxy: the serialized size of
/// the exact [`Message`] and [`ToolDefinition`] values handed to the provider, which
/// counts the role tags, field names, delimiters and tool schemas that travel with
/// the text. It is deliberately NOT the provider's token count and NOT the
/// `PulseHive` wire envelope the adapter builds from these values (ADR-0012 keeps
/// that shape on the far side of the port). It exists to refuse the pathological
/// turn before it costs a call, not to predict a context window.
pub(crate) const DEFAULT_MAX_TURN_BYTES: usize = 64 * 1024;

/// The ceiling on each `record_inapplicable` field, in bytes (r1.s4.w1).
///
/// The two fields become STORED DOMAIN VALUES in the audit trail, and the model
/// controls their length. Generous enough for the paragraph the tool asks for and
/// far below anything that would turn the session row into a document store; a
/// longer one is `MalformedArguments`, which is a recorded turn, not silence.
pub(crate) const MAX_INAPPLICABLE_FIELD_BYTES: usize = 2_000;

/// What one well-formed coach turn answered — the two shapes a turn can END in
/// besides a typed failure (r1.s4.w1, #131).
///
/// An enum rather than an `Option<Proposal>` plus a side channel: the two tools are
/// mutually exclusive, and "a proposal AND recorded inapplicability" is the state
/// the honesty protocol exists to forbid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TurnAnswer {
    /// `propose_mutation`: one validated mutation with its hypothesis.
    Proposal(Proposal),
    /// `record_inapplicable`: the advice this release cannot express, recorded
    /// instead of approximated. It becomes
    /// [`CoachFailure::InapplicableAdvice`] — a RECORDED FAILURE, because no
    /// mutation was proposed, not because the coach did anything wrong.
    Inapplicable {
        /// What the coach wanted to change, structurally (scrubbed, bounded).
        intent: String,
        /// Which observed run facts motivated it (scrubbed, bounded).
        evidence: String,
    },
}

/// The deterministic bytes this turn would send, against [`DEFAULT_MAX_TURN_BYTES`].
///
/// Measured over a serialization of the EXACT `messages` and `tools` values handed
/// to the provider, not a hand-sum of the text inside them: role tags, field names,
/// delimiters and the tool schemas are all content the turn sends, and a hand-sum
/// silently under-counts the moment either type gains a field.
///
/// A serialization failure is unreachable — every field is a `String`, a `u32` or an
/// already-valid `serde_json::Value` — but it is mapped to the same pre-call refusal
/// rather than unwrapped: a turn whose size cannot be established is a turn that
/// must not be sent.
///
/// # Errors
///
/// Returns [`CoachFailure::ContextOverflow`] when the turn exceeds the budget or
/// cannot be measured.
pub(crate) fn check_turn_budget(
    messages: &[Message],
    tools: &[ToolDefinition],
    budget: usize,
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

/// Turn one model response's tool calls into an answer or a typed failure.
///
/// A free function, so the single-call contract and the failure taxonomy are
/// testable without a provider, and so the routing reads as the decision table it
/// is.
///
/// # Errors
///
/// Returns the typed [`CoachFailure`] for every deviation: zero calls, several
/// calls, a foreign-named tool, unparseable or unbounded arguments, an empty
/// hypothesis, and a mutation that does not apply.
pub(crate) fn classify(
    redactor: &Redactor,
    dsl: &crate::domain::StrategyDsl,
    tool_calls: Vec<crate::domain::ToolCall>,
) -> Result<TurnAnswer, CoachFailure> {
    // A3: exactly one call ends the turn. Zero and several are both terminal — and
    // that is unchanged by the second tool: `propose_mutation` AND
    // `record_inapplicable` in one response is a model answering twice, which is
    // the several-calls mistake wearing new clothes.
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

    let name = call.name.clone();
    // SCRUB BEFORE PARSE. Everything downstream — the hypothesis that becomes a
    // stored domain value, the recorded intent/evidence, the path, and any serde
    // error text quoting the input — is derived from this value, so scrubbing here
    // covers all of them at once.
    let arguments = redact_json(redactor, call.arguments);

    match name.as_str() {
        PROPOSE_MUTATION_TOOL => classify_proposal(redactor, dsl, arguments),
        RECORD_INAPPLICABLE_TOOL => classify_inapplicable(redactor, arguments),
        other => Err(CoachFailure::MalformedArguments {
            detail: redactor.redact(&format!(
                "the turn called `{other}`; the coach's tools are \
                 `{PROPOSE_MUTATION_TOOL}` and `{RECORD_INAPPLICABLE_TOOL}`"
            )),
        }),
    }
}

/// The `propose_mutation` arm — unchanged behaviour, moved into its own function
/// when the second tool arrived.
fn classify_proposal(
    redactor: &Redactor,
    dsl: &crate::domain::StrategyDsl,
    arguments: serde_json::Value,
) -> Result<TurnAnswer, CoachFailure> {
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
        Ok(_candidate) => Ok(TurnAnswer::Proposal(Proposal {
            mutation,
            hypothesis,
            disposition: Disposition::Proposed,
            // r1.s4.w4: a freshly proposed mutation has no accept attempt behind
            // it. The field records what the LATEST accept did, and nothing has
            // accepted this yet.
            accept_failure: None,
        })),
        Err(error) => Err(CoachFailure::InapplicableMutation { mutation, error }),
    }
}

/// The `record_inapplicable` arm (r1.s4.w1, #131).
///
/// It touches neither `apply()` nor the DSL: the whole point is advice the locator
/// grammar cannot address. What it DOES enforce is that the two fields are real —
/// non-empty after scrubbing and bounded in length — because they are stored domain
/// values, and an empty "intent" is silence wearing a record's clothes exactly as an
/// empty hypothesis is.
fn classify_inapplicable(
    redactor: &Redactor,
    arguments: serde_json::Value,
) -> Result<TurnAnswer, CoachFailure> {
    let args: RecordInapplicableArgs =
        serde_json::from_value(arguments).map_err(|source| CoachFailure::MalformedArguments {
            detail: redactor.redact(&format!(
                "could not parse record_inapplicable arguments: {source}"
            )),
        })?;

    let intent = bounded_field("intent", &args.intent)?;
    let evidence = bounded_field("evidence", &args.evidence)?;
    Ok(TurnAnswer::Inapplicable { intent, evidence })
}

/// Trim, refuse empty, refuse oversized — the two `record_inapplicable` fields'
/// only rule, applied identically to both so neither can be the lax one.
fn bounded_field(name: &str, value: &str) -> Result<String, CoachFailure> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(CoachFailure::MalformedArguments {
            detail: format!("record_inapplicable: `{name}` must not be empty or whitespace-only"),
        });
    }
    if trimmed.len() > MAX_INAPPLICABLE_FIELD_BYTES {
        return Err(CoachFailure::MalformedArguments {
            detail: format!(
                "record_inapplicable: `{name}` is {} bytes against a \
                 {MAX_INAPPLICABLE_FIELD_BYTES}-byte ceiling",
                trimmed.len()
            ),
        });
    }
    Ok(trimmed.to_owned())
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
