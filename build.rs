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
//! **r1.s1.w1 (ADR-0020) additions.** The script also (a) guarantees the frontend
//! `dist` directory exists before `tauri_build::build()` runs, and (b) runs
//! `tauri_build::build()` itself. See `ensure_frontend_dist` for why (a) is here and
//! not left to npm.
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
    println!("cargo:rerun-if-env-changed=PULSE_ALLOW_PLACEHOLDER_DIST");

    // r1.s1.w1 (ADR-0020): the desktop half. Order matters -- the dist directory must
    // exist before `tauri_build::build()` reads the config that points at it.
    ensure_frontend_dist(Path::new(&manifest_dir));
    tauri_build::build();
}

/// Guarantee `ui/dist/index.html` exists before the Tauri build script reads the config.
///
/// **Why this is in build.rs and not left to npm.** `tauri::generate_context!` embeds the
/// frontend assets at COMPILE time and hard-fails when `frontendDist` is missing. Without
/// this fallback, a bare `cargo test`, `cargo clippy` or a fresh clone would fail to
/// compile until someone had run `npm install && npm run build` -- which would make
/// AC-2/AC-3/AC-4/AC-5 (all plain `cargo test` invocations) depend on a Node toolchain
/// they have nothing to do with, and would break CI's Rust-only job.
///
/// It writes ONLY when the file is absent, so it never clobbers a real Vite build. The
/// placeholder it writes is deliberately inert: `just check` runs the real `npm run
/// build`, which overwrites it. `ui/dist` is gitignored, so this never dirties the tree.
///
/// **The release rule.** `cargo build --release` does NOT go through Tauri's
/// `beforeBuildCommand` (that only runs for `tauri build` / `npm run bundle`), so without a
/// guard this same fallback would silently write the placeholder into a `--release` build
/// too -- and `tauri::generate_context!` would then happily embed it, shipping a binary
/// whose UI is the literal string "Placeholder asset written by build.rs." So when the real
/// `index.html` is absent AND Cargo reports `PROFILE=release`, this panics instead of
/// writing the stub, unless `PULSE_ALLOW_PLACEHOLDER_DIST=1` is set (for a Rust-only release
/// build -- e.g. a CI determinism job -- that is never distributed). Debug/test builds are
/// unaffected: they keep writing the placeholder exactly as before.
fn ensure_frontend_dist(manifest_dir: &Path) {
    let dist = manifest_dir.join("ui").join("dist");
    let index = dist.join("index.html");
    if index.exists() {
        return;
    }
    let is_release = env::var("PROFILE").as_deref() == Ok("release");
    let placeholder_allowed = env::var("PULSE_ALLOW_PLACEHOLDER_DIST").as_deref() == Ok("1");
    assert!(
        !is_release || placeholder_allowed,
        "ui/dist/index.html is missing and this is a --release build: refusing to embed \
         the placeholder frontend into a release binary.\n\
         Fix by either:\n\
         \x20 1. Building the real frontend first: run `npm run build` (or `just bundle` / \
         `npm run bundle`, which runs it automatically via Tauri's beforeBuildCommand), or\n\
         \x20 2. Setting PULSE_ALLOW_PLACEHOLDER_DIST=1 if this is a Rust-only release \
         build that will never be distributed (e.g. a CI determinism job)."
    );
    std::fs::create_dir_all(&dist)
        .unwrap_or_else(|e| panic!("failed to create {}: {e}", dist.display()));
    std::fs::write(
        &index,
        "<!doctype html>\n<html><head><meta charset=\"utf-8\">\
         <title>PulseTrader</title></head>\n<body>\n\
         <p>Placeholder asset written by build.rs. Run `npm run build` for the real bundle.</p>\n\
         </body></html>\n",
    )
    .unwrap_or_else(|e| panic!("failed to write {}: {e}", index.display()));
}
