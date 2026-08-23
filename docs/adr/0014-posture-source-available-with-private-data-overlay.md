# 14. Posture: source-available canonical with a private data-overlay moat

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

The canonical repo `pulseai-labs/pulse-trader` is public under PolyForm Noncommercial
1.0; the AI workspace `pulse-trader-internal` is private. VS-1.3.2 work item 2.03
deliberately built a *moat-in-data* config seam: the composer prompt and the
per-model price table load from `\$PULSE_PROMPT_DIR` / `\$PULSE_CONFIG_DIR`
overrides, falling back to compiled-in defaults. Ossify adoption requires an explicit
posture; absence would have to resolve `fully-private`, which contradicts a repo that
is already public.

## Decision

Posture is **source-available**, with a **data-overlay** moat channel seamed at
`\$PULSE_PROMPT_DIR`. The code is readable and non-commercially usable; the tuned
prompt and price data are the withheld asset and ship from a private overlay rather
than from a private code fork. No moat item is named in the public
`PUBLIC_BOUNDARY.md`; the inventory lives in the AI workspace.

## Consequences

Public->private is impossible, so this posture is effectively one-way. Contributors
can read and run the engine but cannot commercially deploy it. The overlay seam must
stay overridable, which makes `src/agent/config.rs` a bone touch surface: a change
that hardcodes prompts or prices collapses the moat channel. Revenue intent is
licence-restriction, not a paid feature tier; moving to open-core is the revisit
trigger.
