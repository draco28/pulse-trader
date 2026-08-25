//! The ONE structural secret-token heuristic (VS-1.3.2 slice-close FIX C).
//!
//! Shared by BOTH the compose-time redactor
//! ([`agent::composer`](crate::agent)) AND the at-rest `LlmCall` ledger scrubber
//! ([`adapters::llm::redacting_logging`](crate::adapters)), so the persisted copy
//! is **never weaker** than the compose-time scrub — a single source of truth for
//! the prefix set means the two can never silently drift apart again.
//!
//! Pure string logic, zero-I/O — a domain-kernel utility (no `PulseHive`, no
//! domain types, so it is safe for the `PulseHive`-free agent ring to reach).
//!
//! # Why the shapes are constrained
//!
//! This heuristic also runs over the trader's own natural-language target
//! (`composer::frame_target` scrubs it BEFORE the model sees it), so a false
//! positive is not free: it silently rewrites part of the request, and the model
//! then composes a strategy the trader did not ask for. Every matcher therefore
//! requires the FULL credential shape — a bare prefix is never enough. This
//! restores the `sk-` tail-length constraint VS-1.3.1's `redacting_logging`
//! carried and the FIX C unification dropped (PR #93 review).
//!
//! # The credential value types (r1.s1.w2)
//!
//! This file also carries the pure value half of the *LLM credential handling and
//! redaction* risk gate: [`ApiKey`] (an opaque key wrapper), [`CredentialSource`]
//! (which location answered) and [`CredentialStatus`] (the value-free banner
//! read). The resolution I/O — searching, permission-validating and reading the
//! credential file — lives in the adapter ring
//! ([`adapters::secrets`](crate::adapters)); these three types are zero-I/O, so
//! they belong here beside the redaction heuristic they exist to protect.

use serde::{Deserialize, Serialize};

/// Which location a resolved LLM credential came from — the persisted `key_source`
/// label on an [`LlmCall`](crate::domain::LlmCall) (r1.s1.w2 step 7, the audit-trail
/// control).
///
/// The serde tags (`env` / `config-dir` / `cwd-dotenv` / `app-data-dir`) are the
/// literal strings stored in `llm_call.key_source`, so a call's provenance is
/// reconstructible from the ledger alone. It is a LABEL, never the value: the whole
/// point of the audit trail is that it can be read by someone who must not learn
/// the key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CredentialSource {
    /// The process environment variable `OLLAMA_API_KEY`.
    Env,
    /// A `.env` under `$PULSE_CONFIG_DIR` (ADR-0014's data-overlay seam).
    ConfigDir,
    /// A gitignored `.env` in the working directory or the crate manifest dir.
    CwdDotenv,
    /// A `.env` in the application data directory, beside `pulse.db`.
    AppDataDir,
}

/// A resolved LLM API key plus the [`CredentialSource`] that answered.
///
/// **Opaque by construction, not by policy.** It implements neither
/// [`std::fmt::Display`] nor a value-revealing [`std::fmt::Debug`], so the key
/// cannot reach a log line, an error message or a panic payload through an
/// accidental `{}` / `{:?}` / `dbg!`. The only way to the bytes is
/// [`expose`](Self::expose), which is `pub(crate)` — an out-of-crate caller
/// receives a key it can pass on but can never read (the least-privilege control).
#[derive(Clone)]
pub struct ApiKey {
    value: String,
    source: CredentialSource,
}

/// The fixed rendering [`ApiKey`]'s `Debug` emits. It names the SOURCE (useful in a
/// diagnostic) and nothing about the value — not even its length, which is itself a
/// hint about which credential family it belongs to.
impl std::fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiKey")
            .field("source", &self.source)
            .field("value", &"<redacted>")
            .finish()
    }
}

impl ApiKey {
    /// Wrap a raw key value with the source that produced it.
    ///
    /// `pub(crate)`: only the adapter that performed the resolution may mint one, so
    /// an out-of-crate caller cannot smuggle a value in and read it back out.
    pub(crate) fn new(value: impl Into<String>, source: CredentialSource) -> Self {
        Self {
            value: value.into(),
            source,
        }
    }

    /// Which location this key came from — readable by anyone, since it is a label.
    #[must_use]
    pub fn source(&self) -> CredentialSource {
        self.source
    }

    /// Borrow the raw key bytes, for the transport ctor and the redactor's tagged
    /// secret list.
    ///
    /// `pub(crate)` on purpose — this IS the least-privilege control. The
    /// composition root inside this crate needs the value to build the provider and
    /// to tag it for redaction; nothing outside the crate ever does, and nothing
    /// outside the crate can.
    pub(crate) fn expose(&self) -> &str {
        &self.value
    }
}

