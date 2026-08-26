// The honest "not built yet" pane (r1.s1.w6, spec step 4, G6).
//
// A nav row whose destination has no matching `ROUTES` element lands here
// instead of a blank pane or an invented one. It names the destination and
// says plainly that it is not built -- no sample rows, no invented metrics,
// no screen-shaped skeleton (`r1.s1` SPINE.md's fake ledger; see this
// module's own citation in the spec's "Fakes" section).
//
// It is the worked example of this codebase's screen convention: one file
// under `ui/src/screens/`, default export, a component that takes NO props
// -- so, rather than being threaded a title through a parent-supplied prop,
// it derives its own destination the same way `App.tsx` resolves the active
// route: from `location.hash`, through the exact same pure helpers
// (`resolveNavId` + `NAV_ALL`). Both call sites read the same ambient truth
// and can never disagree, and every future screen assigned to
// `Route.element` (`() => ReactNode`) can be written to the same zero-arg
// shape this file demonstrates.

import { NAV_ALL } from "../shell/AppShell";
import { resolveNavId } from "../routes";

export default function UnbuiltScreen() {
  const navId = resolveNavId(
    window.location.hash,
    NAV_ALL.map((item) => item.id),
  );
  const label = NAV_ALL.find((item) => item.id === navId)?.label ?? "This screen";

  return (
    <div className="unbuilt">
      <h1 className="unbuilt-title">{label}</h1>
      <p className="unbuilt-body">Not built yet.</p>
    </div>
  );
}
