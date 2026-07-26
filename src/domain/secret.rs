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

/// Whether a `[A-Za-z0-9_-]` token looks like an API key / secret: it matches a
/// known credential SHAPE (prefix **and** the length/charset that family
/// carries), OR it is a long unbroken alphanumeric run holding BOTH a letter and
/// a digit (an opaque high-entropy token).
///
/// Deliberately conservative so strategy words and plain numbers survive:
/// pure-digit numbers (`12345`), prose (`crossover`), and hyphenated strategy
/// phrases (`EMA-200-crossover-with-RSI-14`) never match.
#[must_use]
pub(crate) fn looks_like_secret_token(token: &str) -> bool {
    if matches_credential_shape(token) {
        return true;
    }
    // Generic opaque token: an UNBROKEN alphanumeric run. Requiring no `-`/`_`
    // is what keeps a long hyphenated strategy phrase from being mistaken for a
    // 32-char opaque secret — the composer's tokenizer treats `-`/`_` as token
    // characters, so `EMA-200-crossover-with-RSI-14-filter` arrives here as ONE
    // 36-char run. Real separator-bearing credentials all carry a recognizable
    // prefix and are caught above.
    token.len() >= GENERIC_MIN_LEN
        && token.chars().all(|c| c.is_ascii_alphanumeric())
        && token.chars().any(|c| c.is_ascii_alphabetic())
        && token.chars().any(|c| c.is_ascii_digit())
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
