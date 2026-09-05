//! The ledger CAPTURE side-channel (r1.s4.w2, `pulseai-labs/pulse-trader#150`'s
//! neighbour).
//!
//! It lives in `mod adapters` because it IS one: a decorator over the
//! [`LlmCallRepository`] port, wrapping the real repository and exposing each write
//! through two side channels without changing the port. It sat in `mod cli` because
//! the CLI was its first consumer, which made the DESKTOP — the product — depend on
//! a type the debug surface owned. Two rings now import it from the ring that owns
//! its kind.
//!
//! Nothing about its behaviour changed in the move.

use std::sync::{Arc, Mutex};

use crate::agent::LlmCallCapture;
use crate::domain::{DataError, LlmCall, LlmCallId, LlmCallRepository};

/// A capture side-channel over an inner [`LlmCallRepository`]: it forwards
/// `save_call` to the real repo (the actual persistence) and — WITHOUT changing the
/// [`LlmCallRepository`] port — exposes each write through two channels:
///
/// - `captured` records a COPY of the most-recent saved row, so the `llm-check`
///   composition root can surface the persisted `LlmCall` (its id, redacted prompt,
///   tokens, cost) after the write; and
/// - `ids` PUSHES each minted [`LlmCallId`] into a shared [`LlmCallCapture`] buffer
///   (VS-1.3.2 work-2.05) that the `compose` composition root also wires into
///   [`Composer::new`](crate::agent::Composer::new), so the composer reads its run's
///   provenance ids back after the loop.
///
/// The port has no "last saved" read and the [`RedactingLoggingProvider`] mints the
/// row id internally, so this thin wrapper is how the id is recovered generically —
/// for BOTH the live arm and the auto-tests — without modifying 1.04's decorator.
pub(crate) struct CapturingRepo<R> {
    inner: R,
    captured: Arc<Mutex<Option<LlmCall>>>,
    ids: LlmCallCapture,
}

impl<R> CapturingRepo<R> {
    /// Wrap `inner`, sharing the single-row `captured` slot (`llm-check`) and the
    /// append-only `ids` buffer (`compose`). `llm-check` passes a throwaway `ids`
    /// buffer; `compose` passes the SAME buffer it wires into `Composer::new`.
    pub(crate) fn new(
        inner: R,
        captured: Arc<Mutex<Option<LlmCall>>>,
        ids: LlmCallCapture,
    ) -> Self {
        Self {
            inner,
            captured,
            ids,
        }
    }
}

impl<R: LlmCallRepository + Send + Sync> LlmCallRepository for CapturingRepo<R> {
    async fn save_call(&self, call: &LlmCall) -> Result<LlmCallId, DataError> {
        // Persist through the real repo FIRST (its clock overrides `created_at`);
        // only capture the row once the write actually succeeded.
        let id = self.inner.save_call(call).await?;
        if let Ok(mut slot) = self.captured.lock() {
            *slot = Some(call.clone());
        }
        // Share the minted id with the composer's provenance buffer (2.05).
        if let Ok(mut ids) = self.ids.lock() {
            ids.push(id.clone());
        }
        Ok(id)
    }

    async fn get_call(&self, id: &LlmCallId) -> Result<Option<LlmCall>, DataError> {
        self.inner.get_call(id).await
    }
}
