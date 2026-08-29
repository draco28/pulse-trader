# Changelog

All notable changes to PulseTrader will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Changed

- **Default LLM model bumped `glm-5.2` → `glm-5.3-flash`** on Ollama Cloud. The
  provider, endpoint (`https://ollama.com/v1`), credential (`OLLAMA_API_KEY`) and
  ledger backend label are all unchanged — this moves a model id and its price row.
  See [ADR-0023](docs/adr/0023-retain-ollama-cloud-bump-default-model-to-glm-5-3-flash.md).

  Notes for anyone upgrading or reading the diff:

  - **The model id is written bare — `glm-5.3-flash`, no `:cloud` tag.** Ollama's
    library page publishes only a `cloud` tag and its examples show
    `glm-5.3-flash:cloud`, but the endpoint accepts the bare id (verified by a live
    call, tool-calling included), and the bare form matches how `glm-5.2` was
    written. Noted because the docs and the endpoint disagree here.
  - **A `$PULSE_CONFIG_DIR/prices.toml` overlay wins over the shipped default.** An
    overlay written before this release still pins `glm-5.2` and will keep using it.
    Delete it to inherit the new default, or edit its `[llm].model` and add a
    matching `[models."glm-5.3-flash"]` row — the cost path fails closed on a
    model with no price row.
  - **Verified end to end before landing.** A live `pulse compose` run on the new
    default dispatched six tool calls, finalized a schema-valid strategy, and
    persisted six `LlmCall` rows (peak `output_tokens` 701 against the 4096 cap, so
    no truncation; no secret in the ledger). This mattered more than a transport
    check: `gpt-oss:120b` once passed API-level tool-calling on this same endpoint
    and then failed mid-loop.
  - Multi-model tiering (`glm-5.3` for harder tasks, `gpt-oss:120b` for light ones)
    is planned work and is **not** implemented; exactly one model id is read.

  A flip to z.ai's GLM Coding Plan endpoint was drafted and fully reviewed before
  this, then rejected: that plan's terms prohibit spending its quota from a custom
  application calling the API directly, by usage shape rather than by user count.
  Preserved unmerged as [PR #123](https://github.com/pulseai-labs/pulse-trader/pull/123).

### Security

- Added repository security hardening, non-commercial license, and supply-chain checks (VS-1.2.3).
