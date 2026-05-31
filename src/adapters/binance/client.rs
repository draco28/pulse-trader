//! HTTP transport wrapper with bounded retry/backoff and error mapping
//! (spec §3 audit C4/C5, AC-5/AC-7).
//!
//! The concrete transport is `reqwest`, but the retry policy is written against
//! an injectable async operation (a `FnMut() -> Future` closure) so AC-5/AC-7
//! can be exercised entirely offline (AC-6) with a fake op that scripts
//! failures. Retry policy:
//! retryable = network errors + HTTP 429/5xx; non-retryable = other 4xx; max 4
//! attempts; exponential backoff base 500 ms, cap 8 s, full jitter.
//!
//! 404 disambiguation (audit C2) is *not* decided here: the transport surfaces
//! a [`TransportError::Status`] carrying the code, and the bulk layer decides
//! pre-listing-skip vs. transient-retry based on whether earlier months in the
//! window succeeded. A 404 is therefore treated as non-retryable at the
//! transport level (it is a definite "not here now"); the bulk-window logic
//! handles the "expected but missing" retry semantics.

use std::future::Future;
use std::time::Duration;

use crate::domain::DataError;

/// Maximum total attempts for a retryable request (audit C4).
const MAX_ATTEMPTS: u32 = 4;
/// Backoff base delay (audit C4).
const BACKOFF_BASE: Duration = Duration::from_millis(500);
/// Backoff cap (audit C4).
const BACKOFF_CAP: Duration = Duration::from_secs(8);

/// A transport-level failure, classified for the retry policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TransportError {
    /// A connection/timeout/DNS-style failure with no HTTP status — retryable.
    Network(String),
    /// An HTTP response with a non-success status code. Retryable iff `429` or
    /// `5xx`; other 4xx are terminal.
    Status(u16),
}

impl TransportError {
    /// Whether this failure should be retried (network + 429/5xx).
    fn is_retryable(&self) -> bool {
        match self {
            TransportError::Network(_) => true,
            TransportError::Status(code) => *code == 429 || (500..600).contains(code),
        }
    }

    /// Map a terminal transport failure to a [`DataError`] (AC-5/AC-7).
    fn into_data_error(self) -> DataError {
        match self {
            TransportError::Network(msg) => DataError::Io(format!("network error: {msg}")),
            TransportError::Status(code) => DataError::Io(format!("HTTP status {code}")),
        }
    }
}

/// The retry/backoff policy (audit C4). Pure decision logic + a pluggable sleep
/// so tests run without real wall-clock delay.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RetryPolicy {
    max_attempts: u32,
    base: Duration,
    cap: Duration,
    /// When false, backoff sleeps are skipped (test mode) — the *decision* to
    /// retry is unchanged; only the wall-clock wait is elided.
    sleep_enabled: bool,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: MAX_ATTEMPTS,
            base: BACKOFF_BASE,
            cap: BACKOFF_CAP,
            sleep_enabled: true,
        }
    }
}

impl RetryPolicy {
    /// A policy that does not sleep between attempts (for offline tests).
    #[cfg(test)]
    fn no_sleep() -> Self {
        Self {
            sleep_enabled: false,
            ..Self::default()
        }
    }

    /// The backoff delay before the next attempt after `failures` failures,
    /// using full jitter over exponential `base * 2^(failures-1)`, capped.
    ///
    /// Exposed for the AC-5 unit test that asserts the ceiling never exceeds the
    /// cap regardless of attempt index.
    fn backoff_ceiling(&self, failures: u32) -> Duration {
        // base * 2^(failures-1), then capped. Cap the exponent at 31 so the
        // multiplier never overflows; the `.min(cap)` below clamps the result
        // long before that point anyway (the cap is reached by ~attempt 5).
        let exp = failures.saturating_sub(1).min(31);
        let factor: u64 = 1u64 << exp;
        let base_ms = u64::try_from(self.base.as_millis()).unwrap_or(u64::MAX);
        let scaled_ms = base_ms.saturating_mul(factor);
        Duration::from_millis(scaled_ms).min(self.cap)
    }

    async fn sleep(&self, ceiling: Duration) {
        if self.sleep_enabled {
            // Full jitter: a uniform draw in [0, ceiling]. A cheap LCG keyed on
            // the ceiling nanos avoids pulling a rng crate for one sleep.
            let span = u64::try_from(ceiling.as_nanos()).unwrap_or(u64::MAX).max(1);
            let nanos = lcg_jitter(span);
            tokio::time::sleep(Duration::from_nanos(nanos)).await;
        }
    }

    /// Run `op` under the retry policy, retrying retryable failures up to
    /// `max_attempts` total, sleeping with capped full-jitter backoff between
    /// attempts. The final failure maps to a [`DataError`] (AC-5).
    async fn run<F, Fut>(&self, op: F) -> Result<Vec<u8>, DataError>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<Vec<u8>, TransportError>>,
    {
        self.run_structured(op)
            .await
            .map_err(TransportError::into_data_error)
    }

