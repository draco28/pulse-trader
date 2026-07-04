# 12. Thin PulseTrader-owned LlmProvider port over PulseHive

Date: 2026-07-04T00:00:00Z

## Status

Proposed

(Companions VS-1.3.1 — PulseHive + GLM provider wiring. Proposed-then-flip: flips to
Accepted via `/flip-adr` once the slice merges with the port + adapter + redacting-logging
decorator in place. Product ADR — the agent-ring integration architecture. Relates to
ADR-0011, which resolves the licensing of the dependency this ADR introduces.)

## Context

VS-1.3.1 wires PulseTrader's first LLM backend (GLM 5.1) in through PulseHive. Two
integration depths were possible:

1. **Full runtime adoption** — make PulseHive's agentic runtime PulseTrader's agent layer:
   `HiveMind`, `AgentDefinition`, `Tool`, `Lens`, and the `pulsehive-db` "shared-consciousness"
   substrate. The roadmap summary for VS-1.3.1 reads literally as "integrate PulseHive
   HiveMind."
2. **Thin transport only** — consume PulseHive's OpenAI-compatible LLM transport behind a
   PulseTrader-owned port, and defer the runtime.

Three facts push toward option 2:

- **MASTER-SPEC §A2 explicitly flags the coupling risk:** *"PulseHive is the author's own
  evolving project; leaning on its types directly couples two moving targets… Consider
  adapting PulseHive behind PulseTrader's OWN thin agent port."* MASTER-SPEC also mandates
  (FR-23) a **uniform `LlmProvider` port** so backend selection is a config flag with no
  domain refactor, and the hexagonal architecture requires ports to live in the zero-dep
  `domain` ring.
- **The thin path is cheap and sufficient.** PulseHive's `pulsehive-openai`
  `OpenAICompatibleProvider` reaches GLM 5.2 with a thin config
  (`OpenAIConfig::new(key, "glm-5.2").with_base_url("https://api.z.ai/api/coding/paas/v4")`;
  the key is a ctor arg, not an env read — Z.AI coding-plan endpoint, owner-confirmed). VS-1.3.1's
  demo bar is only "a GLM 5.2 call round-trips through PulseHive and logs an LLMCall with
  redaction" — transport + persistence + redaction, **not** agent orchestration.
- **The full runtime carries weight VS-1.3.1 does not need.** Adopting `HiveMind` pulls
  `pulsehive-db` (an embedding/ONNX vector substrate) — a second memory store overlapping
  PulseTrader's SQLite — and couples PulseTrader to PulseHive's 2.x runtime API. PulseHive
  also returns `TokenUsage{input,output}` but **no cost**, and exposes **no verbatim-prompt
  event and no redaction hook** (`HiveEvent` carries tokens only), so cost, verbatim
  LLMCall logging, and NFR-6 redaction must be built in PulseTrader regardless of depth.

## Decision

**Consume PulseHive behind a thin PulseTrader-owned `LlmProvider` port.** The port is defined
in `domain` with `impl Future + Send` methods (per the port convention, FR-23); the GLM-5.1
adapter (in a new `adapters/llm/`) wraps `pulsehive-openai::OpenAICompatibleProvider` as
**transport only**.

**Defer `HiveMind` / `AgentDefinition` / `Tool` / `Lens` / the `pulsehive-db` substrate** to
VS-1.3.2 (composer) and VS-1.3.3 (coach), where agent orchestration is actually exercised —
and re-evaluate at that point whether those slices need PulseHive's full runtime or can stay
on the thin port plus PulseTrader-authored tool/coach frameworks.

**Build cost, verbatim LLMCall logging, and NFR-6 redaction inside PulseTrader.** A per-model
price table computes cost from the returned `TokenUsage`; a `RedactingLoggingProvider`
decorator that implements the same `LlmProvider` port captures the verbatim prompt
(`Vec<Message>`) + completion and strips secrets before persisting the `LlmCall` row. The
cost-logged path uses the non-streaming `chat()` call (only it carries usage).

## Consequences

- **(+)** Honors MASTER-SPEC §A2 + FR-23: PulseTrader sits behind its own stable port, so
  PulseHive's evolving 2.x API cannot ripple into the domain, and a backend swap
  (DeepSeek / Gemini / Claude Code / Codex) is a new adapter, not a domain change.
- **(+)** Keeps the engine lean and determinism-safe: no `pulsehive-db`/ONNX/embedding
  transitive weight, no second memory store, and the LLM boundary is a port a
  recorded-fixture double can implement (MASTER-SPEC §9.4 — LLM tests use fixtures; no LLM
  call in the determinism golden path).
- **(+)** Centralizes NFR-6 redaction + FR-24 verbatim/cost logging in one PulseTrader-owned
  seam (the decorator) — necessary regardless of depth, since PulseHive offers no such hook.
- **(−)** PulseTrader forgoes PulseHive's "shared-consciousness" cross-session coach memory
  in v1 — a `pulsehive-db`-substrate feature the coach (VS-1.3.3) may later want. Deliberately
  deferred (aligns with MASTER-SPEC's v2+ PulseDB coach-memory staging); re-open at
  coach-design time.
- **(−)** Re-implementing a minimal tool/coach framework at 1.3.2/1.3.3 partly duplicates
  capabilities `HiveMind` already provides — an accepted cost of decoupling; revisit if the
  duplication grows.
- **(−)** Only the non-streaming `chat()` path carries usage, so the cost-logged path forgoes
  token-streamed UX in v1 (acceptable for the CLI PoC; streaming is a v1.5 GUI concern).
- **Follow-up:** the `LlmProvider` port shape (methods, message/response types, error) is
  locked in VS-1.3.1; whether the composer (1.3.2) and coach (1.3.3) extend the thin port or
  adopt `HiveMind` is a tracked open decision (`10-decisions-log.md`, VS-1.3.1 KICKOFF).
  Flip to Accepted once VS-1.3.1 merges with the port + adapter + decorator in place.
