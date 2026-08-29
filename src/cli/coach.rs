//! `pulse coach <run-id>` — the coach composition root + debug verb (r1.s2.w3,
//! ADR-0021 / ADR-0017).
//!
//! **A developer/debug surface, and it claims no user journey** (A4, and the
//! operator's CLI-is-a-dev-surface ruling): the product surface for coaching is
//! `r1.s4`'s rail in the app. This verb exists so a human can drive one real turn
//! against the configured provider and read what was recorded.
//!
//! This is the ONE place the coach's concrete stack is assembled, keeping every
//! layer generic underneath — the `run_compose` precedent, with the coach's own
//! prompt and tool:
//!
//! ```text
//! resolve_llm_api_key()  →  ApiKey (opaque; carries the CredentialSource label)
//!                      →  OpenAiCompatProvider::{new|with_base_url}(key)
//!                      →  RedactingLoggingProvider::new(inner, capturing, clock, redactor, prices)
//!                         .with_created_by(CoachLlm).with_key_source(source)
//!                         .with_prompt_version(Some(sha256(resolved coach.md)))   ← audit C2
//!                      →  Coach::new(decorator, prompt, config, buffer)
//!                      →  run_turn()  →  ONE recorded CoachingSession
//! ```
//!
//! **Injectable core.** [`run_coach_with`] takes the LLM-side deps bundled in a
//! [`CoachWiring`] plus the three repos, so the offline tests (`coach_turn` = demo
//! `d6`, `coach_failures` = demo `d7`, `coach_redaction`) drive the SAME
//! composition with a scripted provider over the REAL coach, REAL `apply()`, REAL
//! repos and a `tempfile` `SQLite` — never a live LLM, never the network.
//!
//! **Prompt resolution lives in the core, not the caller** (unlike
//! `ComposeWiring.prompt`): the resolved prompt and the `prompt_version` stamped on
//! the ledger row must be the same bytes, and the only way to guarantee that is to
//! resolve them together, once, here.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Context as _;

use crate::adapters::clock::SystemClock;
use crate::adapters::db::{
    Db, SqliteBacktestRunRepo, SqliteCoachingRepo, SqliteLlmCallRepo, SqliteStrategyRepo,
};
use crate::adapters::llm::openai_compat::OpenAiCompatProvider;
use crate::adapters::llm::redacting_logging::{RedactingLoggingProvider, Redactor};
use crate::agent::config::load_coach_prompt_from;
use crate::agent::{Coach, LlmCallCapture};
use crate::domain::strategy::CreatedBy;
use crate::domain::{
    BacktestRunId, BacktestRunRepository, Clock, CoachFailure, CoachingRepository, CoachingSession,
    CoachingSessionId, CredentialSource, LlmBackend, LlmCallRepository, LlmConfig, LlmProvider,
    PriceTable, SessionOutcome, StrategyRepository,
};

use super::llm::CapturingRepo;

/// `pulse coach <RUN_ID> [--db <path>] [--json]`.
#[derive(Debug, clap::Args)]
pub struct CoachArgs {
    /// The persisted backtest run to coach on.
    pub run_id: String,
    /// Override the `pulse.db` path (defaults to the app-support location).
    #[arg(long)]
    pub db: Option<PathBuf>,
}

/// The LLM-side wiring for one coach turn — everything the injectable core needs
/// that a test wants to substitute.
pub struct CoachWiring<P, R, C> {
    /// The inner provider (live `OpenAiCompatProvider`, or a test's scripted double).
    pub provider: P,
    /// The `LlmCall` ledger repo the redacting decorator writes each row through.
    pub llm_repo: R,
    /// The NFR-6 secret scrubber for the PERSISTED prompt/completion copy.
    pub redactor: Redactor,
    /// The cost table (loaded from `config/prices.toml` in the live arm).
    pub prices: PriceTable,
    /// The SINGLE shared clock injected into the decorator AND the repos (#82).
    pub clock: C,
    /// Which credential source supplied the API key — a LABEL, never the key.
    pub key_source: Option<CredentialSource>,
    /// The per-request chat config.
    pub config: LlmConfig,
    /// The prompt-override directory. `None` means "read `$PULSE_PROMPT_DIR`";
    /// a test passes an explicit directory so it never mutates process-global env.
    pub prompt_dir: Option<PathBuf>,
    /// Override the per-turn wall-clock guard (audit C5). `None` = the default.
    pub turn_timeout: Option<Duration>,
    /// Override the pre-call DSL size budget. `None` = the default.
    pub max_dsl_bytes: Option<usize>,
    /// The shared buffer the capturing ledger repo pushes minted ids into, and the
    /// coach reads back to name the turn's ledger row.
    pub captured: LlmCallCapture,
}

