# 23. Development-cycle LLM default: z.ai GLM over the coding endpoint

Date: 2026-08-29T00:00:00Z

## Status

Accepted

(Supersedes **ADR-0013's concrete-backend choice only** — its "provider pivot,
2026-07-10" paragraph, which named Ollama Cloud as the v1 backend and explicitly
rejected the z.ai coding endpoint. ADR-0013's actual decision — extend the thin
`LlmProvider` port with tool-calling rather than adopt `HiveMind` — is untouched and
remains Accepted, as does ADR-0012's thin-transport premise. This ADR moves the base
URL, the model and the price row; it moves no port boundary.

Authored `Accepted` rather than `Proposed`-then-flip because the exercise that
normally gates the flip already happened: the operator drove a full `pulse compose`
round trip against this exact endpoint and model on 2026-08-28, through a
`$PULSE_CONFIG_DIR` overlay, before the ruling was made. The decision is being
recorded after the evidence, not ahead of it.)

## Context

ADR-0013's provider pivot picked Ollama Cloud and gave a specific reason for
rejecting the alternative:

> **NOT** the GLM/Z.AI coding-plan endpoint (that endpoint is licensed for personal
> coding-agent use only; Ollama Cloud is a subscription API for programmatic use).

That reasoning was sound when written. What broke was the premise underneath it: the
Ollama Cloud subscription this project shipped against **does not work**. The
operator's account returns HTTP 402 on `https://ollama.com/v1`, so the shipped
default was not merely suboptimal — it was untestable. Nobody could run
`pulse compose` against a clean checkout and reach a model. A default that no
operator can execute is not a default; it is a placeholder that the repository was
describing as a decision.

Two further facts made the situation worse than a dead endpoint:

- The drift was already on record and already unresolved. ADR-0001's index note on
  ADR-0008 says in as many words that "the current default is an open question
  needing its own ADR, not a settled decision this entry still describes." That note
  has been standing since 2026-08-23. This ADR is the record it asked for.
- The operator holds a working **GLM Coding Plan** subscription and had verified,
  by hand, that `https://api.z.ai/api/coding/paas/v4` with `glm-5.3` drives the
  composer's full tool-calling loop. The working configuration and the shipped
  configuration had diverged, with the working one living only in a private overlay.

### What this ADR decides, and what it does not

The distinction a future reader is most likely to get wrong, so it goes first:
**this sets the default for the development cycle, not the shipped end-state.**
PulseTrader is a pre-distribution proof-of-concept. The only traffic this default
carries is the operator's own iteration — composing strategies, exercising the coach
loop, driving the tool-calling path. What a *distributed* build points at is a
separate decision, deliberately deferred and deliberately made configurable (see
**Deployment** in the Decision below).

Reading this ADR as "PulseTrader ships on the coding plan" is reading it backwards.

### The licensing consideration, stated plainly

ADR-0013 rejected this endpoint on licensing grounds, and this ADR uses it anyway
for the development cycle. That is a conscious acceptance of a weighed tradeoff, not
an oversight, and the terms belong in writing:

- The GLM Coding Plan is a **flat monthly subscription with a prompt quota**, sold
  for use inside coding-agent tooling. It is not a per-token programmatic API tier.
- Through the development cycle, the subscription is the operator's own, the binary
  runs on the operator's machine, and the credential is the operator's personal one.
  The exposure ADR-0013 guarded against — a *product* running on a personal-use
  plan — does not exist while there is exactly one user and that user is the plan
  holder.
- **It would exist the moment PulseTrader is distributed**, which is exactly why
  distribution does not inherit this default. The deployment provider is a separate,
  already-scheduled choice, not an oversight to be caught later.

## Decision

**For the development cycle: z.ai GLM over the coding endpoint.**

`config/prices.toml` carries it, as data — ADR-0014's data-overlay posture and
ADR-0013's slice-close FIX A both make `[llm]` a live config table read by
`agent::config::load_llm_transport`:

```toml
[llm]
base_url = "https://api.z.ai/api/coding/paas/v4"
model = "glm-5.3"
```

**The compiled-in `const` fallbacks move with it.** This is not redundancy. The
config table does not reach every path: `pulse compose` and the Tauri compose
command read it, but `pulse llm-check` is transport-const-pinned and would have kept
dialling `ollama.com` with `glm-5.2` after a config-only edit — a default that moved
in one verb and not the other. So `OLLAMA_BASE_URL` / `OLLAMA_MODEL_ID`
(`src/adapters/llm/openai_compat.rs`), `DEMO_MODEL` (`src/cli/llm.rs`) and
`COMPOSE_MODEL` (`src/cli/compose.rs`) all move. Values only; no wiring changed.

**GLM is the model family — for the development cycle and past it.** That much is
settled, and it holds across every provider candidate below. **The variant is not
settled.** `glm-5.3` is the current default *pending evaluation*, not a selection:
`glm-5.3` versus `glm-5.3-flash` is an open experiment (cost and latency against
composer quality on real strategy targets), and this ADR deliberately leaves it open.
Changing variant is a one-line `[llm].model` edit plus a price row; nothing else in
this decision depends on which one wins.

**Deployment: the provider becomes configurable, and this default does not carry
over.** A distributed build selects its provider through the configuration seam
rather than inheriting the operator's coding plan. Two candidates, both keeping GLM
as the model family:

1. **z.ai's per-token API** (`https://api.z.ai/api/paas/v4`) — the same account and
   the same models, on the tier unambiguously licensed for programmatic use. The
   price row this ADR adds is already that tier's published tariff.
