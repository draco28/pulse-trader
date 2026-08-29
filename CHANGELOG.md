# Changelog

All notable changes to PulseTrader will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Changed

- **Development-cycle LLM default is now z.ai `glm-5.3` over the coding endpoint**
  (`https://api.z.ai/api/coding/paas/v4`), replacing Ollama Cloud
  (`https://ollama.com/v1`, `glm-5.2`). The Ollama Cloud subscription is dropped, and
  that endpoint answers **HTTP 402 for the operator's account** — an account-state
  observation, not a claim about the service, though the practical effect is the same
  here: the previous default was not runnable by anyone building this repo. This
  is the default for the pre-distribution phase; a distributed build will select its
  provider through the config seam instead (candidates: z.ai's per-token API, or
  Ollama serving GLM). GLM is the model family either way, and the `glm-5.3` /
  `glm-5.3-flash` variant choice is still open. See
  [ADR-0023](docs/adr/0023-development-llm-default-zai-glm-coding-endpoint.md).

  **Upgrade note — an existing install will NOT pick this up on its own.** Three
  pieces of local state survive the update and each fails differently:

  1. **A `prices.toml` overlay wins over the new default.** The loader prefers an
     on-disk file whenever one exists (`$PULSE_CONFIG_DIR/prices.toml`, or the
     app-support config dir), and only falls back to the shipped table when none is
     found. An overlay written before this release still names `ollama.com` and
     `glm-5.2`, so `pulse compose` keeps dialling the retired endpoint — failing
     with **HTTP 402** on an account whose Ollama Cloud subscription has lapsed, or
     with a `glm-5.2` model error on one that has not. Fix: delete the overlay to
     inherit the new default, or edit its `[llm]` table to the values above.
  2. **A stale overlay also breaks `pulse llm-check`, with a misleading message.**
     That verb takes its endpoint and model from compiled-in constants (which moved
     with this release) but its prices from the same overlay file — so an overlay
     with no `[models."glm-5.3"]` row fails with **`no price for model glm-5.3`**,
     which is a config-staleness error wearing a pricing error's name. Fix: add the
     row, or delete the overlay.
  3. **The stored credentials are still the Ollama ones**, and only one of them can
     actually be rotated today.
     - `pulse compose` reads `OLLAMA_API_KEY` (environment, or a `.env` in the
       search order at the top of `.env.example`). Put a z.ai key there and compose
       works. The variable keeps its Ollama-era name on purpose — see ADR-0023.
     - `pulse llm-check` reads **only** the macOS data-protection Keychain account
       `glm_api_key`, which still holds the retired key and will return **401**.
       There is currently **no supported way to rotate it**: that Keychain path is
       read-only in this crate, `pulse setup-keys` does not exist yet, and
       `security add-generic-password` writes the *login* keychain, which `keyring`
       cannot see. So `llm-check` is expected to fail after this change until a
       credential writer lands. Use `pulse compose` to exercise the provider in the
       meantime. Tracked, not fixed here.

### Security

- Added repository security hardening, non-commercial license, and supply-chain checks (VS-1.2.3).
