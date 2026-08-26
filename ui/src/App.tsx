// The app's root composition. r1.s1.w5 built the chrome + sidebar with the
// no-credential banner as the pane's sole child; r1.s1.w6 makes the shell
// navigable (G5-G9): `location.hash` drives which nav row is active and what
// the content pane renders, through the table-driven router in `routes.ts`.
//
// No product screen mounts here -- the Library is `w3`, the Designer is `w4`.
// Every nav row lands on either a real route's `element` (none exist yet) or
// the honest `UnbuiltScreen` -- never a screen-shaped placeholder, which is
// exactly the fake `r1.s1` SPINE.md's ledger exists to catch.

import { useEffect, useState } from "react";
import type { ReactNode } from "react";

import { CredentialBanner } from "./components/CredentialBanner";
import { NAV_ALL, Sidebar, WindowChrome } from "./shell/AppShell";
import { resolveNavId, resolveRoute } from "./routes";
import type { Route } from "./routes";
import UnbuiltScreen from "./screens/UnbuiltScreen";

const KNOWN_NAV_IDS = NAV_ALL.map((item) => item.id);

/**
 * Given a resolved route (or none), render its screen or the unbuilt pane.
 * Deliberately independent of `location.hash` / nav-id resolution, so it is
 * testable with a synthetic `Route` regardless of what `ROUTES` currently
 * contains (r1.s1.w6, spec step 7's rendered layer).
 */
export function RouteContent({ route }: { route: Route | undefined }): ReactNode {
  if (route?.element !== undefined) {
    const Element = route.element;
    return <Element />;
  }
  return <UnbuiltScreen />;
}

export function App() {
  const [navId, setNavId] = useState<string>(() =>
    resolveNavId(window.location.hash, KNOWN_NAV_IDS),
  );

  useEffect(() => {
    const onHashChange = () => setNavId(resolveNavId(window.location.hash, KNOWN_NAV_IDS));
    window.addEventListener("hashchange", onHashChange);
    return () => window.removeEventListener("hashchange", onHashChange);
  }, []);

  const route = resolveRoute("/" + navId);
  const navEntry = NAV_ALL.find((item) => item.id === navId);
  const title = route?.title ?? navEntry?.label;

  // No route landed by this item ever declares a details pane -- the third
  // (360px) `.layout` track has nothing to show. `w3` opts the Library route
  // back into it when it ports `DetailsPane`; until then the shell always
  // collapses the track rather than leaving it an empty hole beside the pane.
  const showDetailsPane = false;

  return (
    <WindowChrome docTitle={title}>
      <div className={`layout${showDetailsPane ? "" : " layout-no-details"}`}>
        <Sidebar active={navId} />
        <main className="content">
          <CredentialBanner />
          <RouteContent route={route} />
        </main>
      </div>
    </WindowChrome>
  );
}