    /// Like [`run`](Self::run) but surfaces the terminal [`TransportError`]
    /// **structurally** instead of stringifying it into a [`DataError`]. Callers
    /// that must branch on the HTTP status — e.g. [`BinanceClient::fetch_optional`]
    /// recovering a `404` as absence — use this so the decision is made on the
    /// status *code*, not on error-message text (audit C2).
    async fn run_structured<F, Fut>(&self, mut op: F) -> Result<Vec<u8>, TransportError>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<Vec<u8>, TransportError>>,
    {
        let mut failures: u32 = 0;
        loop {
            match op().await {
                Ok(body) => return Ok(body),
                Err(err) => {
                    failures += 1;
                    let exhausted = failures >= self.max_attempts;
                    if !err.is_retryable() || exhausted {
                        return Err(err);
                    }
                    let ceiling = self.backoff_ceiling(failures);
                    self.sleep(ceiling).await;
                }
            }
        }
    }
}

/// Map a structured transport result to the [`BinanceClient::fetch_optional`]
/// contract: a `404` **status** is a legitimate absence (`Ok(None)`) the bulk
/// window disambiguates; any other terminal failure is a real error. Pure so the
/// 404-vs-error discrimination is unit-tested offline and is decided on the HTTP
/// status code rather than on a stringified error message.
fn classify_optional(
    result: Result<Vec<u8>, TransportError>,
) -> Result<Option<Vec<u8>>, DataError> {
    match result {
        Ok(body) => Ok(Some(body)),
        Err(TransportError::Status(404)) => Ok(None),
        Err(other) => Err(other.into_data_error()),
    }
}

/// Tiny deterministic LCG used only for backoff jitter (no crypto, no rng dep).
fn lcg_jitter(span: u64) -> u64 {
    use std::cell::Cell;
    use std::time::{SystemTime, UNIX_EPOCH};
    thread_local! {
        static STATE: Cell<u64> = const { Cell::new(0) };
    }
    STATE.with(|s| {
        let mut x = s.get();
        if x == 0 {
            x = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX))
                .unwrap_or(0x9E37_79B9)
                | 1;
        }
        // LCG (Numerical Recipes constants).
        x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        s.set(x);
        x % span
    })
}

/// The production transport + retry wrapper around `reqwest` (AC-5).
///
/// Holds a configured `reqwest::Client` (rustls, gzip, per-request timeout) and
/// the [`RetryPolicy`]. `fetch` runs a GET under the policy and returns the body
/// bytes; all failures map to [`DataError`] (no panics).
pub(crate) struct BinanceClient {
    http: reqwest::Client,
    policy: RetryPolicy,
}

