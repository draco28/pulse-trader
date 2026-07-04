# 11. PulseHive dual-license & commercial self-grant for PulseTrader linkage

Date: 2026-07-04T00:00:00Z

## Status

Proposed

(Companions VS-1.3.1 — PulseHive + GLM provider wiring. Proposed-then-flip: flips to
Accepted via `/flip-adr` once the slice merges with PulseHive dual-licensed and the
`deny.toml` license exceptions green in CI. Product ADR — the public canonical repo's
license posture.)

## Context

VS-1.3.1 introduces PulseTrader's first third-party AI dependency: **PulseHive** — the
author-owned Rust multi-agent SDK — consumed for GLM-5.2 LLM transport via its
OpenAI-compatible provider. The dependency graph pulls `pulsehive` (meta) and transitively
`pulsehive-core`, `pulsehive-runtime`, `pulsehive-openai`, and `pulsehive-db`. All are
published on crates.io (pulsehive* at `2.0.2`, `pulsehive-db` at `0.5.1`) under
**AGPL-3.0-only**.

The canonical `pulse-trader` repository is **public** and licensed **PolyForm Noncommercial
1.0** (established in the VS-1.2.3 governance/IP-hardening slice). AGPL-3.0 is a strong
network-copyleft license: linking AGPL-covered code into a distributed or network-served
work normally obliges the *combined* work to be offered under AGPL (including source
availability to network users). That collides with PolyForm Noncommercial — a
source-available, non-copyleft, noncommercial license — because the two cannot both govern
the same distributed binary without an explicit resolution.

The author owns the copyright to PulseHive (and PulseDB), and is therefore free to offer
either under additional terms. PulseDB already models the resolution: `pulsehive-db`
declares `AGPL-3.0-only` in `Cargo.toml` but ships a `LICENSING.md` that additionally offers
a **commercial license** "for proprietary use without AGPL obligations" (contact
`praveensingh2897@gmail.com`).

Finally, the public repo's supply-chain gate (`cargo-deny`, from the VS-1.2.3 governance
slice) is configured **permissive-only** — MPL-2.0 was demoted to a single named
`option-ext` exception. AGPL is not permissive, so the PulseHive crates would fail the
license gate unless explicitly excepted.

## Decision

**(1) Dual-license PulseHive as `AGPL-3.0-only` + a commercial license**, mirroring PulseDB's
`LICENSING.md` posture: the crate metadata keeps `AGPL-3.0-only`; a `LICENSING.md` offers a
separate commercial license that removes the copyleft/network obligations for proprietary
use. (Publishing that `LICENSING.md` on PulseHive is a **prerequisite** of VS-1.3.1 — the
self-grant below has no license text to reference until it exists.)

**(2) PulseTrader links PulseHive under the owner-self-granted commercial license, not
AGPL.** As sole copyright holder, the author grants PulseTrader (also author-owned) a
commercial license to PulseHive. PulseTrader therefore carries **no AGPL copyleft
obligation**, and the public canonical repo **remains PolyForm Noncommercial 1.0**,
unchanged.

**(3) Record the AGPL crates as named license exceptions in the public repo's `deny.toml`** —
`pulsehive`, `pulsehive-core`, `pulsehive-runtime`, `pulsehive-openai`, `pulsehive-db` — each
commented with a pointer to this ADR and the commercial-grant rationale, exactly as the
MPL-2.0/`option-ext` exception is handled. This keeps `cargo-deny` green while
self-documenting *why* an otherwise-nonpermissive license is admitted.

## Consequences

- **(+)** The public repo's license posture is unchanged (PolyForm Noncommercial); no AGPL
  copyleft leaks to PulseTrader or its users. The mechanism is the standard, well-understood
  open-core dual-license — identical to PulseDB, so there is a working precedent to copy.
- **(+)** The commercial grant is internal (author → author): no third-party negotiation, no
  fee, no external dependency.
- **(+)** The `deny.toml` exceptions keep the supply-chain gate honest and self-documenting
  (each exception comment cites this ADR).
- **(−)** Adds a licensing surface to keep in sync: PulseHive must actually ship its
  dual-license `LICENSING.md`. **Track:** if PulseHive is still AGPL-only when VS-1.3.1
  lands, the self-grant points at nothing — publishing PulseHive's `LICENSING.md` is a hard
  prerequisite of the slice.
- **(−)** For any future *distribution* to third parties (the v1.5 signed `.app`), each
  distributee's use of the AGPL-linked binary must be covered by the commercial grant's
  terms — the commercial license text must permit end-user redistribution, not merely
  author use. Confirm the commercial-license wording covers redistribution before the first
  public release.
- **(−)** A third party who forks the public PolyForm repo does **not** automatically receive
  the PulseHive commercial grant — they would obtain PulseHive under AGPL and inherit its
  copyleft. Accepted as an intrinsic property of the open-core split (external contribution
  is already disabled per `CONTRIBUTING`).
- **Follow-up:** publish PulseHive's `LICENSING.md`; add the five `deny.toml` exceptions in
  VS-1.3.1; flip this ADR to Accepted (with an `## Empirical validation` note) once CI is
  green with the dependency + exceptions in place. Relates to ADR-0012 (the integration
  shape that introduces the dependency).
