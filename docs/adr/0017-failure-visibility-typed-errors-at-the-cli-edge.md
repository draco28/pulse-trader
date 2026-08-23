# 17. Failure visibility: typed errors surfaced at the CLI edge; structured tracing deferred

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

v1 is a single-user CLI proof-of-concept. There is no background process, no service to
page, and no operator other than the person who typed the command.

## Decision

Domain and adapter layers use **typed errors**. The **CLI edge does not preserve
them**: `cli::run() -> anyhow::Result<()>`, and the verbs convert with
`anyhow::anyhow!("<context>: {e}")`, which stringifies and discards the variant. The
operator gets a contextual message; a caller cannot branch by variant at that
boundary. Structured tracing is **deliberately deferred** — the `eprintln!`-vs-
`tracing` question was parked at the VS-1.2.4 close rather than answered.

## Consequences

Adequate while a human watches every invocation, and honest about being thin. It stops
being adequate the moment something runs unattended: the native app shell (v1.5), paper
trading, or live execution each need operator-visible health that a CLI stderr line
cannot provide. That is the revisit trigger, and it will arrive before v2.
