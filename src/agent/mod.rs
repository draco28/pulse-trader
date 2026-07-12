//! Agent ring (outer): `PulseHive` integration, tools, agent definitions.
//!
//! Empty stub for WI-01 to pin the hexagonal layout.

// VS-1.3.2 work-2.03: the composer's "moat in DATA" config seam — loads the
// versioned composer prompt + the nominal price table as DATA (VS-1.3.1
// decision 4). Kept `pub(crate)`: an internal seam whose first production caller
// is the composition root (2.05). Append-only (keep-both with 2.01's re-exports
// at the merge into `slice/VS-1.3.2`).
pub(crate) mod config;

// VS-1.3.2 work-2.02: the FR-3 heart — the six server-validated builder tools +
// the `StrategyBuilder` accumulator they mutate. `mod builder` owns the
// order-independent partial-strategy state machine + `finalize` → whole-document
// `validate()` (reused verbatim); `mod tools` owns the flat-primitive tool fns,
// the isolated flat→tagged mapping seam (README C4 reversibility), and the
// `ToolDefinition` instances. Their first production caller is the composer loop
// (2.04, R3), so the re-exported surface is `#[allow(unused_imports)]` +
// `#![allow(dead_code)]` (the VS-1.3.1 dead-code-under-deny(warnings) gotcha).
// Append-only (keep-both with 2.03's `mod config;` at the merge into slice/VS-1.3.2).
mod builder;
mod tools;

#[allow(unused_imports)]
pub(crate) use builder::StrategyBuilder;
#[allow(unused_imports)]
pub(crate) use tools::{
    ToolOutcome, add_entry_signal, add_filter, builder_tool_definitions, create_strategy,
    finalize_strategy, set_exit_rules, set_risk_params,
};
