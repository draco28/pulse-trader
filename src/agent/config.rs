//! The composer's config seam — the "moat in DATA, not code" loader (VS-1.3.2
//! work-2.03, VS-1.3.1 decision 4).
//!
//! The composer's behavioural content is authored as **data**, not Rust
//! literals:
//!
//! - the system prompt is a versioned `.md` file
//!   ([`prompts/composer.md`](../prompts/composer.md)), compiled in as the
//!   default via [`include_str!`] with an optional
//!   `$PULSE_PROMPT_DIR/composer.md` runtime override (the private-workspace
//!   override — forward-compat to the owner's runtime-private moat);
//! - the per-model price table loads from `config/prices.toml` through the
//!   EXISTING [`PriceTable::from_config`] seam (VS-1.3.1 C5) — this module
//!   carries **no** price VALUES (AC-8), only the wiring that reads them.
//!
//! Config-directory resolution order (README C6):
//! 1. `$PULSE_CONFIG_DIR` (explicit override),
//! 2. the dev default — the canonical repo's `config/` (via
//!    `CARGO_MANIFEST_DIR`),
//! 3. `~/Library/Application Support/PulseTrader/config/` (the app-support dir).
//!
//! All fallible paths return [`ConfigError`] (a `thiserror` enum) with a clear
//! message — **never** a panic.
//!
//! Visibility: the loader is `pub(crate)`. The composition root (2.05, R4) is
//! its first production caller; until then its only callers are this module's
//! unit tests, so the whole seam is `#![allow(dead_code)]` under
//! `deny(warnings)` (the VS-1.3.1 harvested dead-code gotcha). This is an
//! internal seam, deliberately NOT a `pub` re-export on the crate's public API
//! surface.

// The loader is built-but-unwired this slice (2.05 is its first production
// caller). Under `deny(warnings)` a `pub(crate)` fn whose only non-test caller
// does not yet exist is a `dead_code` BUILD error, so the seam is allowed here.
#![allow(dead_code)]

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::domain::{ModelPrice, PriceTable};

/// The application name namespacing the app-support config dir
/// (`~/Library/Application Support/PulseTrader/config/` on macOS).
const APP_DIR: &str = "PulseTrader";

/// Env override for the config directory (resolution order #1).
const CONFIG_DIR_ENV: &str = "PULSE_CONFIG_DIR";

/// Env override for the prompt directory (the private-workspace prompt override).
const PROMPT_DIR_ENV: &str = "PULSE_PROMPT_DIR";

/// The price-table file name under the resolved config dir.
const PRICES_FILE: &str = "prices.toml";

/// The composer-prompt file name under the resolved prompt-override dir.
const COMPOSER_FILE: &str = "composer.md";

/// The compiled-in default composer prompt (the versioned `.md`, authored as
/// DATA per `PROMPT_GOVERNANCE` §3 — not a Rust `const` string literal).
const COMPOSER_PROMPT_DEFAULT: &str = include_str!("prompts/composer.md");

/// The compiled-in default price table — the SHIPPED `config/prices.toml`,
/// embedded verbatim so a relocated or packaged binary is self-contained.
///
/// Without this floor, [`resolve_config_dir`] falls through to an app-support
/// directory that no code in this crate ever populates, so `pulse compose` and
/// `pulse llm-check` would both fail before contacting the provider whenever the
/// compile-time `CARGO_MANIFEST_DIR` checkout is absent. Embedding (rather than
/// carrying Rust price literals) keeps AC-8's grep of this file for a price
/// literal empty — the numbers still live only in the data file.
const PRICES_DEFAULT: &str = include_str!("../../config/prices.toml");

/// A config-loading failure — a missing file or a parse error. Carries the
/// offending path and the underlying error for a clear message; never a panic.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ConfigError {
    /// No platform config directory could be resolved (the `directories` crate
    /// returned `None` and no override/dev-default applied).
    #[error("no platform config directory available")]
    NoConfigDir,
    /// A config file could not be read (most often: it does not exist).
    #[error("reading config file {}: {source}", .path.display())]
    Read {
        /// The path that failed to read.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// A config file was read but could not be parsed as the expected TOML shape.
    #[error("parsing config file {}: {source}", .path.display())]
    Parse {
        /// The path that failed to parse.
        path: PathBuf,
        /// The underlying TOML deserialization error.
        #[source]
        source: toml::de::Error,
    },
}