2. **Ollama serving GLM** — the local Ollama runtime hosting GLM weights, which is
   *not* the retired Ollama Cloud subscription. A different tradeoff: no per-call
   cost and no third-party licence question, against the user supplying their own
   hardware.

Choosing between them is out of scope here. It is registered on the ossify feature
map as **"Configurable LLM provider selection"**, together with the runtime settings
surface that exposes it.

**The price row is nominal, and sourced.** The coding endpoint is quota-billed, so
no per-token tariff is levied on this traffic — exactly the situation Ollama Cloud
created, and `prices.toml` has always said so. What changes is that the nominal is
now a published number rather than an estimate: `glm-5.3` is priced at z.ai's
per-token API rate for the same model, **$1.40 / 1M input and $4.40 / 1M output**
([docs.z.ai pricing](https://docs.z.ai/guides/overview/pricing), retrieved
2026-08-29). That doubles as forward-pricing for deployment candidate 1, so ledger
figures accumulated now stay meaningful afterwards. ADR-0014's discipline holds —
the values live in the data file and `src/agent/config.rs` still carries no price
numbers.

**Nothing is renamed.** Three identifiers still say Ollama while the traffic is
z.ai: the `OLLAMA_API_KEY` env var the resolver chain reads first, the `OLLAMA_*`
transport consts, and `LlmBackend::Ollama`, which is persisted verbatim on every
`llm_call` row. They move together or not at all, because the third is a migration
rather than a rename, and half a rename is worse than none. Registered on the same
feature-map entry as the provider seam, which is the work that motivates it.

## Consequences

**(+)** The development default is executable. A clean checkout plus a key in
`OLLAMA_API_KEY` reaches a model, which was not true of `main` before this change.

**(+)** The `LlmCall` ledger's cost column stops being a number nobody can trace.
Still nominal, but now a citable tariff for the model actually being called — and
for the most likely deployment provider.

**(+)** Because `[llm]` is data, the deployment choice costs a config edit, not a
code change. The seam that makes a provider-specific dev default safe is the same
seam that makes the later decision cheap.

**(−) The deployment provider becomes a required decision before first
distribution.** It cannot slide past that point: shipping a build pointed at the
coding endpoint is the thing the licence forbids. Scheduled work rather than a
discovered blocker — but a hard gate on distribution, recorded here so it is not
rediscovered at the notarization step.

**(−) The model variant is unresolved and is shipping as a default anyway.**
`glm-5.3` sits in `prices.toml` because something has to, not because it beat
`glm-5.3-flash` on evidence. Read the config file as the current default, not as a
conclusion.

**(−) Existing installs do not see the flip, silently.** `read_prices_text` prefers
an on-disk `prices.toml` over the embedded default whenever the file exists, so an
operator with a `$PULSE_CONFIG_DIR` overlay keeps dialling the retired endpoint and
gets HTTP 402, and `llm-check` fails with a misleading "no price for model
`glm-5.3`" when its stale `[models]` table has no such row. The Keychain
`glm_api_key` entry likewise still holds the retired Ollama key. Documented as an
upgrade note in `CHANGELOG.md` rather than fixed with a runtime warning — a
version-aware config migration is a real feature and does not belong in a default
flip.

**(−) The retained naming debt gets one release older.** Every `llm_call` row
written from now on is tagged `ollama` while pointing at z.ai. The ledger stays
internally consistent (the label never meant the endpoint), but a reader who has not
seen this ADR will be misled, which is why the enum, the consts and `.env.example`
all now say so at their definition sites.

**(−) `glm-5.3`'s token-cap and timeout posture is unverified on real runs.** The
4096 `max_tokens` cap and the 60s / 2-retry transport posture were tuned for
`glm-5.2` on a different endpoint and have not been measured against this model.
Tracked as an issue, not blocked on here.

## Alternatives considered

**Stay on Ollama Cloud, fix the subscription.** Rejected by the operator: the
subscription is being dropped, not renewed. Keeping a default alive purely so an ADR
need not be rewritten inverts the purpose of the record.

**Use z.ai's per-token API for development too.** Rejected *for now*, and it is the
strongest alternative — it removes the licensing tension entirely and needs the same
one-line config change. Not taken because the operator's paid capacity **is** the
coding plan: routing iteration traffic through the per-token tier would bill twice
for the same work, on a proof-of-concept whose only user already holds a quota that
covers it. This is not a rejection of the endpoint, only of using it during
development — it is deployment candidate 1 above.

**Decide the deployment provider in this ADR.** Rejected as premature. Both
candidates are viable and the choice turns on facts not yet in hand: what deployment
actually looks like, whether a user brings hardware or a key, what the variants cost
at real volume. Deciding now would be guessing with an ADR's authority. The seam is
built; the choice is registered on the feature map.

**Pin the model variant.** Rejected for the same reason. `glm-5.3` versus
`glm-5.3-flash` is an evaluation that has not been run, and recording a default is
honest where recording a decision would not be.

**Rename the `OLLAMA_*` surface in the same change.** Rejected. The credential env
var, the transport consts and the persisted `LlmBackend::Ollama` label are one debt
item; the third requires a migration, so renaming the first two now would leave the
codebase in a state where the names disagree with each other as well as with the
endpoint. Registered on the feature map to be done once, with the provider seam that
motivates it.

**Add a runtime warning when a stale overlay names a retired endpoint.** Rejected
for this change. Detecting "your config is from a previous version" correctly means
versioning the config file, which is a feature with its own design (what version
marker, what happens on downgrade, warn or refuse). A default flip is the wrong
vehicle. The upgrade note in `CHANGELOG.md` covers the operator who is actually
affected — currently one person, who is reading this.
