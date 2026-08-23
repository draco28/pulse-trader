# 16. Trust boundaries: Keychain-only secrets, redact-before-persist, no order-placing path in v1

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

Secrets live in the macOS Keychain (or a gitignored `.env` for the Ollama transport)
and are injected as constructor arguments — never a committed config file, never baked
into the binary, never plaintext on disk. Every `LlmCall` is redacted **before**
persistence. **No order-placing or fund-moving path ships in v1**; the money risk gate
arrives with live execution, not before.

## Consequences

Disclosure is bounded to the redactor's correctness, which is why the redaction path is
a registered risk-gate surface carrying audit-trail, least-privilege and
no-secret-in-log controls. The Keychain choice has a live cost: `keyring` binds the
data-protection keychain, so only the `pulse` binary can seed a key it can later read
— and the seeding verb does not exist (see ADOPTION.md gap 1). The moment any path can
place an order, this bone is revisited and a Money gate is added.
