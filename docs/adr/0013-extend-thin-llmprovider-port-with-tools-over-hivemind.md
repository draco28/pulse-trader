# 13. Extend the thin LlmProvider port with tool-calling over PulseHive HiveMind

Date: 2026-07-10T00:00:00Z

## Status

Accepted

(Companions VS-1.3.2 — Composer agent + builder tools. Proposed-then-flip: flipped to
Accepted on 2026-07-26 when the slice merged into `sprint-1.3` (PR #93, merge commit
`2a9d9f4`) with the tools-carrying port + the PulseTrader-owned composer loop in
place — see § Empirical validation. Product ADR — the agent-orchestration
architecture. Directly resolves the open follow-up ADR-0012 named at its close:
"whether the composer (1.3.2) and coach (1.3.3) extend the thin port or adopt
`HiveMind` is a tracked open decision.")

## Context

VS-1.3.2 builds the composer agent: it must advertise six builder tools to GLM 5.2,
receive tool calls back, dispatch them, and loop until the strategy is finalized. This
is the first slice that exercises **agent orchestration** (not just transport), so it
forces the integration-depth decision ADR-0012 deliberately deferred. Two paths:

1. **Adopt PulseHive's agentic runtime** — `HiveMind`, `Agent`, `Tool` (trait), `Lens`,
   and the `pulsehive-runtime` + `pulsehive-db` "shared-consciousness" substrate — and
   express the composer as a PulseHive `Agent` with PulseHive `Tool`s and a `Lens`.
2. **Extend PulseTrader's own thin `LlmProvider` port with tool-calling** and run a
   PulseTrader-owned dispatch loop, keeping the runtime deferred.

Facts established at VS-1.3.2 kickoff (verified against PulseHive 2.0.2 source, 2026-07-10):

- **Tool-calling lives at the transport layer, not behind `HiveMind`.**
  `pulsehive_core::llm::LlmProvider::chat(messages, tools: Vec<ToolDefinition>, config)`
  already accepts tool definitions and returns `LlmResponse { tool_calls }`;
  `pulsehive-openai::OpenAICompatibleProvider` (which PulseTrader already consumes)
  serializes the OpenAI `{"type":"function","function":{…}}` wire array and parses the
  wire `arguments` string back to a `serde_json::Value`. So tool-calling needs **none**
  of `HiveMind`/`Agent`/`Lens`/`pulsehive-db`.
- **`pulsehive-openai` does not depend on `pulsehive-db`/`-runtime`.** Staying on the
  transport port means the embedding/ONNX vector stack (`hnsw_rs`→`bincode`,
  `tokenizers`→`paste` — the two unmaintained advisories PulseTrader already had to
  policy-accept) is **never activated**. Adopting `HiveMind` would pull `pulsehive-db`
  into active use — the exact cost ADR-0012 flagged for re-evaluation at this slice.
- **VS-1.3.1 pre-designed this exact change.** Its README C1 named `tools` "the additive
  port change 1.3.2 makes when it needs it," and `LlmResponse.tool_calls` is already
  carried through the domain.
- **The composer's Lens scope is a construction discipline, not a runtime need.** The
  composer must see only the strategy-composition surface (PROMPT_GOVERNANCE §2.1) — this
  is satisfied by simply never fetching backtests/trades/balances/secrets into the
  prompt; it does not require PulseHive's `Lens` machinery for a single-agent,
  no-cross-session-memory composer.
- **A PulseTrader-owned loop is small and gives full control** over per-tool validation,
  correctable-error feedback, streaming (one visible step per tool call), compose-time
  redaction, and the `LlmCall` ledger — all PulseTrader-owned concerns PulseHive does not
  expose hooks for (established in ADR-0012).

## Decision

**Extend the thin PulseTrader-owned `LlmProvider` port with tool-calling and run a
PulseTrader-owned composer loop.** Concretely:

- Add a PulseTrader-owned `ToolDefinition` type (in `domain`, mirroring PulseHive's shape
  — ADR-0012 insulation) and extend `LlmProvider::chat` additively to
  `chat(messages, tools: &[ToolDefinition], config)`; an empty `tools` slice reproduces
  the VS-1.3.1 no-tool behavior.
- The `GlmProvider` anti-corruption adapter translates PulseTrader `ToolDefinition` ↔
  PulseHive `ToolDefinition` and PulseTrader `ToolCall` ↔ PulseHive `ToolCall`
  field-by-field; it remains the only `pulsehive`-importing file.
- The composer (`mod agent`) is a PulseTrader-authored loop that calls
  `provider.chat(messages, &tools, &config)`, dispatches each returned tool call to
  PulseTrader builder tools that validate against the existing DSL schema
  (`domain::dsl::validate`), feeds correctable errors back, and finalizes a
  `StrategyVersion`.

**Continue to defer `HiveMind` / `Agent` / `Lens` / the `pulsehive-db` substrate** — now
through VS-1.3.3 (coach) as well. Re-open only if the coach's cross-session memory
(a `pulsehive-db`-substrate feature) is judged worth the active-use cost at coach-design
time (aligns with MASTER-SPEC's v2+ PulseDB coach-memory staging).

**Concrete v1 backend (provider pivot, 2026-07-10):** the first backend behind this port
is **Ollama Cloud** (`https://ollama.com/v1`, OpenAI-compatible, `gpt-oss:120b`), consumed
via `pulsehive_openai::OpenAICompatibleProvider` — NOT the GLM/Z.AI coding-plan endpoint
(that endpoint is licensed for personal coding-agent use only; Ollama Cloud is a
subscription API for programmatic use). A live spike verified clean `/v1` tool-calling on
`gpt-oss:120b` and the full premium catalog. The VS-1.3.1 `GlmProvider` is generalized to a
config-driven `OpenAiCompatProvider`; `LlmBackend::Glm` → `LlmBackend::Ollama`; the base URL +
model are config-driven from `config/prices.toml`'s `[llm]` table (each with a `const` fallback),
realized at slice-close (FIX A). This does not change the thin-transport premise (ADR-0012) — only
the concrete base-URL/model/key. A native PulseHive `OllamaProvider` is a filed non-blocking
enhancement (`pulseai-labs/PulseHive#35`).

## Consequences

- **(+)** Keeps the engine free of `pulsehive-db`/ONNX/embedding **active** use and of a
  second memory store overlapping SQLite; the two unmaintained advisories stay dormant
  (linked, not exercised). Determinism surface unmoved.
- **(+)** Honors MASTER-SPEC §A2 + FR-23: PulseTrader sits behind its own stable port, so
  PulseHive's evolving 2.x runtime API cannot ripple into the composer, and a backend
  swap remains a new adapter, not a domain change.
- **(+)** Full PulseTrader ownership of per-tool validation, correctable-error UX,
  streaming, compose-time redaction, and the `LlmCall` ledger — the FR-3 / NFR-6 / FR-24
  guarantees are all PulseTrader-controlled.
- **(−)** PulseTrader hand-rolls a small agent loop (turn cap, budget guard, tool
  dispatch) that `HiveMind` would provide — an accepted, bounded duplication (the loop is
  a single-agent, finalize-terminated loop, not a general agentic runtime). Revisit if
  the coach/auto-optimizer make the duplication grow.
- **(−)** Forgoes PulseHive `Lens`-enforced scoping in favor of construction-discipline
  scoping (the composer simply never injects forbidden context). Acceptable for a
  single-agent composer; re-evaluate if multi-agent shared context arrives.
- **Follow-up:** the tools-carrying port shape is locked in VS-1.3.2; VS-1.3.3's coach
  reuses the same thin loop unless cross-session memory forces a `pulsehive-db`
  re-evaluation. Flip to Accepted once VS-1.3.2 merges with the port + composer loop in
  place.

## Empirical validation

- **Validated:** 2026-07-26
- **Signal:** A live end-to-end `pulse compose` run against glm-5.2 (Ollama Cloud,
  OpenAI-compatible) drove the full natural-language → persisted `StrategyVersion`
  round-trip through the thin tools-carrying port: streamed tool calls dispatched by
  name, a finalized schema-valid document, and version `9ca7c9d7-a842-4b50-a95a-e49264a5bb3c`
  persisted with `created_by = ComposerLlm` and 6 `creating_llm_call_ids`, with zero
  secret leaks in the resulting `LlmCall` rows (NFR-6). The decision therefore holds in
  practice: the composer reached a finalized strategy with **no** `HiveMind` / `Agent` /
  `Lens` runtime and with `pulsehive-db` still not activated.
- **Merged:** VS-1.3.2 landed on `sprint-1.3` as PR #93, merge commit `2a9d9f4`
  (2026-07-26), after four review passes — 11 findings fixed, 10 tracked
  (#92, #94–#100). `openai_compat.rs` remains the sole `PulseHive` importer and the
  agent ring stays `HiveMind`/`pulsehive-db`-free.
- **Determinism:** golden `142.29083294950040454` byte-unmoved across the whole slice;
  100× sequential + 100× parallel determinism and the cross-arch (x86_64 == arm64)
  `result_content_hash` compare green on the merged head.

### What the review passes revealed about the decision

The decision's `(−)` consequence — "PulseTrader hand-rolls a small agent loop … an
accepted, bounded duplication" — is where the review concentrated. The loop needed
several hardening fixes a mature runtime would have supplied (rejecting unknown tool
arguments, sealing the untrusted-target delimiter, ordering within batched tool-call
turns → #98). That is the predicted cost of option 2, arriving as predicted and at a
bounded size; it did not change the verdict, but it sharpens the stated follow-up: if
VS-1.3.3's coach grows the same class of loop concerns again, re-evaluate rather than
hand-roll a second time.
