// The app's root composition (r1.s1.w5): the ported chrome + sidebar, with the
// no-credential banner mounted where a screen will land in round 3.
//
// No product screen mounts here -- the Library is `w3`, the Designer is `w4`. The
// `.layout` grid's third (360px) track is deliberately left empty rather than
// filled with placeholder content: a screen-shaped placeholder that looks like a
// feature is exactly the fake `r1.s1` SPINE.md's ledger exists to catch.

import { CredentialBanner } from "./components/CredentialBanner";
import { Sidebar, WindowChrome } from "./shell/AppShell";

export function App() {
  return (
    <WindowChrome>
      <div className="layout">
        <Sidebar />
        <main className="content">
          <CredentialBanner />
        </main>
      </div>
    </WindowChrome>
  );
}
