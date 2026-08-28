// Test environment setup (r1.s1.w6), run once before every test file
// (`vite.config.ts`'s `test.setupFiles`). Two gaps closed here rather than
// per test file, since every rendered test hits both:

import { afterEach } from "vitest";
import { cleanup } from "@testing-library/react";

// 1. `jsdom` does not implement `window.matchMedia`, and `AppShell.tsx`'s
//    `useTheme` calls it on every `WindowChrome` mount (it existed before
//    this item -- w5 -- and is unrelated to navigation).
//
//    A plain function, deliberately NOT a `vi.fn()` -- if `vite.config.ts`
//    ever grows a `restoreMocks: true`, that resets every `vi.fn()` before
//    EACH test (including ones a `setupFiles` module or a `vi.mock()`
//    factory establishes once), which would silently empty this
//    implementation right back to a no-op before the first test ran.
if (typeof window.matchMedia !== "function") {
  window.matchMedia = (query: string) =>
    ({
      matches: false,
      media: query,
      onchange: null,
      addListener: () => {},
      removeListener: () => {},
      addEventListener: () => {},
      removeEventListener: () => {},
      dispatchEvent: () => false,
    }) as MediaQueryList;
}

// 2. `@testing-library/react`'s automatic post-test `cleanup()` only
//    self-registers when it detects Vitest's `globals: true`. This project
//    does not set that (explicit imports everywhere, matching its
//    no-implicit-globals style), so without this, an un-unmounted tree from
//    one test stays in `document.body` and pollutes the next test's DOM
//    queries within the same file (two "Settings" links instead of one).
afterEach(() => {
  cleanup();
});

// 3. `@tauri-apps/api/core`'s `Channel` registers its message callback with
//    `window.__TAURI_INTERNALS__.transformCallback` at CONSTRUCTION — a bridge
//    that only exists inside a real Tauri webview. A screen that opens a
//    per-invocation channel (r1.s1.w4's Designer) therefore cannot even
//    construct one under jsdom without this stub. The registered callback is
//    never driven from here (tests mock the bindings module, and drive the
//    channel's `onmessage` directly); the returned id just has to be a unique
//    number per channel, exactly as in the webview.
declare global {
  interface Window {
    __TAURI_INTERNALS__?: {
      transformCallback: (callback: (response: unknown) => void, once?: boolean) => number;
    };
  }
}

if (window.__TAURI_INTERNALS__ === undefined) {
  window.__TAURI_INTERNALS__ = {
    transformCallback: () => Math.floor(Math.random() * 2 ** 31),
  };
}
