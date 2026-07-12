//! `pulse compose "<NL target>"` — the VS-1.3.2 composition root + demo verb
//! (FR-3 / FR-4 / NFR-6, README C8).
//!
//! This is the ONE place the composer's concrete stack is assembled
//! (monomorphized), keeping every layer generic underneath. The full chain the
//! live arm ([`run_compose`]) wires:
//!
//! ```text
//! .env OLLAMA_API_KEY  →  OpenAiCompatProvider::new(key)   (Ollama Cloud, gpt-oss:120b)
//!                      →  RedactingLoggingProvider::new(inner, capturing, clock, redactor, prices)
//!                         where capturing = CapturingRepo<SqliteLlmCallRepo> sharing ONE
//!                         LlmCallCapture buffer + ONE SystemClock (repo owns created_at, #82)
//!                      →  Composer::new(decorator, builder_tool_definitions(), prompt, config, buffer)
//!                      →  compose()  →  a finalized StrategyVersion value (DB-free)
//!                      →  SqliteStrategyRepo::create_strategy + create_version  →  a persisted,
//!                         attributable version (created_by = ComposerLlm, provenance ids)
//! ```
//!
//! while streaming each [`ComposerEvent`] as a visible tool-call step.
//!
//! **Injectable core (mirror `run_llm_check_with`).** [`run_compose_with`] takes the
//! LLM-side deps bundled in a [`ComposeWiring`] plus the [`StrategyRepository`], so
//! the offline e2e (`tests/compose_cli.rs`, demo criterion 1) drives the SAME
//! composition with a FAKE provider over the REAL composer + REAL builder tools + a
//! `tempfile` `SQLite` repo — never a live LLM, never the network/Keychain
//! (MASTER-SPEC §9.4).
//!
//! **Single shared clock (#82).** ONE clock is injected into BOTH the
//! [`RedactingLoggingProvider`] AND the [`SqliteLlmCallRepo`]; the repo's `save_call`
//! overrides `created_at` from its own clock, so a single shared clock keeps the
//! persisted ledger timestamp single-sourced.
//!
//! **One shared `LlmCallCapture` buffer.** ONE `Arc<Mutex<Vec<LlmCallId>>>` is wired
//! into BOTH the [`CapturingRepo`](super::llm::CapturingRepo) (which pushes each
//! minted id as the decorator writes an `LlmCall`) AND [`Composer::new`], so the
//! composer reads its run's provenance ids back after the loop.
//!
//! **Prices from config (2.03).** The price table loads from `config/prices.toml`
//! via `agent::config::load_price_table` — no price VALUE literal lives in
//! `src/cli/` (AC-11).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::adapters::clock::SystemClock;
use crate::adapters::db::{Db, SqliteLlmCallRepo, SqliteStrategyRepo};
use crate::adapters::llm::openai_compat::OpenAiCompatProvider;
use crate::adapters::llm::redacting_logging::{RedactingLoggingProvider, Redactor};
use crate::agent::config::{load_composer_prompt, load_price_table};
use crate::agent::{
    ComposeOutcome, Composer, ComposerEvent, LlmCallCapture, builder_tool_definitions,
};
use crate::domain::strategy::{NewVersion, Strategy, StrategyVersion};
use crate::domain::{
    Clock, LlmBackend, LlmCallId, LlmCallRepository, LlmConfig, LlmProvider, PriceTable,
    StrategyRepository,
};

use super::llm::CapturingRepo;

/// The Ollama Cloud model id the composer drives (README C2/C8). A model-id STRING
/// (not a price literal) — AC-11 greps `src/cli/` only for price VALUE field names.
const COMPOSE_MODEL: &str = "gpt-oss:120b";

/// A conservative sampling temperature for the compose run (wire-level `f32`, never
/// a determinism input — MASTER-SPEC §9.4 / the `LlmConfig` note).
const COMPOSE_TEMPERATURE: f32 = 0.2;

/// The response token cap. gpt-oss (like GLM 5.2) is a **reasoning** model whose
/// thinking tokens count against this cap BEFORE its tool calls, so keep generous
/// headroom past the reasoning ([VS-1.3.1 GLM]; the live demo uses 4096).
const COMPOSE_MAX_TOKENS: u32 = 4096;

/// The `.env` variable naming the Ollama Cloud API key (live-dev inject, deferral d).
const OLLAMA_API_KEY_VAR: &str = "OLLAMA_API_KEY";

