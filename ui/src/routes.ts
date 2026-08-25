// The route table (ADR-0020, bus contract clause 4) — the frontend half of the
// append-only registration contract.
//
// ONE table, ONE line per screen. Adding a screen means appending one entry here and
// one entry to `BUS_COMMANDS` in `src/tauri/commands.rs`. Nothing else.
//
// Why this shape is load-bearing rather than tidy: `r1.s1.w3` and `r1.s1.w4` run in
// PARALLEL in round 3, and each adds one screen. With one append-only table they
// conflict TEXTUALLY — two adjacent added entries, resolved by keeping both — and never
// SEMANTICALLY. A nested router, a per-screen registration call, or two tables would
// re-create the dependency edge the DAG was authored without. The round-3 plan dropped
// that edge on this property; weakening it is not a refactor, it is a replan.
//
// `tests/tauri_bus_contract.rs::the_route_table_is_one_append_only_entry_per_screen`
// asserts the shape: exactly one `ROUTES` table, one `path:` per line, no duplicates.
//
// r1.s1.w5 replaces the placeholder screen's rendering wholesale. It must keep the
// TABLE, not just the file.
//
// r1.s1.w5 retitles the one entry below (the placeholder page it replaces is gone,
// so "Shell placeholder" no longer describes anything) without adding a row -- w5
// mounts no product screen, so there is still exactly one entry. w3/w4 append
// theirs in round 3.

/** One screen in the shell. */
export interface Route {
  /** URL fragment that selects this screen. Unique across the table. */
  readonly path: string;
  /** Human-readable title, rendered in the app's own titlebar (w5). */
  readonly title: string;
}

export const ROUTES: readonly Route[] = [
  {
    path: "/",
    title: "PulseTrader",
  },
];

/** Resolve a fragment to a route, or `undefined` when nothing matches. */
export function resolveRoute(fragment: string): Route | undefined {
  return ROUTES.find((route) => route.path === fragment);
}