/// The on-disk `prices.toml` shape. Only `currency` + `models` are consumed by
/// this struct's [`PriceTable::from_config`] seam; the `[llm]` table
/// (`base_url`/`model`) is parsed SEPARATELY by [`load_llm_transport`] (the
/// composition root reads it to drive the model/base-url, slice-close FIX A), so
/// it is intentionally NOT modelled here (serde ignores it), keeping this struct
/// free of any unused-field carry.
///
/// Crucially, the per-model VALUES deserialize DIRECTLY into the domain's
/// [`ModelPrice`] — this module never spells out the per-Mtok price field
/// names, so the price numbers live only in the data file, and AC-8's grep of
/// this file for a price literal stays empty.
#[derive(Debug, Deserialize)]
struct PricesConfig {
    currency: String,
    models: HashMap<String, ModelPrice>,
}

/// The resolved `[llm]` transport pinning read from `prices.toml` (slice-close
/// FIX A, ADR-0013 "config-driven model/base-url"). Both fields are optional: a
/// missing `[llm]` table or a missing field yields `None`, and the composition
/// root falls back to its documented `const` — never an error.
pub(crate) struct LlmTransport {
    /// The OpenAI-compatible base URL override (e.g. `https://ollama.com/v1`), or
    /// `None` to use the provider's `const` default.
    pub(crate) base_url: Option<String>,
    /// The model id override (e.g. `glm-5.2`), or `None` to use the compose
    /// `const` default.
    pub(crate) model: Option<String>,
}

/// The `[llm]` table's on-disk shape (only the two transport-pinning fields). A
/// separate parse struct from [`PricesConfig`] so each loader models exactly what
/// it consumes; toml ignores the sibling `currency`/`[models]` tables here.
#[derive(Debug, Default, Deserialize)]
struct TransportConfig {
    #[serde(default)]
    llm: Option<LlmTable>,
}

/// The `[llm]` table's two optional fields.
#[derive(Debug, Default, Deserialize)]
struct LlmTable {
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    model: Option<String>,
}

/// Resolve the config directory per the README C6 order (override → dev default
/// → app-support).
///
/// # Errors
///
/// Returns [`ConfigError::NoConfigDir`] when no override or dev default applies
/// and the platform data directory cannot be determined.
fn resolve_config_dir() -> Result<PathBuf, ConfigError> {
    // 1. Explicit override.
    if let Some(dir) = std::env::var_os(CONFIG_DIR_ENV) {
        return Ok(PathBuf::from(dir));
    }
    // 2. Dev default: the canonical repo's `config/` (compile-time manifest dir).
    let dev_default = Path::new(env!("CARGO_MANIFEST_DIR")).join("config");
    if dev_default.is_dir() {
        return Ok(dev_default);
    }
    // 3. App-support: ~/Library/Application Support/PulseTrader/config/.
    let dirs = directories::ProjectDirs::from("", "", APP_DIR).ok_or(ConfigError::NoConfigDir)?;
    Ok(dirs.data_dir().join("config"))
}

/// Load the price table from `prices.toml` in the resolved config dir, through
/// the EXISTING [`PriceTable::from_config`] seam (VS-1.3.1 C5).
///
/// # Errors
///
/// Returns [`ConfigError`] if the config dir cannot be resolved, the file
/// cannot be read, or its TOML cannot be parsed.
pub(crate) fn load_price_table() -> Result<PriceTable, ConfigError> {
    load_price_table_from(&resolve_config_dir()?)
}

