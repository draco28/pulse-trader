// Vite config for the PulseTrader desktop frontend (ADR-0020).
//
// `root` is `ui/`, so the HTML entry point and the TypeScript sources live beside each
// other and away from the Rust tree. `outDir` resolves to `ui/dist`, which is what
// `tauri.conf.json`'s `frontendDist` points at and what `build.rs` writes a placeholder
// into when it is absent (so a Rust-only `cargo test` still compiles).
//
// `emptyOutDir` is on: a stale asset from a previous build must not survive into the
// bundle, because `generate_context!` embeds whatever is in that directory.
import { defineConfig } from "vite";

export default defineConfig({
  root: "ui",
  build: {
    outDir: "dist",
    emptyOutDir: true,
    target: "safari15",
    // The window is a fixed 1440x900 desktop shell, not a page served over a network.
    // Sourcemaps cost nothing at that size and make a WKWebView-only bug debuggable --
    // which matters given ADR-0020's recorded "WKWebView is not Chromium" risk.
    sourcemap: true,
  },
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
  },
});
