//! LLM transport adapters (VS-1.3.1, README C7/C8).
//!
//! Home of the `PulseHive`-backed [`glm::GlmProvider`] anti-corruption layer — the
//! ONLY module tree in the crate importing the `PulseHive` SDK crate (AC-6). Kept
//! a thin module so 1.04's redacting-logging decorator lands as a sibling here
//! next round without disturbing the transport.

pub mod glm;
