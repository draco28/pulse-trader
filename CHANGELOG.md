# Changelog

All notable changes to PulseTrader will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Changed

- **Default LLM provider is now z.ai `glm-5.3` over the coding endpoint**
  (`https://api.z.ai/api/coding/paas/v4`), replacing Ollama Cloud
  (`https://ollama.com/v1`, `glm-5.2`). The Ollama Cloud subscription is dropped and
  that endpoint now answers HTTP 402, so the previous default was not runnable. See
  [ADR-0023](docs/adr/0023-default-llm-provider-zai-glm-5-3.md), which also records
  the licensing tradeoff and the constraint it places on distribution.

  **Upgrade note — an existing install will NOT pick this up on its own.** Three
  pieces of local state survive the update and each fails differently:

  1. **A `prices.toml` overlay wins over the new default.** The loader prefers an
     on-disk file whenever one exists (`$PULSE_CONFIG_DIR/prices.toml`, or the
     app-support config dir), and only falls back to the shipped table when none is
     found. An overlay written before this release still names `ollama.com` and
     `glm-5.2`, so `pulse compose` keeps dialling the retired endpoint and fails
     with **HTTP 402**. Fix: delete the overlay to inherit the new default, or edit
     its `[llm]` table to the values above.
  2. **A stale overlay also breaks `pulse llm-check`, with a misleading message.**
     That verb takes its endpoint and model from compiled-in constants (which moved
     with this release) but its prices from the same overlay file — so an overlay
     with no `[models."glm-5.3"]` row fails with **`no price for model glm-5.3`**,
     which is a config-staleness error wearing a pricing error's name. Fix: add the
     row, or delete the overlay.
  3. **The stored credential is still the Ollama one.** `OLLAMA_API_KEY` (env or
     `.env`) and the macOS Keychain entry `glm_api_key` both still hold the retired
     key, which will return **401** against z.ai. Rotate both to a z.ai key. The
     variable and Keychain account keep their Ollama-era names on purpose — see
     ADR-0023 on why that rename waits.

### Security

- Added repository security hardening, non-commercial license, and supply-chain checks (VS-1.2.3).