/// Load the price table from `prices.toml` under an explicit `config_dir` (the
/// testable core of [`load_price_table`]).
///
/// # Errors
///
/// Returns [`ConfigError::Read`] if the file is present but unreadable, or
/// [`ConfigError::Parse`] if its TOML does not match the expected shape. An
/// ABSENT file is not an error — it falls back to [`PRICES_DEFAULT`].
fn load_price_table_from(config_dir: &Path) -> Result<PriceTable, ConfigError> {
    let (text, path) = read_prices_text(config_dir)?;
    let parsed: PricesConfig =
        toml::from_str(&text).map_err(|source| ConfigError::Parse { path, source })?;
    // Reuse the domain cost model's loader seam — no price VALUES live here.
    Ok(PriceTable::from_config(parsed.currency, parsed.models))
}

/// Read `prices.toml` from `config_dir`, falling back to the compiled-in
/// [`PRICES_DEFAULT`] when the file is ABSENT (a relocated/packaged binary).
///
/// Returns the TOML text plus the path to blame in a [`ConfigError::Parse`] —
/// the real path when the file was read, else the would-be path (so a malformed
/// SHIPPED default still reports a meaningful location).
///
/// A file that EXISTS but cannot be read stays a hard [`ConfigError::Read`]: an
/// unreadable override is an operator error worth surfacing, not something to
/// paper over with the default.
///
/// # Errors
///
/// Returns [`ConfigError::Read`] when the file exists but cannot be read.
fn read_prices_text(config_dir: &Path) -> Result<(String, PathBuf), ConfigError> {
    let path = config_dir.join(PRICES_FILE);
    if path.is_file() {
        let text = fs::read_to_string(&path).map_err(|source| ConfigError::Read {
            path: path.clone(),
            source,
        })?;
        return Ok((text, path));
    }
    Ok((PRICES_DEFAULT.to_owned(), path))
}

/// Load the `[llm]` transport pinning (`base_url` + model) from `prices.toml` in the
/// resolved config dir (slice-close FIX A). Reuses the SAME config-dir resolution +
/// file as [`load_price_table`]; a missing `[llm]` table or field is `None`, never
/// an error.
///
/// # Errors
///
/// Returns [`ConfigError`] if the config dir cannot be resolved, the file cannot be
/// read, or its TOML cannot be parsed (the same failure modes as the price loader).
pub(crate) fn load_llm_transport() -> Result<LlmTransport, ConfigError> {
    load_llm_transport_from(&resolve_config_dir()?)
}

/// Load the `[llm]` transport pinning from `prices.toml` under an explicit
/// `config_dir` (the testable core of [`load_llm_transport`]).
///
/// # Errors
///
/// Returns [`ConfigError::Read`] if the file is present but unreadable, or
/// [`ConfigError::Parse`] if its TOML does not parse. A present file with no
/// `[llm]` table (or empty fields) is `Ok` with `None`s — never an error, and an
/// ABSENT file falls back to [`PRICES_DEFAULT`] (same seam as the price loader).
fn load_llm_transport_from(config_dir: &Path) -> Result<LlmTransport, ConfigError> {
    let (text, path) = read_prices_text(config_dir)?;
    let parsed: TransportConfig =
        toml::from_str(&text).map_err(|source| ConfigError::Parse { path, source })?;
    let llm = parsed.llm.unwrap_or_default();
    Ok(LlmTransport {
        base_url: llm.base_url,
        model: llm.model,
    })
}

/// Load the composer system prompt.
///
/// Resolution: if `$PULSE_PROMPT_DIR/composer.md` exists it wins (the
/// private-workspace runtime override); otherwise the compiled-in default
/// ([`prompts/composer.md`](../prompts/composer.md)) is returned.
///
/// # Errors
///
/// Returns [`ConfigError::Read`] only when a `$PULSE_PROMPT_DIR/composer.md`
/// override exists but cannot be read; the compiled-in default path is
/// infallible.
pub(crate) fn load_composer_prompt() -> Result<String, ConfigError> {
    let override_dir = std::env::var_os(PROMPT_DIR_ENV).map(PathBuf::from);
    load_composer_prompt_from(override_dir.as_deref())
}

