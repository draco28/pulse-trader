// Vite config for the PulseTrader desktop frontend (ADR-0020).
//
// `root` is `ui/`, so the HTML entry point and the TypeScript sources live beside each
// other and away from the Rust tree. `outDir` resolves to `ui/dist`, which is what
// `tauri.conf.json`'s `frontendDist` points at and what `build.rs` writes a placeholder
// into when it is absent (so a Rust-only `cargo test` still compiles).
//
// `emptyOutDir` is on: a stale asset from a previous build must not survive into the
// bundle, because `generate_context!` embeds whatever is in that directory.
//
// `@vitejs/plugin-react` (r1.s1.w5, ADR-0020 step 1) is the JSX/Fast-Refresh transform
// for the ported design system -- pinned to the `^5.x` line because `^6.x` requires
// Vite 8, and this item ports the design system into the existing Vite 5 toolchain
// rather than bumping it (an unrelated, larger change this item does not make).
//
// `defineConfig` comes from `vitest/config` rather than `vite` (r1.s1.w6, G9): it
// re-exports Vite's own `defineConfig` with the `test` key merged into the type, so
// this stays ONE config file/format for both `vite build` and `vitest run` rather than
// a second config format bolted on beside it. `test.environment` is `jsdom` (not the
// default `node`) because the rendered layer's assertions (exactly one nav row is
// `.is-active`, no row is a dead link) need a DOM to query.
import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

export default defineConfig({
  root: "ui",
  plugins: [react()],
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
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.{ts,tsx}"],
    // No `globals: true` -- `describe`/`it`/`expect` are imported explicitly per file,
    // matching this codebase's no-implicit-globals style everywhere else.
    // No `restoreMocks` -- it resets every `vi.fn()` (including ones a
    // `vi.mock()` factory or a setup file establishes once) before EACH
    // test, which silently empties a `mockResolvedValue`/`mockImplementation`
    // set up outside a `beforeEach`. Tests that need per-test isolation
    // reset their own mocks explicitly.
    setupFiles: ["src/test/setup.ts"],
  },
});