/// The outcome of one coach turn at the CLI edge.
pub struct CoachCliOutcome {
    /// The single recorded session — a proposal or a typed failure, never neither.
    pub session: CoachingSession,
    /// The version stamped on the turn's ledger row: SHA-256 hex of the RESOLVED
    /// prompt (audit C2).
    pub prompt_version: String,
}

/// The injectable, doubleable core: resolve the prompt, load the persisted run and
/// its version, assemble the decorator, and run exactly one coach turn.
///
/// # Errors
///
/// Returns an error when the run or its version is absent, when the prompt overlay
/// exists but cannot be read, when the provider transport fails (the turn never
/// happened — ADR-0017 preserves it here at the edge), or when the session cannot
/// be recorded.
pub async fn run_coach_with<P, L, B, S, K, C>(
    wiring: CoachWiring<P, L, C>,
    run_repo: &B,
    strategy_repo: &S,
    coaching_repo: &K,
    run_id: &BacktestRunId,
) -> anyhow::Result<CoachCliOutcome>
where
    P: LlmProvider + Send + Sync,
    L: LlmCallRepository + Send + Sync,
    B: BacktestRunRepository + Send + Sync,
    S: StrategyRepository + Send + Sync,
    K: CoachingRepository + Send + Sync,
    C: Clock + Send + Sync,
{
    let CoachWiring {
        provider,
        llm_repo,
        redactor,
        prices,
        clock,
        key_source,
        config,
        prompt_dir,
        turn_timeout,
        max_dsl_bytes,
        captured,
    } = wiring;

    // The prompt and its version, resolved together from the same bytes (audit C2).
    let prompt =
        load_coach_prompt_from(prompt_dir.as_deref()).context("resolving the coach prompt")?;

    // The persisted run the coach READS (and never recomputes), plus its trades and
    // the version whose DSL a mutation addresses.
    let run = run_repo
        .get_run(run_id)
        .await
        .context("loading the backtest run")?
        .with_context(|| format!("no persisted backtest run `{}`", run_id.as_str()))?;
    let trades = run_repo
        .get_trades(run_id)
        .await
        .context("loading the run's trades")?;
    let version = strategy_repo
        .get_version(&run.strategy_version_id)
        .await
        .context("loading the run's strategy version")?
        .with_context(|| {
            format!(
                "backtest run `{}` names strategy version `{}`, which is absent",
                run_id.as_str(),
                run.strategy_version_id.as_str()
            )
        })?;

    // The coach speaks to the PORT, behind the decorator that redacts what is
    // persisted and stamps the ledger row's cost, actor and prompt version.
    let decorated =
        RedactingLoggingProvider::new(provider, llm_repo, clock, redactor.clone(), prices)
            .with_created_by(CreatedBy::CoachLlm)
            .with_key_source(key_source)
            .with_prompt_version(Some(prompt.version.clone()));

    // The SAME redactor on both roads: the decorator scrubs the ledger copy of the
    // prompt/completion, the coach scrubs the tool arguments that become stored
    // domain values (AC-3).
    let mut coach = Coach::new(decorated, prompt.text, config, captured).with_redactor(redactor);
    if let Some(timeout) = turn_timeout {
        coach = coach.with_turn_timeout(timeout);
    }
    if let Some(budget) = max_dsl_bytes {
        coach = coach.with_max_dsl_bytes(budget);
    }

    let session_id = CoachingSessionId::new(uuid::Uuid::new_v4().to_string());
    let session = coach
        .run_turn(coaching_repo, session_id, &run, &trades, &version)
        .await?;

    Ok(CoachCliOutcome {
        session,
        prompt_version: prompt.version,
    })
}