/// Load the composer prompt given an optional override directory (the testable
/// core of [`load_composer_prompt`]).
///
/// # Errors
///
/// Returns [`ConfigError::Read`] when `prompt_dir/composer.md` exists but cannot
/// be read.
fn load_composer_prompt_from(prompt_dir: Option<&Path>) -> Result<String, ConfigError> {
    if let Some(dir) = prompt_dir {
        let path = dir.join(COMPOSER_FILE);
        if path.is_file() {
            return fs::read_to_string(&path).map_err(|source| ConfigError::Read { path, source });
        }
    }
    Ok(COMPOSER_PROMPT_DEFAULT.to_owned())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{
        CONFIG_DIR_ENV, ConfigError, load_composer_prompt_from, load_llm_transport,
        load_llm_transport_from, load_price_table_from, resolve_config_dir,
    };
    use crate::domain::{SchemaVersion, TokenUsage};
    use std::sync::Mutex;

    /// Serializes the one env-mutating test so it cannot race any other test
    /// that reads `$PULSE_CONFIG_DIR` (only this test touches it).
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// AC-5: prices load from the config FILE (not a Rust literal). Write a
    /// temp `prices.toml`, load it through the seam, and assert
    /// `PriceTable::cost` returns the configured nominal value.
    /// FR-25 / NFR-10 (cost accounting reads real per-model prices from config).
    #[test]
    fn load_price_table_from_config_reads_configured_nominal_value() {
        // Materialize the SHIPPED nominal price file into an isolated temp
        // config dir (no env mutation → race-free) and load it through the real
        // seam. Sourcing the fixture from the shipped file (rather than an
        // inline literal) also keeps price field names OUT of `config.rs`, so
        // AC-8's grep of the loader stays empty by construction.
        let shipped = include_str!("../../config/prices.toml");
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("prices.toml"), shipped).unwrap();

        let table = load_price_table_from(dir.path()).expect("load price table");
        assert_eq!(table.currency(), "USD");
        // The shipped nominal: 1_000_000 in @ 0.50/Mtok + 1_000_000 out @
        // 1.50/Mtok = 0.50 + 1.50 = 2.00 (guards the shipped data values too).
        let cost = table
            .cost(
                "gpt-oss:120b",
                &TokenUsage {
                    input_tokens: 1_000_000,
                    output_tokens: 1_000_000,
                },
            )
            .expect("known model");
        assert_eq!(
            cost.normalize(),
            rust_decimal::Decimal::new(2, 0).normalize()
        );
    }

    /// An ABSENT price file falls back to the compiled-in shipped table, so a
    /// relocated/packaged binary stays self-contained (PR #93 Codex P1: nothing
    /// in this crate ever installs `prices.toml` into the app-support dir).
    #[test]
    fn load_price_table_from_missing_file_uses_the_shipped_default() {
        let dir = tempfile::tempdir().unwrap();
        let table = load_price_table_from(dir.path()).expect("absent file falls back");
        // Same nominal the shipped file encodes (guards the embed, not a literal).
        assert_eq!(table.currency(), "USD");
        assert!(
            table
                .cost(
                    "gpt-oss:120b",
                    &TokenUsage {
                        input_tokens: 1_000_000,
                        output_tokens: 1_000_000,
                    },
                )
                .is_ok(),
            "the embedded default must price the shipped models"
        );
    }

    /// The `[llm]` transport pinning also survives an absent file — same seam,
    /// so `pulse compose` still resolves its model/base-url off a packaged binary.
    #[test]
    fn load_llm_transport_from_missing_file_uses_the_shipped_default() {
        let dir = tempfile::tempdir().unwrap();
        let transport = load_llm_transport_from(dir.path()).expect("absent file falls back");
        assert_eq!(transport.model.as_deref(), Some("glm-5.3-flash"));
    }

    /// Malformed TOML is a clear [`ConfigError::Parse`], never a panic.
    #[test]
    fn load_price_table_from_malformed_toml_is_parse_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("prices.toml"),
            "this is = not valid toml [[[",
        )
        .unwrap();
        let err = load_price_table_from(dir.path()).expect_err("malformed toml errors");
        assert!(matches!(err, ConfigError::Parse { .. }));
    }

    /// AC-6: the compiled-in composer prompt encodes the never-emit-raw-JSON
    /// rule (`PROMPT_GOVERNANCE` §2.1 / §7) — a content invariant a future prompt
    /// edit cannot silently drop. NFR-6 (untrusted-input framing).
    #[test]
    fn composer_prompt_forbids_raw_dsl() {
        let prompt = load_composer_prompt_from(None).expect("compiled-in default");
        let lower = prompt.to_lowercase();
        assert!(
            lower.contains("never emit raw dsl json"),
            "composer prompt must state the never-emit-raw-DSL-JSON rule"
        );
    }

    /// The composer prompt frontmatter pins `dsl_schema_version` to
    /// `SchemaVersion::CURRENT` (NFR-12 model/schema pinning). A machine guard
    /// so the prompt and the DSL schema can never silently desync.
    #[test]
    fn composer_prompt_frontmatter_pins_current_schema_version() {
        let prompt = load_composer_prompt_from(None).expect("compiled-in default");
        let needle = format!("dsl_schema_version: \"{}\"", SchemaVersion::CURRENT);
        assert!(
            prompt.contains(&needle),
            "frontmatter must carry {needle:?} (SchemaVersion::CURRENT)"
        );
    }

    /// The `$PULSE_PROMPT_DIR/composer.md` override wins over the compiled-in
    /// default (the private-workspace override path).
    #[test]
    fn composer_prompt_override_dir_wins_over_default() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("composer.md"), "OVERRIDDEN COMPOSER PROMPT").unwrap();
        let prompt = load_composer_prompt_from(Some(dir.path())).expect("override read");
        assert_eq!(prompt, "OVERRIDDEN COMPOSER PROMPT");
    }

    /// Resolution order #1: an explicit `$PULSE_CONFIG_DIR` wins. The sole
    /// env-mutating test, serialized by `ENV_LOCK`.
    #[test]
    fn resolve_config_dir_honors_pulse_config_dir_env() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: serialized by ENV_LOCK; no other test reads $PULSE_CONFIG_DIR.
        unsafe {
            std::env::set_var(CONFIG_DIR_ENV, dir.path());
        }
        let resolved = resolve_config_dir().expect("env override resolves");
        // SAFETY: same lock scope; restore the environment before releasing it.
        unsafe {
            std::env::remove_var(CONFIG_DIR_ENV);
        }
        assert_eq!(resolved, dir.path());
    }

    /// FIX A: the `[llm]` table is now LIVE data — a `$PULSE_CONFIG_DIR` prices.toml
    /// whose `[llm].model` is `kimi-k2.6` resolves through the SAME config-dir order
    /// as the price table. The sole other env-mutating test shares `ENV_LOCK`.
    #[test]
    fn load_llm_transport_reads_model_from_config_dir_env() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("prices.toml"),
            "currency = \"USD\"\n[llm]\nbase_url = \"https://example.test/v1\"\nmodel = \"kimi-k2.6\"\n",
        )
        .unwrap();
        // SAFETY: serialized by ENV_LOCK; no other test reads $PULSE_CONFIG_DIR.
        unsafe {
            std::env::set_var(CONFIG_DIR_ENV, dir.path());
        }
        let transport = load_llm_transport();
        // SAFETY: same lock scope; restore the environment before releasing it.
        unsafe {
            std::env::remove_var(CONFIG_DIR_ENV);
        }
        let transport = transport.expect("transport loads from the env config dir");
        assert_eq!(transport.model.as_deref(), Some("kimi-k2.6"));
        assert_eq!(
            transport.base_url.as_deref(),
            Some("https://example.test/v1")
        );
    }

    /// FIX A: a present prices.toml with NO `[llm]` table yields `None`s (never an
    /// error) — the composition root then falls back to its documented `const`s.
    /// Race-free (explicit dir, no env mutation).
    #[test]
    fn load_llm_transport_missing_table_is_none_not_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("prices.toml"), "currency = \"USD\"\n").unwrap();
        let transport =
            load_llm_transport_from(dir.path()).expect("no [llm] table is not an error");
        assert!(transport.model.is_none());
        assert!(transport.base_url.is_none());
    }
}
