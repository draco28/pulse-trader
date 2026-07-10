//! Agent ring (outer): `PulseHive` integration, tools, agent definitions.
//!
//! Empty stub for WI-01 to pin the hexagonal layout.

// VS-1.3.2 work-2.03: the composer's "moat in DATA" config seam — loads the
// versioned composer prompt + the nominal price table as DATA (VS-1.3.1
// decision 4). Kept `pub(crate)`: an internal seam whose first production caller
// is the composition root (2.05). Append-only (keep-both with 2.01's re-exports
// at the merge into `slice/VS-1.3.2`).
pub(crate) mod config;
