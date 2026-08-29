# 23. Default LLM provider: z.ai `glm-5.3` over the coding endpoint

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

### The licensing consideration, stated plainly

ADR-0013 rejected this endpoint on licensing grounds, and this ADR ships it anyway.
That is a conscious reversal of a weighed tradeoff, not an oversight, and the terms
of it belong in writing:

- The GLM Coding Plan is a **flat monthly subscription with a prompt quota**, sold
  for use inside coding-agent tooling. It is not a per-token programmatic API tier.
- PulseTrader at v1 is a single-operator proof-of-concept. The subscription is the
  operator's own, the binary runs on the operator's machine, and the credential is
  the operator's personal one. The exposure ADR-0013 was guarding against — shipping
  a programmatic product on a personal-use plan — does not exist while there is
  exactly one user and that user is the plan holder.
- **It starts existing the moment PulseTrader is distributed.** A code-signed,
  notarized `.app` handed to anyone else cannot carry the operator's coding-plan
  credential, and pointing someone else's install at this endpoint under this plan
  is the thing the licence forbids. This is a *distribution* blocker, recorded here
  so it is not rediscovered at the notarization step.

The consequence section below carries the exit: z.ai's standard per-token API is a
different endpoint on the same account, and the price row this ADR adds is already
that endpoint's published tariff.

## Decision

**Ship z.ai `glm-5.3` over the coding endpoint as the default.**

`config/prices.toml` is the shipped default, and it is data — ADR-0014's
data-overlay posture and ADR-0013's slice-close FIX A both make `[llm]` a live
config table read by `agent::config::load_llm_transport`:

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

**The price row is nominal, and sourced.** The coding endpoint is quota-billed, so
no per-token tariff is levied on this traffic — exactly the situation Ollama Cloud
created, and `prices.toml` has always said so. What changes is that the nominal is
now a published number rather than an estimate: `glm-5.3` is priced at z.ai's
standard-API rate for the same model, **$1.40 / 1M input and $4.40 / 1M output**
([docs.z.ai pricing](https://docs.z.ai/guides/overview/pricing), retrieved
2026-08-29). ADR-0014's discipline holds — the values live in the data file and
`src/agent/config.rs` still carries no price numbers.

**Nothing is renamed.** Three identifiers still say Ollama while the traffic is
z.ai: the `OLLAMA_API_KEY` env var the resolver chain reads first, the `OLLAMA_*`
transport consts, and `LlmBackend::Ollama`, which is persisted verbatim on every
`llm_call` row. They move together or not at all, because the third is a migration
rather than a rename, and half a rename is worse than none. Registered on the ossify
feature map under "Configurable LLM provider selection".

## Consequences

**(+)** The shipped default is executable. A clean checkout plus a key in
`OLLAMA_API_KEY` reaches a model, which was not true of `main` before this change.

**(+)** The `LlmCall` ledger's cost column stops being a number nobody can trace.
It is still nominal, but it is now a citable tariff for the model actually being
called.

**(−) Distribution is now gated on a provider change, not just on notarization.**
Shipping to any second user means moving off the coding plan — to z.ai's standard
per-token endpoint (`https://api.z.ai/api/paas/v4`, same account, billed at the rate
this ADR's price row already names), or to a provider each user configures for
themselves. Because `[llm].base_url` is config, that move is a one-line data edit,
not a code change; it is the licence, not the architecture, that has to be dealt
with. Revisit this ADR at the first distribution attempt.

**(−) Existing installs do not see the flip, silently.** `read_prices_text` prefers
an on-disk `prices.toml` over the embedded default whenever the file exists, so an
operator with a `$PULSE_CONFIG_DIR` overlay keeps dialling the retired endpoint and
gets HTTP 402, and `llm-check` fails with a misleading "no price for model
`glm-5.3`" when its stale `[models]` table has no such row. The Keychain
`glm_api_key` entry likewise still holds the retired Ollama key. This is documented
as an upgrade note in `CHANGELOG.md` rather than fixed with a runtime warning — a
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
does not need rewriting inverts the purpose of the record.

**Use z.ai's standard per-token endpoint (`https://api.z.ai/api/paas/v4`) now.**
Rejected for v1, and it is the strongest alternative. It is unambiguously licensed
for programmatic use — it removes the entire licensing tension above — and it needs
the same one-line config change. It was not taken because the operator's paid
capacity is the coding plan; routing v1's iteration traffic through the per-token
tier would bill twice for the same work, on a proof-of-concept whose only user
already holds a quota that covers it. The moment there is a second user, this
becomes the answer.

**Rename the `OLLAMA_*` surface in the same change.** Rejected. The credential env
var, the transport consts and the persisted `LlmBackend::Ollama` label are one debt
item; the third requires a migration, so renaming the first two now would leave the
codebase in a state where the names disagree with each other as well as with the
endpoint. Registered on the feature map to be done once, with the settings surface
that motivates it.

**Add a runtime warning when a stale overlay names a retired endpoint.** Rejected
for this change. Detecting "your config is from a previous version" correctly means
versioning the config file, which is a feature with its own design (what version
marker, what happens on downgrade, does it warn or refuse). A default flip is the
wrong vehicle. The upgrade note in `CHANGELOG.md` covers the operator who is
actually affected — currently one person, who is reading this.