/// The value-free credential read for a UI banner (r1.s1.w2 step 6) — which source
/// answered, or that none did.
///
/// This is the seam `r1.s1.w5` renders its no-credential banner from. It carries no
/// key material at all, so it is safe to send across the Tauri IPC boundary, and it
/// is computed without performing any LLM request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CredentialStatus {
    /// The process environment answered.
    Env,
    /// `$PULSE_CONFIG_DIR/.env` answered.
    ConfigDir,
    /// A working-directory / manifest-directory `.env` answered.
    CwdDotenv,
    /// The application data directory's `.env` answered.
    AppDataDir,
    /// No usable credential — absent everywhere, or found but REFUSED by the
    /// permission checks. Both read as "not usable", which is what a banner needs.
    None,
}

impl From<CredentialSource> for CredentialStatus {
    fn from(source: CredentialSource) -> Self {
        match source {
            CredentialSource::Env => Self::Env,
            CredentialSource::ConfigDir => Self::ConfigDir,
            CredentialSource::CwdDotenv => Self::CwdDotenv,
            CredentialSource::AppDataDir => Self::AppDataDir,
        }
    }
}

/// Minimum tail length for the `sk-`/`sk_`/`pk-` key families (`OpenAI`, Stripe).
/// Restored from VS-1.3.1 `redacting_logging::SK_MIN_TAIL_LEN`; without it a
/// two-character `sk-1` reads as a credential.
const SK_MIN_TAIL_LEN: usize = 16;

/// Minimum tail length for GitHub `ghp_`/`gho_` tokens (a real PAT carries 36).
const GH_MIN_TAIL_LEN: usize = 20;

/// Minimum TOTAL length for a Slack `xox…` token (real ones run well past 40).
const SLACK_MIN_LEN: usize = 24;

/// Body length after `akia` for an AWS access-key id (`AKIA` + exactly 16).
const AWS_BODY_LEN: usize = 16;

/// Minimum length of a prefix-less token for the generic high-entropy branch (a
/// long mixed-alphanumeric run — session/API tokens).
const GENERIC_MIN_LEN: usize = 32;

/// Minimum length of a separator-delimited SEGMENT that must also mix letters and
/// digits for a separator-bearing token to read as opaque (see
/// [`looks_like_opaque_run`]).
const OPAQUE_SEGMENT_MIN_LEN: usize = 8;

/// Whether a `[A-Za-z0-9_-]` token looks like an API key / secret: it matches a
/// known credential SHAPE (prefix **and** the length/charset that family
/// carries), or it reads as a long opaque high-entropy run.
///
/// Deliberately conservative so strategy words and plain numbers survive:
/// pure-digit numbers (`12345`), prose (`crossover`), and hyphenated strategy
/// phrases (`EMA-200-crossover-with-RSI-14`) never match.
#[must_use]
pub(crate) fn looks_like_secret_token(token: &str) -> bool {
    matches_credential_shape(token) || looks_like_opaque_run(token)
}

/// Whether `token` reads as a long opaque credential rather than human text.
///
/// Both directions matter here, and a naive charset rule gets one of them wrong:
///
/// - Requiring EVERY character to be alphanumeric spares hyphenated strategy
///   phrases, but silently stops redacting **base64url** credentials — whose
///   alphabet is exactly `[A-Za-z0-9_-]` (PR #93 review).
/// - Allowing separators unconditionally redacts
///   `EMA-200-crossover-with-RSI-14-filter`, because the composer's tokenizer
///   keeps `-`/`_` INSIDE a run, so that phrase arrives as one 36-char token.
///
/// The discriminator is what the separators DELIMIT. A human-readable phrase
/// splits into word-like segments that are each pure-alphabetic or pure-numeric
/// (`EMA` / `200` / `crossover`). An opaque credential carries at least one long
/// segment that MIXES letters and digits. So a separator-bearing token counts as
/// opaque only when such a segment is present.
///
/// Still imperfect in the other direction: an unbroken human-readable identifier
/// (`EMA200CrossoverWithRSI14TrendFilter`) has no separators to inspect and is
/// classified as opaque. That residual false positive needs real entropy
/// evidence rather than shape, and is tracked separately.
fn looks_like_opaque_run(token: &str) -> bool {
    if token.len() < GENERIC_MIN_LEN {
        return false;
    }
    if !token.chars().any(|c| c.is_ascii_alphabetic()) || !token.chars().any(|c| c.is_ascii_digit())
    {
        return false;
    }
    if !token.contains(['-', '_']) {
        return true;
    }
    token.split(['-', '_']).any(|segment| {
        segment.len() >= OPAQUE_SEGMENT_MIN_LEN
            && segment.chars().any(|c| c.is_ascii_alphabetic())
            && segment.chars().any(|c| c.is_ascii_digit())
    })
}

