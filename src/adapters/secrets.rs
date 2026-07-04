//! macOS Keychain secret accessor (VS-1.3.1 work-1.03, FR-1 / NFR-5, decision 5).
//!
//! The READ half of secret handling: [`glm_api_key`] fetches the GLM API key from
//! the login Keychain so the composition root (1.05) can inject it into
//! [`GlmProvider`](crate::adapters::llm::glm::GlmProvider) as a ctor arg — the key
//! never lives in a committed config file, an env var baked into the binary, or
//! plaintext on disk (NFR-5). A **missing** entry returns a clear
//! [`LlmError::Config`] pointing at `pulse setup-keys`, NEVER a panic.
//!
//! READ path ONLY — the `set_password` / verify half (`pulse setup-keys`) is
//! VS-1.3.4. On darwin, `keyring` 4.x default features bundle the apple-native
//! keychain store, so no extra feature flags are needed.
//!
//! **Dev-binary re-prompt caveat:** an unsigned dev binary presents a *different*
//! code identity to the Keychain ACL on each rebuild, so macOS re-prompts to
//! authorize the read every rebuild until code-signing lands (v1.5). For headless
//! / CI runs, seed the item once with
//! `security add-generic-password -s PulseTrader -a glm_api_key -w <key> -U` (or
//! add the rebuilt binary to the item's ACL) to avoid the interactive prompt.

use keyring::Entry;

use crate::domain::LlmError;

/// The Keychain service name `PulseTrader` stores its secrets under.
const KEYCHAIN_SERVICE: &str = "PulseTrader";

/// The Keychain account (item name) for the GLM API key.
const GLM_API_KEY_ACCOUNT: &str = "glm_api_key";

/// Read the GLM API key from the macOS login Keychain (FR-1 / NFR-5).
///
/// # Errors
///
/// Returns [`LlmError::Config`] when the Keychain entry is absent or the platform
/// store cannot be reached (e.g. the key was never seeded) — with a message
/// pointing the operator at `pulse setup-keys`. Never panics.
pub fn glm_api_key() -> Result<String, LlmError> {
    read_secret(KEYCHAIN_SERVICE, GLM_API_KEY_ACCOUNT)
}

/// Read a single Keychain secret, mapping every failure to a clear
/// [`LlmError::Config`].
///
/// Parameterized on `service` / `account` so a test can target a
/// guaranteed-absent name (the real `PulseTrader/glm_api_key` is seeded on the
/// owner's machine, so it can NOT stand in for the missing-entry path).
fn read_secret(service: &str, account: &str) -> Result<String, LlmError> {
    let entry = Entry::new(service, account).map_err(|error| {
        LlmError::Config(format!(
            "could not open the macOS Keychain entry {service}/{account}: {error} — run `pulse setup-keys`"
        ))
    })?;
    entry.get_password().map_err(|error| {
        LlmError::Config(format!(
            "no GLM API key found in the macOS Keychain ({service}/{account}): {error} — run `pulse setup-keys`"
        ))
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::read_secret;
    use crate::domain::LlmError;

    #[test]
    fn secrets_missing_entry_is_config_error() {
        // The REAL `PulseTrader/glm_api_key` entry is seeded (owner's live Z.AI
        // key), so this test targets a DISTINCT, guaranteed-absent account. A
        // missing item resolves to `NoEntry` immediately with NO Keychain prompt
        // (there is no ACL to authorize on a nonexistent item). The accessor must
        // surface a clear `Config` error pointing at `pulse setup-keys`, not panic.
        let err = read_secret(
            "PulseTrader",
            "glm_api_key_test_missing_9f3c1a7e_never_seed",
        )
        .expect_err("a guaranteed-absent keychain entry must return Err, not a value");
        assert!(
            matches!(&err, LlmError::Config(_)),
            "missing key must be a Config error, got {err:?}"
        );
        assert!(
            err.to_string().contains("setup-keys"),
            "config error should point at `pulse setup-keys`, got: {err}"
        );
    }
}