/// `pulse compose "<NL target>" [--db <path>]` — turn a natural-language strategy
/// target into a persisted, attributable [`StrategyVersion`] via the composer.
#[derive(Debug, clap::Args)]
pub struct ComposeArgs {
    /// The natural-language strategy target to compose (e.g. "RSI oversold bounce
    /// on BTC with a trend filter").
    pub nl_target: String,
    /// `pulse.db` path override (defaults to the platform Application Support db);
    /// `global = true` so it parses in any position (mirror `LlmArgs.db`).
    #[arg(long, global = true)]
    pub db: Option<PathBuf>,
}

/// The LLM-side deps of one compose run, bundled so [`run_compose_with`] stays under
/// the argument-count limit and the e2e wires the SAME composition the live arm does.
///
/// `provider` is the (possibly faked) inner [`LlmProvider`]; `llm_repo` the ledger
/// repo the decorator writes through; `clock` the SINGLE shared clock (#82);
/// `prompt` the composer system prompt (2.03); `config` the per-request knobs.
pub struct ComposeWiring<P, R, C> {
    /// The inner provider (live `OpenAiCompatProvider`, or the e2e's fake).
    pub provider: P,
    /// The `LlmCall` ledger repo the redacting decorator writes each row through.
    pub llm_repo: R,
    /// The NFR-6 secret scrubber for the PERSISTED prompt/completion copy.
    pub redactor: Redactor,
    /// The README-C5 cost table (loaded from `config/prices.toml`, 2.03).
    pub prices: PriceTable,
    /// The SINGLE shared clock injected into BOTH the decorator AND `llm_repo` (#82).
    pub clock: C,
    /// The composer system prompt (2.03 `load_composer_prompt`).
    pub prompt: String,
    /// The per-request chat config (backend / model / temperature / `max_tokens`).
    pub config: LlmConfig,
}

/// The outcome of one compose run: the persisted strategy + its initial version
/// (repo-minted id / hash / `created_at`), the minted `LlmCall` provenance ids, and
/// the streamed [`ComposerEvent`]s (recorded in order).
pub struct ComposeCliOutcome {
    /// The newly-created owning [`Strategy`] (repo-minted id + timestamp).
    pub strategy: Strategy,
    /// The persisted initial [`StrategyVersion`] (`parent_version_id = None`,
    /// `created_by = ComposerLlm`, repo-minted id / `version_hash` / `created_at`).
    pub version: StrategyVersion,
    /// The `LlmCall` ids minted during the run (this version's provenance).
    pub llm_call_ids: Vec<LlmCallId>,
    /// The streamed steps, recorded in order (the CLI renders these live).
    pub events: Vec<ComposerEvent>,
}

/// The injectable, fixture-doubleable core (mirror `run_llm_check_with`): assemble
/// the redacting + cost-logging decorator over `wiring`'s provider + ledger repo
/// (sharing ONE `LlmCallCapture` buffer + ONE clock), run [`Composer::compose`] over
/// `nl_target` streaming each event to `on_event`, then persist the finalized
/// [`StrategyVersion`] as a NEW strategy + its initial version via `strategy_repo`.
///
/// The e2e drives THIS with a FAKE provider + a `tempfile` `SQLite` repo — never a live
/// LLM, never the network/Keychain (MASTER-SPEC §9.4).
///
/// # Errors
///
/// Returns an [`anyhow::Error`] if the composer loop fails (transport / budget /
/// never-finalized / max-turns), or if persisting the strategy or its version fails.
pub async fn run_compose_with<P, R, S, C>(
    wiring: ComposeWiring<P, R, C>,
    strategy_repo: &S,
    nl_target: &str,
    on_event: &mut dyn FnMut(ComposerEvent),
) -> anyhow::Result<ComposeCliOutcome>
where
    P: LlmProvider + Send + Sync,
    R: LlmCallRepository + Send + Sync,
    S: StrategyRepository + Send + Sync,
    C: Clock + Send + Sync,
{
    let ComposeWiring {
        provider,
        llm_repo,
        redactor,
        prices,
        clock,
        prompt,
        config,
    } = wiring;

    // ONE shared LlmCallCapture buffer wired into BOTH the capturing ledger repo
    // (which pushes each minted id) AND the Composer (which reads them back).
    let captured: LlmCallCapture = Arc::new(Mutex::new(Vec::new()));
    let capturing = CapturingRepo::new(llm_repo, Arc::new(Mutex::new(None)), Arc::clone(&captured));

    // The composition root: provider → redacting + cost-logging decorator over the
    // capturing repo, sharing the SINGLE clock (the ledger repo owns created_at, #82).
    let decorator = RedactingLoggingProvider::new(provider, capturing, clock, redactor, prices);
    let composer = Composer::new(
        decorator,
        builder_tool_definitions(),
        prompt,
        config,
        Arc::clone(&captured),
    );

    let outcome: ComposeOutcome = composer
        .compose(nl_target, on_event)
        .await
        .map_err(|e| anyhow::anyhow!("compose run failed: {e}"))?;

    // Persist the finalized StrategyVersion as a NEW strategy + its initial version
    // (parent_version_id = None); the repo mints id / strategy_id / version_hash /
    // created_at (the composer left those as DB-free placeholders).
    let strategy = strategy_repo
        .create_strategy(&outcome.version.dsl.name, None, &[])
        .await
        .map_err(|e| anyhow::anyhow!("persist strategy: {e}"))?;
    let version = strategy_repo
        .create_version(NewVersion {
            strategy_id: strategy.id.clone(),
            parent_version_id: None,
            dsl_json: outcome.version.dsl_original.clone(),
            created_by: outcome.version.created_by,
            creating_llm_call_ids: outcome.version.creating_llm_call_ids.clone(),
        })
        .await
        .map_err(|e| anyhow::anyhow!("persist strategy version: {e}"))?;

    Ok(ComposeCliOutcome {
        strategy,
        version,
        llm_call_ids: outcome.llm_call_ids,
        events: outcome.events,
    })
}