/// Whether `token` matches a known credential family in FULL — prefix plus the
/// length/charset constraint that family actually carries.
///
/// A prefix hit with the wrong shape returns `false` and falls through to the
/// caller's generic branch, so a long opaque token is never lost merely by
/// starting with a familiar-looking prefix.
fn matches_credential_shape(token: &str) -> bool {
    let lower = token.to_ascii_lowercase();

    // OpenAI / Stripe key families: `sk-`, `sk_`, `pk-` + a long opaque tail.
    for prefix in ["sk-", "sk_", "pk-"] {
        if let Some(tail) = lower.strip_prefix(prefix) {
            return tail.len() >= SK_MIN_TAIL_LEN;
        }
    }

    // GitHub personal-access / OAuth tokens: `ghp_`/`gho_` + a long body.
    for prefix in ["ghp_", "gho_"] {
        if let Some(tail) = lower.strip_prefix(prefix) {
            return tail.len() >= GH_MIN_TAIL_LEN;
        }
    }

    // Slack: `xox` + a one-letter token type + `-` + a long body (`xoxb-…`).
    if let Some(rest) = lower.strip_prefix("xox") {
        let bytes = rest.as_bytes();
        return matches!(bytes.first(), Some(b'a' | b'b' | b'e' | b'p' | b'r' | b's'))
            && bytes.get(1) == Some(&b'-')
            && token.len() >= SLACK_MIN_LEN;
    }

    // AWS access-key id: `AKIA` + exactly 16 alphanumerics (20 total).
    if let Some(body) = lower.strip_prefix("akia") {
        return body.len() == AWS_BODY_LEN && body.chars().all(|c| c.is_ascii_alphanumeric());
    }

    false
}

#[cfg(test)]
mod tests {
    use super::looks_like_secret_token;

    /// A realistic Slack bot-token SHAPE, assembled from fragments so the literal
    /// never appears whole in source — GitHub push protection matches the real
    /// `xox<type>-…` pattern and would (correctly) block a verbatim fixture.
    const SLACK_SHAPED: &str = concat!("xoxb", "-2401234567-1234567890123-AbCdEfGhIjKlMnOp");

    #[test]
    fn matches_known_prefixes_case_insensitively() {
        for token in [
            "sk-ABCD1234efGH5678",
            "sk_live_51H8xKmLpQrStUvWxYz",
            "pk-test-51H8xKmLpQrStUvWxYz",
            "ghp_abcdEFGH1234abcdEFGH1234abcdEFGH",
            "gho_abcdEFGH1234abcdEFGH1234abcdEFGH",
            SLACK_SHAPED,
            "AKIAIOSFODNN7EXAMPLE",
        ] {
            assert!(looks_like_secret_token(token), "should match: {token}");
        }
    }

    #[test]
    fn matches_long_mixed_alphanumeric_runs() {
        assert!(looks_like_secret_token(
            "abcdEFGH1234abcdEFGH1234abcdEFGH1234"
        ));
    }

    /// PR #93 review: a base64url credential's alphabet is exactly
    /// `[A-Za-z0-9_-]`, so an "all characters must be alphanumeric" rule would
    /// silently stop redacting unprefixed session/API tokens — a detection hole,
    /// the opposite failure from the strategy-phrase false positive below.
    #[test]
    fn matches_unprefixed_base64url_credentials() {
        for token in [
            "v1_9fK2mQ7xR4tZ8pL3nW6yB1cV5sD0gH-jN2kM4qP",
            "dGhpc2lz-YVRlc3QxMjM0NTY3ODkw_QWJjRGVmR2hJ",
            "9fK2mQ7xR4tZ8pL3nW6yB1cV5sD0gHjN-kM4qP7rS2t",
        ] {
            assert!(
                looks_like_secret_token(token),
                "separator-bearing credential must still be redacted: {token}"
            );
        }
    }

    #[test]
    fn spares_numbers_and_prose() {
        for token in ["12345", "12345678", "crossover", "RSI", "oversold"] {
            assert!(!looks_like_secret_token(token), "should NOT match: {token}");
        }
    }

    /// PR #93 (Codex review): a bare prefix is not a credential. Each of these
    /// trips the OLD flat `starts_with` heuristic and would silently rewrite
    /// part of the trader's request before the model ever saw it.
    #[test]
    fn spares_short_prefix_lookalikes() {
        for token in [
            "sk-1", // the dropped-SK_MIN_TAIL_LEN regression
            "SK-D", // ...case-insensitively
            "sk_1",
            "pk-USD", // a plausible ticker-ish token
            "xoxo",   // the reviewer's own example
            "XOXO123",
            "xox",
            "ghp_short",
            "Akiana", // a proper noun beginning with "akia"
            "akia",
            "AKIAIOSFODNN7", // right prefix, wrong body length
        ] {
            assert!(!looks_like_secret_token(token), "should NOT match: {token}");
        }
    }

    /// A hyphenated strategy phrase stays intact: the composer's tokenizer keeps
    /// `-`/`_` INSIDE a run, so a >=32-char phrase would otherwise hit the
    /// generic high-entropy branch and be replaced wholesale — silently changing
    /// the strategy the trader asked for.
    #[test]
    fn spares_long_hyphenated_strategy_phrases() {
        for token in [
            "EMA-200-crossover-with-RSI-14-filter",
            "BTCUSDT-M15-RSI-14-oversold-bounce",
            "trend-following-EMA-50-200-cross",
            "risk_reward_2R_stop_1p5pct_BTCUSDT",
        ] {
            assert!(
                !looks_like_secret_token(token),
                "should NOT match a strategy phrase: {token}"
            );
        }
    }
}
