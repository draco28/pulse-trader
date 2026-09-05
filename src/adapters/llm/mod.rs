//! LLM transport adapters (VS-1.3.1 → VS-1.3.2, README C2/C7/C8).
//!
//! Home of the `PulseHive`-backed [`openai_compat::OpenAiCompatProvider`]
//! anti-corruption layer — the ONLY module tree in the crate importing the
//! `PulseHive` SDK crate (AC-9). Kept a thin module so the redacting-logging
//! decorator sits as a sibling here without disturbing the transport.

pub mod openai_compat;
pub mod redacting_logging;
// r1.s4.w1 (#132): the attributed coach provider — the redacting/logging decorator
// and the capture buffer composed into ONE adapter, so the sealed turn receives a
// response and the ledger row it minted together instead of a provider and a
// capture handle it has to pair correctly.
pub(crate) mod attributed;
