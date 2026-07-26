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

/// Known API-key / access-token prefixes (matched case-insensitively). Covers the
/// shapes both call sites recognized before they were unified: the `sk-`/`sk_`/`pk-`
/// key families, `ghp_`/`gho_` (GitHub), `xox…` (Slack), and `akia` (AWS
/// access-key ids).
const SECRET_PREFIXES: [&str; 7] = ["sk-", "sk_", "pk-", "ghp_", "gho_", "xox", "akia"];

/// Minimum length of a prefix-less token for the generic high-entropy branch (a
/// long mixed-alphanumeric run — session/API tokens).
const GENERIC_MIN_LEN: usize = 32;

/// Whether a `[A-Za-z0-9_-]` token looks like an API key / secret: it starts with
/// a known [`SECRET_PREFIXES`] prefix (case-insensitive), OR it is a long run
/// carrying BOTH a letter AND a digit (an opaque high-entropy token).
///
/// Deliberately conservative so strategy words and plain numbers survive:
/// pure-digit numbers (`12345`) and pure-word prose (`crossover`) never match.
#[must_use]
pub(crate) fn looks_like_secret_token(token: &str) -> bool {
    let lower = token.to_ascii_lowercase();
    if SECRET_PREFIXES
        .iter()
        .any(|prefix| lower.starts_with(prefix))
    {
        return true;
    }
    token.len() >= GENERIC_MIN_LEN
        && token.chars().any(|c| c.is_ascii_alphabetic())
        && token.chars().any(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::looks_like_secret_token;

    #[test]
    fn matches_known_prefixes_case_insensitively() {
        for token in [
            "sk-ABCD1234efGH5678",
            "sk_live_0123456789",
            "pk-test-abc123",
            "ghp_abcdEFGH1234",
            "gho_abcdEFGH1234",
            "xoxb-2401-abcdEFGH",
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
}
