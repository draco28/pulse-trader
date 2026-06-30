//! Build-time `engine_fingerprint` computation (VS-1.2.3 work-3.01, decision D5).
//!
//! Runs BEFORE the crate compiles (so it cannot import crate types) and computes a
//! deterministic sha2-256 hex digest over the four D5 inputs that pin a
//! byte-reproducible engine build:
//!   1. the raw bytes of the workspace `Cargo.lock` (the full resolved dep graph);
//!   2. the *resolved* `rustc -vV` filtered to its `release:` + `commit-hash:`
//!      lines — the `host:` line is EXCLUDED (it varies by build host and is not
//!      the property we fingerprint; the target triple below covers arch). The
//!      resolved compiler is hashed (not `rust-toolchain.toml`'s text) so a stray
//!      `rustup override` cannot silently desync the binary from its toolchain;
//!   3. the DSL schema-version string (`DSL_SCHEMA_VERSION`, `include!`'d from the
//!      SAME `schema_version_const.rs` the crate reads — the non-drift seam);
//!   4. the full target triple (`$TARGET`, set by Cargo for build scripts).
//!
//! It then bakes the digest into the binary via
//! `cargo:rustc-env=PULSE_ENGINE_FINGERPRINT=<hex>` and the triple via
//! `PULSE_TARGET_TRIPLE`, plus `cargo:rerun-if-changed=` for `Cargo.lock` and the
//! schema-const file so the fingerprint stays correct without over-rebuilding.

// The crate-wide `[lints.clippy]` table denies `unwrap_used`/`expect_used` so
// *library* paths cannot panic (audit C5). A build script is the opposite case:
// panicking IS its idiomatic, correct failure mode — a missing Cargo-guaranteed
// env var or an unreadable `Cargo.lock` MUST hard-fail the build, not be silently
// recovered. We scope the allow narrowly to this build-script crate; the library
// no-panic invariant is untouched.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::env;
use std::path::Path;
use std::process::Command;

use sha2::{Digest, Sha256};

// The SAME single-source const the crate's `schema_version.rs` reads. `include!`
// (not a module load) brings `DSL_SCHEMA_VERSION` into this build script's scope,
// so the schema input to the fingerprint can never drift from `SchemaVersion::CURRENT`
// (the crate's non-drift test asserts the equality).
include!("src/domain/dsl/schema_version_const.rs");

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR is always set by Cargo for a build script");

    // Input (a): the raw bytes of the workspace Cargo.lock (next to Cargo.toml in
    // this single-package repo). Hashing the bytes captures the full resolved
    // dependency graph.
    let lock_path = Path::new(&manifest_dir).join("Cargo.lock");
    let lock_bytes = std::fs::read(&lock_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", lock_path.display()));

    // Input (b): the resolved `rustc -vV`, filtered to `release:` + `commit-hash:`.
    // Use the compiler Cargo selected ($RUSTC) so a `rustup override` is reflected;
    // fall back to a bare `rustc` only if unset.
    let rustc = env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned());
    let rustc_vv = Command::new(&rustc)
        .arg("-vV")
        .output()
        .unwrap_or_else(|e| panic!("failed to run `{rustc} -vV`: {e}"));
    assert!(
        rustc_vv.status.success(),
        "`{rustc} -vV` exited with {:?}",
        rustc_vv.status
    );
    let rustc_vv = String::from_utf8(rustc_vv.stdout).expect("`rustc -vV` output is valid UTF-8");
    // Keep ONLY the release: + commit-hash: lines (exclude host:, LLVM version, etc.).
    // The retained lines are joined with '\n' in their original order for a stable,
    // host-independent compiler identity.
    let rustc_resolved = rustc_vv
        .lines()
        .filter(|line| line.starts_with("release:") || line.starts_with("commit-hash:"))
        .collect::<Vec<_>>()
        .join("\n");

    // Input (d): the full target triple Cargo is building for.
    let target = env::var("TARGET").expect("TARGET is always set by Cargo for a build script");

    // sha2-256 over the four inputs, domain-separated by a NUL byte so no input's
    // tail can be confused with the next input's head.
    let mut hasher = Sha256::new();
    hasher.update(&lock_bytes);
    hasher.update([0u8]);
    hasher.update(rustc_resolved.as_bytes());
    hasher.update([0u8]);
    // Input (c): the DSL schema version (from the `include!`'d single-source const).
    hasher.update(DSL_SCHEMA_VERSION.as_bytes());
    hasher.update([0u8]);
    hasher.update(target.as_bytes());
    let fingerprint = hex::encode(hasher.finalize());

    // Bake the fingerprint + triple into the crate (read via `env!` in fingerprint.rs).
    println!("cargo:rustc-env=PULSE_ENGINE_FINGERPRINT={fingerprint}");
    println!("cargo:rustc-env=PULSE_TARGET_TRIPLE={target}");

    // rerun-if-changed hygiene: recompute only when the lock graph or the schema
    // const changes (the fingerprint stays correct without over-rebuilding). The
    // resolved rustc / target are picked up on every build invocation anyway.
    println!("cargo:rerun-if-changed=Cargo.lock");
    println!("cargo:rerun-if-changed=src/domain/dsl/schema_version_const.rs");
}
