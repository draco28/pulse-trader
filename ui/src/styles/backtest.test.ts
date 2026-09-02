// The Backtest Lab stylesheet's pinned-content suite (r1.s3.w4, AC-2) —
// authored BEFORE `backtest.css` exists and driven RED.
//
// The spec's palette values were selected by the dataviz validator at planning
// time (spec "Validator evidence"); this suite is the durable repo-local gate
// that keeps that evidence true: it asserts the shipped CSS variables equal
// the exact hex sets, in their exact theme scopes, so no later edit can
// silently rot the computed contrast/CVD analysis. It also pins the sheet's
// other spec-mandated content: the reduced-motion block, the ≥24px hit-target
// floors, and the print reveal of the chart twins.
//
// Reading the sheet: vitest stubs CSS imports (a `?raw` import yields an
// empty string), so the test reads the file through Node's runtime builtin
// instead. This toolchain deliberately ships no `@types/node` (see
// package.json), so the one cast below is the whole type story — no ambient
// declarations, no config change.

/** The minimum `fs` surface this suite needs. */
interface FsLike {
  readFileSync(path: string, encoding: "utf8"): string;
}

/** The Node runtime globals vitest always runs on, reached through
 * `globalThis` because the `process` name itself is untyped here. */
const nodeRuntime = (globalThis as unknown as {
  process: { cwd(): string; getBuiltinModule(id: string): unknown };
}).process;

/** The stylesheet under test, read from the repo at test time. The path is
 * resolved from the working area vitest runs in (the repo root — the same
 * root every `scripts/check-*.sh` gate resolves). */
const fs = nodeRuntime.getBuiltinModule("node:fs") as FsLike;
const backtestCss: string = fs.readFileSync(
  `${nodeRuntime.cwd()}/ui/src/styles/backtest.css`,
  "utf8",
);

import { describe, expect, it } from "vitest";

/** The body of a `{ ... }` block for a selector, or null when absent. */
function block(selector: string): string | null {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = backtestCss.match(new RegExp(`${escaped}\\s*\\{([^}]*)\\}`));
  return match === null ? null : match[1];
}

/** A `--var: value;` assignment inside a block, or null when absent. */
function cssVar(blockBody: string | null, name: string): string | null {
  if (blockBody === null) return null;
  const match = blockBody.match(new RegExp(`${name}\\s*:\\s*([^;]+);`));
  return match === null ? null : match[1].trim();
}

const DARK_SLOTS = {
  "--bt-c1": "#3987e5",
  "--bt-c2": "#d95926",
  "--bt-c3": "#199e70",
  "--bt-c4": "#c98500",
} as const;

const LIGHT_SLOTS = {
  "--bt-c1": "#2a78d6",
  "--bt-c2": "#eb6834",
  "--bt-c3": "#1baf7a",
  "--bt-c4": "#eda100",
} as const;

describe("backtest.css (the scoped chart palette, pinned)", () => {
  it("defines the four dark categorical slots and both chart surfaces on the screen scope", () => {
    const dark = block(".bt-lab");
    expect(dark).not.toBeNull();
    for (const [name, hex] of Object.entries(DARK_SLOTS)) {
      expect(cssVar(dark, name)).toBe(hex);
    }
    expect(cssVar(dark, "--bt-surface")).toBe("#16181D");
    expect(cssVar(dark, "--bt-surface-raised")).toBe("#1D1F25");
  });

  it("re-declares the dark set under the explicit dark-theme scope", () => {
    const dark = block('[data-theme="dark"] .bt-lab');
    expect(dark).not.toBeNull();
    for (const [name, hex] of Object.entries(DARK_SLOTS)) {
      expect(cssVar(dark, name)).toBe(hex);
    }
  });

  it("overrides every slot and both surfaces under the light-theme scope", () => {
    const light = block('[data-theme="light"] .bt-lab');
    expect(light).not.toBeNull();
    for (const [name, hex] of Object.entries(LIGHT_SLOTS)) {
      expect(cssVar(light, name)).toBe(hex);
    }
    expect(cssVar(light, "--bt-surface")).toBe("#FFFFFF");
    expect(cssVar(light, "--bt-surface-raised")).toBe("#F9FAFB");
  });

  it("scopes the palette to the screen — nothing is declared on :root", () => {
    // A :root block in this sheet would leak the chart palette application-wide.
    expect(block(":root")).toBeNull();
    expect(block(':root, [data-theme="dark"]')).toBeNull();
  });

  it("keeps the palette values out of every other rule — slots are referenced, never re-inlined", () => {
    // Strip comments, then check no rule outside the three scope blocks
    // carries a palette hex literal (a stray literal would drift from the pin).
    const noComments = backtestCss.replace(/\/\*[\s\S]*?\*\//g, "");
    const scopeStart = noComments.indexOf(".bt-lab {");
    const lightStart = noComments.indexOf('[data-theme="light"] .bt-lab');
    const lightEnd = noComments.indexOf("}", lightStart);
    const outside =
      noComments.slice(0, scopeStart) + noComments.slice(lightEnd + 1);
    for (const hex of [...Object.values(DARK_SLOTS), ...Object.values(LIGHT_SLOTS)]) {
      expect(outside).not.toContain(hex);
    }
  });
});

describe("backtest.css (pinned accessibility content)", () => {
  it("disables decorative transitions under prefers-reduced-motion", () => {
    const reduced = block("@media (prefers-reduced-motion: reduce)");
    expect(reduced).not.toBeNull();
    expect(reduced).toMatch(/transition\s*:\s*none/);
    expect(reduced).toMatch(/animation\s*:\s*none/);
  });

  it("carries the ≥24px hit-target floors for columns and regime rows", () => {
    const col = block(".bt-col");
    expect(col).not.toBeNull();
    expect(col).toMatch(/min-width\s*:\s*24px/);
    const row = block(".bt-regime-row");
    expect(row).not.toBeNull();
    expect(row).toMatch(/min-height\s*:\s*24px/);
  });

  it("reveals the table twins in print — the graphic is never the only carrier", () => {
    const print = block("@media print");
    expect(print).not.toBeNull();
    expect(print).toContain(".bt-twin");
    expect(print).toMatch(/position\s*:\s*static/);
  });
});
