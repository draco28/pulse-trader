//! macOS Keychain secret accessor (VS-1.3.1 work-1.03, FR-1 / NFR-5, decision 5).
//!
//! The READ half of secret handling: [`glm_api_key`] fetches the GLM API key from
//! the macOS Keychain (the data-protection keychain `keyring` binds to — see the
//! seeding-reality note below) so the composition root (1.05) can inject it into
//! [`GlmProvider`](crate::adapters::llm::glm::GlmProvider) as a ctor arg — the key
//! never lives in a committed config file, an env var baked into the binary, or
//! plaintext on disk (NFR-5). A **missing** entry returns a clear
//! [`LlmError::Config`] pointing at `pulse setup-keys`, NEVER a panic.
//!
//! READ path ONLY — the `set_password` / verify half (`pulse setup-keys`) is
//! VS-1.3.4. Pinned to `keyring` 3.x, whose `Entry::new` + `get_password` bind
//! DIRECTLY to the platform keychain (no store registration). `keyring` 4.x split
//! into `keyring-core` + store crates whose lazy default-store init returned
//! `NoDefaultStore` at runtime for our unsigned dev binary (found by the live
//! user-demo at VS-1.3.1 slice close).
//!
//! **Seeding reality (macOS data-protection keychain — found at slice close):**
//! `keyring` reads/writes the modern *data-protection* keychain, which is
//! access-group-scoped by code identity. Two consequences: (a) a key written by
//! the `security add-generic-password` CLI lands in the *file-based* login
//! keychain and is INVISIBLE to `keyring` — the CLI stopgap does NOT work; (b) an
//! unsigned dev binary gets a distinct ad-hoc identity, so only the `pulse` binary
//! ITSELF (via VS-1.3.4 `pulse setup-keys`, which calls `set_password`) can seed a
//! key it can later read back. Until `setup-keys` + code-signing (v1.5) land, the
//! read path is exercised end-to-end only from a same-identity binary; the live
//! GLM transport is validated by injecting the key directly (VS-1.3.1 close demo).

use keyring::Entry;

use crate::domain::LlmError;

/// The Keychain service name `PulseTrader` stores its secrets under.
const KEYCHAIN_SERVICE: &str = "PulseTrader";

/// The Keychain account (item name) for the GLM API key.
const GLM_API_KEY_ACCOUNT: &str = "glm_api_key";

/// Read the GLM API key from the macOS Keychain (FR-1 / NFR-5).
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
