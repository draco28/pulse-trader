// Pure tests over the route table (r1.s1.w6, spec step 7's "pure" layer).
//
// These exercise `routes.ts` in isolation -- no DOM, no React. They assert
// fragment normalization, route resolution including the no-`element` path,
// the derived unbuilt set, and the `path === "/" + nav` convention over the
// REAL `ROUTES` table (not a fixture), so a round-4 append that breaks the
// convention fails here rather than only at runtime.

import { describe, expect, it } from "vitest";

import { DEFAULT_NAV_ID, ROUTES, isNavBuilt, resolveNavId, resolveRoute } from "./routes";

const KNOWN_NAV_IDS = ["library", "designer", "backtest", "deploy", "journal", "analytics", "settings", "help"];

describe("resolveNavId (fragment normalization)", () => {
  it("normalizes an empty fragment to the default nav", () => {
    expect(resolveNavId("", KNOWN_NAV_IDS)).toBe(DEFAULT_NAV_ID);
  });

  it("normalizes a bare '#/' to the default nav", () => {
    expect(resolveNavId("#/", KNOWN_NAV_IDS)).toBe(DEFAULT_NAV_ID);
  });

  it("normalizes an unrecognized fragment to the default nav", () => {
    expect(resolveNavId("#/not-a-real-destination", KNOWN_NAV_IDS)).toBe(DEFAULT_NAV_ID);
  });

  it("keeps a recognized, non-default nav id as-is", () => {
    expect(resolveNavId("#/settings", KNOWN_NAV_IDS)).toBe("settings");
  });

  it("is insensitive to a leading '#' being absent", () => {
    expect(resolveNavId("/settings", KNOWN_NAV_IDS)).toBe("settings");
  });
});

describe("resolveRoute (including the no-element path)", () => {
  it("resolves the '/' entry, which has no element", () => {
    const route = resolveRoute("/");
    expect(route).toBeDefined();
    expect(route?.element).toBeUndefined();
  });

  it("returns undefined for a path with no matching entry at all", () => {
    expect(resolveRoute("/library")).toBeUndefined();
  });
});

describe("isNavBuilt (the derived unbuilt set)", () => {
  // r1.s1.w4: the designer row went live (its ROUTES entry carries an
  // `element`), so the derived set changed with it -- which is G8 working as
  // designed: unbuilt-ness is derived from the table, never pre-seeded, and
  // this test tracks whatever the table now says.
  it("is true for the designer and false for every other nav id", () => {
    expect(isNavBuilt("designer")).toBe(true);
    for (const navId of KNOWN_NAV_IDS.filter((id) => id !== "designer")) {
      expect(isNavBuilt(navId)).toBe(false);
    }
  });
});

describe("the path === '/' + nav convention, asserted over the real ROUTES", () => {
  it("holds for every entry that declares a nav", () => {
    for (const route of ROUTES) {
      if (route.nav !== undefined) {
        expect(route.path).toBe("/" + route.nav);
      }
    }
  });
});
