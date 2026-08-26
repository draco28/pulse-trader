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