impl BinanceClient {
    /// Build a client with a sensible request timeout and the default retry
    /// policy.
    ///
    /// # Errors
    ///
    /// [`DataError::Io`] if the underlying `reqwest::Client` cannot be built.
    pub(crate) fn new() -> Result<Self, DataError> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| DataError::Io(format!("cannot build HTTP client: {e}")))?;
        Ok(Self {
            http,
            policy: RetryPolicy::default(),
        })
    }

    /// GET `url` under the retry policy, returning the body bytes (AC-5/AC-7).
    ///
    /// # Errors
    ///
    /// [`DataError::Io`] after retries are exhausted or on a non-retryable
    /// transport failure.
    pub(crate) async fn fetch(&self, url: &str) -> Result<Vec<u8>, DataError> {
        self.policy.run(|| self.transport_get(url)).await
    }

    /// One GET against reqwest, classified into a [`TransportError`].
    async fn transport_get(&self, url: &str) -> Result<Vec<u8>, TransportError> {
        let resp = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|e| TransportError::Network(e.to_string()))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(TransportError::Status(status.as_u16()));
        }
        resp.bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| TransportError::Network(e.to_string()))
    }

    /// GET `url`, mapping a `404` to `Ok(None)` (a legitimate absence the bulk
    /// window disambiguates) and any other terminal failure to `Err` (AC-7).
    ///
    /// # Errors
    ///
    /// [`DataError::Io`] on a non-`404` terminal failure or after retries.
    pub(crate) async fn fetch_optional(&self, url: &str) -> Result<Option<Vec<u8>>, DataError> {
        // Decide 404-absence on the HTTP status *code* (via `run_structured`),
        // NOT by substring-matching a stringified error: a non-404 transport
        // failure whose message merely contains "404" must not be misread as
        // absence and silently drop a month's data (CodeRabbit Major).
        classify_optional(self.policy.run_structured(|| self.transport_get(url)).await)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{RetryPolicy, TransportError, classify_optional};
    use crate::domain::DataError;
    use std::cell::Cell;
    use std::time::Duration;

    // ---- fetch_optional 404 detection is status-typed, not string-matched --
    //
    // Regression (CodeRabbit Major): `fetch_optional` previously recovered a 404
    // absence via `msg.contains("404")` on a stringified error, so a non-404
    // failure whose message happened to contain "404" was misread as `Ok(None)`
    // and silently dropped a month. Discrimination now keys on the status code.

    #[test]
    fn classify_optional_treats_a_404_status_as_absence() {
        assert!(matches!(
            classify_optional(Err(TransportError::Status(404))),
            Ok(None)
        ));
    }

    #[test]
    fn classify_optional_returns_the_body_on_success() {
        let got = classify_optional(Ok(b"data".to_vec())).expect("ok");
        assert_eq!(got, Some(b"data".to_vec()));
    }

    #[test]
    fn classify_optional_does_not_read_a_404ish_network_message_as_absence() {
        // A network failure whose message merely contains "404" must surface as
        // an error, NOT a false absence (the old substring match returned None).
        let res = classify_optional(Err(TransportError::Network(
            "connection reset talking to host x404y".into(),
        )));
        assert!(
            matches!(res, Err(DataError::Io(_))),
            "a non-404 network error must not be classified as absence, got {res:?}"
        );
    }

    #[test]
    fn classify_optional_propagates_non_404_statuses_as_errors() {
        assert!(matches!(
            classify_optional(Err(TransportError::Status(500))),
            Err(DataError::Io(_))
        ));
        assert!(matches!(
            classify_optional(Err(TransportError::Status(403))),
            Err(DataError::Io(_))
        ));
    }

    // ---- AC-5: retry classification + bounds ------------------------------

    #[test]
    fn classifies_retryable_vs_terminal() {
        assert!(TransportError::Network("reset".into()).is_retryable());
        assert!(TransportError::Status(429).is_retryable());
        assert!(TransportError::Status(503).is_retryable());
        assert!(TransportError::Status(500).is_retryable());
        // Non-retryable 4xx.
        assert!(!TransportError::Status(404).is_retryable());
        assert!(!TransportError::Status(400).is_retryable());
        assert!(!TransportError::Status(403).is_retryable());
    }

    #[test]
    fn backoff_ceiling_grows_then_caps_at_8s() {
        let p = RetryPolicy::no_sleep();
        assert_eq!(p.backoff_ceiling(1), Duration::from_millis(500));
        assert_eq!(p.backoff_ceiling(2), Duration::from_secs(1));
        assert_eq!(p.backoff_ceiling(3), Duration::from_secs(2));
        assert_eq!(p.backoff_ceiling(4), Duration::from_secs(4));
        // Capped at 8s thereafter.
        assert_eq!(p.backoff_ceiling(5), Duration::from_secs(8));
        assert_eq!(p.backoff_ceiling(50), Duration::from_secs(8));
    }

    #[tokio::test]
    async fn retries_network_failure_then_succeeds() {
        let attempts = Cell::new(0u32);
        let op = || {
            let n = attempts.get();
            attempts.set(n + 1);
            async move {
                if n < 2 {
                    Err(TransportError::Network("reset".into()))
                } else {
                    Ok(b"ok".to_vec())
                }
            }
        };
        let body = RetryPolicy::no_sleep().run(op).await.expect("eventual ok");
        assert_eq!(body, b"ok");
        assert_eq!(attempts.get(), 3, "two failures then a success");
    }

    #[tokio::test]
    async fn gives_up_after_max_4_attempts_on_persistent_5xx() {
        let attempts = Cell::new(0u32);
        let op = || {
            attempts.set(attempts.get() + 1);
            async move { Err::<Vec<u8>, _>(TransportError::Status(503)) }
        };
        let err = RetryPolicy::no_sleep().run(op).await.expect_err("exhausts");
        assert!(matches!(err, DataError::Io(_)));
        assert_eq!(attempts.get(), 4, "max 4 attempts (audit C4)");
    }

    #[tokio::test]
    async fn does_not_retry_terminal_4xx() {
        let attempts = Cell::new(0u32);
        let op = || {
            attempts.set(attempts.get() + 1);
            async move { Err::<Vec<u8>, _>(TransportError::Status(400)) }
        };
        let err = RetryPolicy::no_sleep()
            .run(op)
            .await
            .expect_err("400 is terminal");
        assert!(matches!(err, DataError::Io(_)));
        assert_eq!(attempts.get(), 1, "no retry on a non-retryable 4xx");
    }

    #[tokio::test]
    async fn maps_terminal_failure_to_data_error_no_panic() {
        let op = || async { Err::<Vec<u8>, _>(TransportError::Status(404)) };
        let err = RetryPolicy::no_sleep().run(op).await.expect_err("404");
        match err {
            DataError::Io(msg) => assert!(msg.contains("404")),
            other => panic!("expected Io, got {other:?}"),
        }
    }
}
