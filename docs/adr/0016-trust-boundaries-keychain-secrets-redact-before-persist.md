# 16. Trust boundaries: secrets never committed, redact-before-persist, no order-placing path in v1

Date: 2026-08-23T00:00:00Z

## Status

Accepted

(Accepted at adoption. This decision was made and exercised under the scaffold-dev
stack across sprints 1.1-1.3; `/ossify:adopt` recorded it as a bone on 2026-08-23
against baseline `49f229a`. Per ossify's bones protocol, decisions the adopted
baseline already exercises are minted `Accepted`, not `Proposed` — the baseline
*is* the release that exercised them. Retrospective record: it documents a
standing decision, it does not introduce one.)

## Context

The agent loop sends provider credentials upstream and persists an `LlmCall`
provenance ledger containing model inputs and outputs. Both are disclosure surfaces. v1
is backtest-only: no code path places an order or moves funds.

## Decision

Secrets are injected as constructor arguments and never appear in a committed file,
in the binary, or in a log.

**Two storage paths, and they are not equally strong.** The macOS Keychain is the
intended one. The Ollama transport instead reads `OLLAMA_API_KEY` from the environment
or a **gitignored `.env`** — which *is* plaintext on disk. That is a deliberate
development-path exception rather than an oversight: the Keychain path is currently
unreachable on a fresh install — `keyring` binds the code-identity-scoped
data-protection keychain, so only the `pulse` binary can seed a key it can later read
back, and the verb that would do it (`pulse setup-keys`) does not exist in this tree.
`src/adapters/secrets.rs` carries the full account. So `.env` is the only working path
today. The guarantee here is therefore **"never in a committed artifact"**, not "never
plaintext on disk".

**Redaction is enforced by the composition root, not by the persistence boundary.**
`RedactingLoggingProvider` scrubs before writing and every shipped composition root
routes through it. But `SqliteLlmCallRepo` is publicly exported (`src/lib.rs`) and its
`save_call` serializes `prompt_messages` and copies `completion` verbatim, so a caller
constructing it directly can persist an unredacted `LlmCall`. The guarantee holds for
the current composition roots and is **not** enforced at the write boundary. **No order-placing or fund-moving path ships in v1**; the money risk gate
arrives with live execution, not before.

## Consequences

Disclosure is bounded to the redactor's correctness, which is why the redaction path is
a registered risk-gate surface carrying audit-trail, least-privilege and
no-secret-in-log controls. The Keychain choice has a live cost: `keyring` binds the
data-protection keychain, so only the `pulse` binary can seed a key it can later read
— and the seeding verb (`pulse setup-keys`) does not exist in this tree. The moment any path can
place an order, this bone is revisited and a Money gate is added.