/// The live arm: assemble the real stack against `db` and run one turn, printing
/// the proposal or the typed failure.
///
/// # Errors
///
/// Returns an error when the credential cannot be resolved, the run/version is
/// absent, the transport fails, or the session cannot be recorded — each preserved
/// with its context at the CLI edge (ADR-0017).
pub async fn run_coach(db: Option<&Db>, args: &CoachArgs) -> anyhow::Result<()> {
    let db = db.context("`pulse coach` needs an opened database")?;
    let key = crate::adapters::secrets::resolve_llm_api_key()
        .map_err(|e| anyhow::anyhow!("resolve LLM API key: {e}"))?;
    let key_source = Some(key.source());
    // Tag the live key so an accidental echo is scrubbed at rest too (structural
    // api-key-shaped stripping is always on) — the `run_compose` discipline.
    let redactor = Redactor::from_config(vec![key.expose().to_owned()]);

    let transport =
        crate::agent::config::load_llm_transport().context("loading the [llm] transport config")?;
    let prices = crate::agent::config::load_price_table().context("loading the price table")?;
    let model = transport
        .model
        .clone()
        .unwrap_or_else(|| super::compose::COMPOSE_MODEL.to_owned());
    let provider = match transport.base_url.clone() {
        Some(base_url) => OpenAiCompatProvider::with_base_url(key.expose().to_owned(), base_url),
        None => OpenAiCompatProvider::new(key.expose().to_owned()),
    };

    let clock = SystemClock;
    let captured: LlmCallCapture = Arc::new(Mutex::new(Vec::new()));
    let llm_repo = CapturingRepo::new(
        SqliteLlmCallRepo::with_deps(db.pool().clone(), clock),
        Arc::new(Mutex::new(None)),
        Arc::clone(&captured),
    );

    let wiring = CoachWiring {
        provider,
        llm_repo,
        redactor,
        prices,
        clock,
        key_source,
        config: LlmConfig {
            backend: LlmBackend::Ollama,
            model,
            temperature: 0.0,
            max_tokens: 2_048,
        },
        prompt_dir: None,
        turn_timeout: None,
        max_dsl_bytes: None,
        captured,
    };

    let run_repo = SqliteBacktestRunRepo::with_deps(db.pool().clone(), clock);
    let strategy_repo = SqliteStrategyRepo::new(db.pool().clone());
    let coaching_repo = SqliteCoachingRepo::with_deps(db.pool().clone(), clock);

    let outcome = run_coach_with(
        wiring,
        &run_repo,
        &strategy_repo,
        &coaching_repo,
        &BacktestRunId::new(args.run_id.clone()),
    )
    .await?;

    print_outcome(&outcome);

    // r1.s2.w4: a transport fault is now RECORDED (the session row above) AND
    // LOUD. Routing it into the taxonomy must not quietly turn a provider outage
    // into a successful `pulse coach` invocation — the row is for the audit trail,
    // the non-zero exit is for the human and the shell (ADR-0017). The other six
    // failures are genuine coaching outcomes and exit 0.
    if let SessionOutcome::Failed {
        failure: CoachFailure::TransportFailure { detail },
    } = &outcome.session.outcome
    {
        anyhow::bail!(
            "the coach's provider call failed: {detail} (recorded as coaching session {})",
            outcome.session.id.as_str()
        );
    }
    Ok(())
}

/// Print one recorded turn. A failure is printed as loudly as a proposal — the
/// whole point of the taxonomy is that a failed turn is a result, not a blank.
fn print_outcome(outcome: &CoachCliOutcome) {
    println!("session:        {}", outcome.session.id.as_str());
    println!(
        "run:            {}",
        outcome.session.backtest_run_id.as_str()
    );
    println!(
        "version:        {}",
        outcome.session.strategy_version_id.as_str()
    );
    println!("prompt_version: {}", outcome.prompt_version);
    match outcome.session.llm_call_id.as_ref() {
        Some(id) => println!("llm_call:       {}", id.as_str()),
        None => println!("llm_call:       (none — the turn failed before any provider call)"),
    }
    match &outcome.session.outcome {
        SessionOutcome::Proposed { proposal } => {
            let (path, value) = match &proposal.mutation {
                crate::domain::Mutation::SetParam { path, new_value } => (path, new_value),
            };
            println!("\nPROPOSAL");
            println!("  path:       {path}");
            println!("  new_value:  {value:?}");
            println!("  hypothesis: {}", proposal.hypothesis.as_str());
        }
        SessionOutcome::Failed { failure } => {
            println!("\nRECORDED FAILURE");
            println!("  {failure}");
        }
    }
}