/// The LIVE arm (composition root): source the Ollama key from the dev `.env`, build
/// the `OpenAiCompatProvider` → decorator → `SqliteLlmCallRepo` composition over the
/// opened `db`, run the composer via [`run_compose_with`], and print the streamed
/// steps + the persisted version. This is the ONLY place the concrete stack is
/// assembled.
///
/// `db` is `Some` for this verb (the dispatcher opens a migrated `pulse.db` — the
/// ledger + version writes need it); it is `Option<&Db>` to mirror the sibling arms.
///
/// # Errors
///
/// Returns an [`anyhow::Error`] on an absent db, a missing API key, a config-load
/// failure, a composer/transport failure, or a persist failure — every path a clear
/// message + non-zero exit, never a panic.
pub async fn run_compose(db: Option<&Db>, args: &ComposeArgs) -> anyhow::Result<()> {
    let db = db.ok_or_else(|| anyhow::anyhow!("internal: compose requires an open db"))?;

    let key = ollama_api_key()?;
    // Tag the live key as a secret so an accidental echo is scrubbed at rest too
    // (structural api-key-shaped stripping is always on). Clone before the ctor move.
    let redactor = Redactor::from_config(vec![key.clone()]);
    let provider = OpenAiCompatProvider::new(key);

    // SINGLE SHARED CLOCK (#82): ONE SystemClock into the ledger repo AND the
    // decorator, so the persisted LlmCall.created_at is single-sourced. SystemClock
    // is a zero-sized Copy value, so the strategy repo's clock is the same clock.
    let clock = SystemClock;
    let llm_repo = SqliteLlmCallRepo::with_deps(db.pool().clone(), clock);
    let strategy_repo = SqliteStrategyRepo::new(db.pool().clone());

    let prices = load_price_table().map_err(|e| anyhow::anyhow!("load price table: {e}"))?;
    let prompt =
        load_composer_prompt().map_err(|e| anyhow::anyhow!("load composer prompt: {e}"))?;

    let wiring = ComposeWiring {
        provider,
        llm_repo,
        redactor,
        prices,
        clock,
        prompt,
        config: compose_config(),
    };

    let mut on_event = |event: ComposerEvent| println!("{}", render_event(&event));
    let outcome = run_compose_with(wiring, &strategy_repo, &args.nl_target, &mut on_event).await?;
    print_outcome(&outcome);
    Ok(())
}

/// The compose chat config (backend = Ollama, model = [`COMPOSE_MODEL`], generous
/// reasoning-headroom `max_tokens`).
fn compose_config() -> LlmConfig {
    LlmConfig {
        backend: LlmBackend::Ollama,
        model: COMPOSE_MODEL.to_owned(),
        temperature: COMPOSE_TEMPERATURE,
        max_tokens: COMPOSE_MAX_TOKENS,
    }
}

/// Render one streamed [`ComposerEvent`] as a single line (kept single-line so a
/// multi-line render can never truncate mid-field — the R1 harvested bug).
fn render_event(event: &ComposerEvent) -> String {
    match event {
        ComposerEvent::ToolCallStarted {
            name,
            arguments_preview,
        } => format!("  -> {name} {arguments_preview}"),
        ComposerEvent::ToolCallResult { name, outcome } => format!("     {name}: {outcome}"),
        ComposerEvent::Finalized { version_summary } => format!("  finalized: {version_summary}"),
    }
}

