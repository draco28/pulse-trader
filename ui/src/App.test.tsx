// Rendered tests over the shell (r1.s1.w6, spec step 7's layer, finding C6).
//
// These are the PRIMARY evidence for round-3 requirements 1 and 2 -- exactly
// one nav row is `.is-active`, and no row is a dead link -- both rendering
// facts a pure-function suite could not assert. `check-shell-navigation.sh`
// (AC-2) is a backstop against these being deleted, not the proof itself.
//
// `commands.credentialStatus` is mocked so the credential banner (w5) has a
// deterministic, real (non-"none") DOM to assert against without depending on
// a live Tauri IPC bridge, which does not exist under `jsdom`.

import { render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("./bindings", () => ({
  commands: {
    credentialStatus: vi.fn().mockResolvedValue("none"),
    // r1.s1.w3: the default landing is now the Library screen, which reads
    // `libraryOverview` on mount — mocked to an empty payload so these shell
    // tests keep asserting the shell (the screen's own behaviour lives in
    // `screens/LibraryScreen.test.tsx`).
    libraryOverview: vi.fn().mockResolvedValue({ status: "ok", data: { strategies: [] } }),
  },
}));

import { App } from "./App";
import { RouteContent } from "./App";
import type { Route } from "./routes";

function setHash(hash: string) {
  window.location.hash = hash;
}

describe("<App /> shell navigation", () => {
  beforeEach(() => {
    setHash("");
  });

  afterEach(() => {
    setHash("");
  });

  it("opens on Strategy Library rather than a blank pane", async () => {
    render(<App />);
    const activeRows = await screen.findAllByRole("link", { name: /strategy library/i });
    expect(activeRows).toHaveLength(1);
    expect(activeRows[0].className).toContain("is-active");
  });

  it("marks exactly one nav row active, and it is the one the fragment selects", async () => {
    setHash("#/settings");
    render(<App />);
    const settingsRow = await screen.findByRole("link", { name: /settings/i });
    expect(settingsRow.className).toContain("is-active");

    const nav = settingsRow.closest("nav");
    expect(nav).not.toBeNull();
    // Every OTHER row in the same nav list must not be active.
    const others = within(nav as HTMLElement)
      .getAllByRole("link")
      .filter((row) => row !== settingsRow);
    for (const row of others) {
      expect(row.className).not.toContain("is-active");
    }
  });

  it("gives every nav row a real fragment href -- none is a dead '#' link", async () => {
    render(<App />);
    const rows = await screen.findAllByRole("link");
    expect(rows.length).toBeGreaterThan(0);
    for (const row of rows) {
      const href = row.getAttribute("href");
      expect(href).not.toBe("#");
      expect(href).toMatch(/^#\//);
    }
  });

  // r1.s3.w4: the Backtest Lab's ROUTES entry makes the existing nav row live
  // through `isNavBuilt` alone — the "Soon" badge disappears with no nav-side
  // edit, while still-unbuilt rows keep theirs (the flip is a derivation, not
  // a blanket badge removal).
  it("shows the Backtest Lab nav row live once its ROUTES entry lands", async () => {
    render(<App />);
    const backtestRow = await screen.findByRole("link", { name: /backtest lab/i });
    expect(within(backtestRow).queryByText("Soon")).toBeNull();
    const deployRow = screen.getByRole("link", { name: /deployment dashboard/i });
    expect(within(deployRow).getByText("Soon")).toBeTruthy();
  });

  it("keeps the credential banner mounted regardless of the active route", async () => {
    setHash("#/settings");
    render(<App />);
    expect(await screen.findByRole("status")).toBeTruthy();
  });

  it("renders the unbuilt pane, not fabricated content, for a destination with no route", async () => {
    setHash("#/settings");
    render(<App />);
    expect(await screen.findByText(/not built/i)).toBeTruthy();
  });
});

describe("RouteContent (given a resolved route, independent of which nav id is active)", () => {
  it("mounts the route's element when one is declared", () => {
    const route: Route = {
      path: "/x",
      title: "X",
      element: () => <div>Real screen content</div>,
    };
    render(<RouteContent route={route} />);
    expect(screen.getByText("Real screen content")).toBeTruthy();
  });

  it("renders the unbuilt pane when the route has no element", () => {
    render(<RouteContent route={undefined} />);
    expect(screen.getByText(/not built/i)).toBeTruthy();
  });
});