/// Print the persisted result: the strategy + version ids, a one-line DSL summary,
/// the provenance (`created_by` + minted `LlmCall` count).
fn print_outcome(outcome: &ComposeCliOutcome) {
    let version = &outcome.version;
    println!(
        "compose\tstrategy_id={}\tversion_id={}\tname={}\tdirection={:?}\tfilters={}\texits={}\tcreated_by={:?}\tcreating_llm_call_ids={}",
        outcome.strategy.id.as_str(),
        version.id.as_str(),
        version.dsl.name,
        version.dsl.direction,
        version.dsl.filters.len(),
        version.dsl.exits.len(),
        version.created_by,
        version.creating_llm_call_ids.len(),
    );
}

/// Source the Ollama Cloud API key for the LIVE compose run (the `user:` demo).
///
/// Order: (1) the process environment `OLLAMA_API_KEY`; (2) a gitignored `.env` in
/// the working directory or the crate manifest dir (the VS-1.3.1-validated dev
/// inject, deferral d). The real seeded-keychain read is VS-1.3.4's `pulse
/// setup-keys`. The `.env` is read IN-PROCESS only and is NEVER committed (gitignored).
///
/// # Errors
///
/// Returns an [`anyhow::Error`] naming `OLLAMA_API_KEY` + `.env` / `pulse setup-keys`
/// when no key can be sourced.
fn ollama_api_key() -> anyhow::Result<String> {
    if let Ok(key) = std::env::var(OLLAMA_API_KEY_VAR)
        && !key.is_empty()
    {
        return Ok(key);
    }
    if let Some(key) = dotenv_value(OLLAMA_API_KEY_VAR) {
        return Ok(key);
    }
    anyhow::bail!(
        "no {OLLAMA_API_KEY_VAR} found — set it in the environment or the gitignored \
         `.env` (dev inject), or seed the Keychain via `pulse setup-keys` (VS-1.3.4)"
    )
}

/// Look up `var` in the first `.env` found in the working directory or the crate
/// manifest dir. Reads the file in-process only — the caller never stages/commits it.
fn dotenv_value(var: &str) -> Option<String> {
    let candidates = [
        std::env::current_dir().ok().map(|dir| dir.join(".env")),
        Some(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".env")),
    ];
    for path in candidates.into_iter().flatten() {
        if let Ok(text) = std::fs::read_to_string(&path)
            && let Some(value) = parse_dotenv(&text, var)
        {
            return Some(value);
        }
    }
    None
}

/// Extract `var`'s value from `.env` text (the first matching `KEY=VALUE` line;
/// blank + `#`-comment lines ignored). Surrounding quotes are trimmed.
fn parse_dotenv(text: &str, var: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() == var {
            let value = value.trim().trim_matches('"').trim_matches('\'');
            if !value.is_empty() {
                return Some(value.to_owned());
            }
        }
    }
    None
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{COMPOSE_MODEL, compose_config, parse_dotenv};
    use crate::cli::{Cli, Command};
    use crate::domain::LlmBackend;
    use clap::Parser;

    #[test]
    fn parses_compose_with_positional_target() {
        let cli = Cli::try_parse_from(["pulse", "compose", "RSI oversold on BTC"]).expect("parse");
        let Command::Compose(args) = cli.command else {
            panic!("expected a compose command");
        };
        assert_eq!(args.nl_target, "RSI oversold on BTC");
        assert!(args.db.is_none());
    }

    #[test]
    fn parses_compose_db_override_globally() {
        let cli =
            Cli::try_parse_from(["pulse", "compose", "hi", "--db", "/tmp/x.db"]).expect("parse");
        let Command::Compose(args) = cli.command else {
            panic!("expected a compose command");
        };
        assert_eq!(
            args.db.as_deref().and_then(std::path::Path::to_str),
            Some("/tmp/x.db")
        );
    }

    #[test]
    fn compose_config_targets_the_pinned_ollama_model() {
        let config = compose_config();
        assert_eq!(config.backend, LlmBackend::Ollama);
        assert_eq!(config.model, COMPOSE_MODEL);
        // Reasoning headroom (gpt-oss/GLM thinking tokens count against the cap).
        assert!(
            config.max_tokens >= 4096,
            "generous cap for reasoning tokens"
        );
    }

    #[test]
    fn parse_dotenv_reads_key_ignoring_comments_and_quotes() {
        let env = "# a comment\n\nOLLAMA_API_KEY = \"abc123\"\nOTHER=nope\n";
        assert_eq!(
            parse_dotenv(env, "OLLAMA_API_KEY").as_deref(),
            Some("abc123")
        );
        assert_eq!(parse_dotenv(env, "OTHER").as_deref(), Some("nope"));
        assert!(parse_dotenv(env, "MISSING").is_none());
    }
}
