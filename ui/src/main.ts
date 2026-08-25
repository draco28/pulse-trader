// The placeholder frontend entry point (r1.s1.w1, ADR-0020 step 8).
//
// DELIBERATELY UNSTYLED AND DELIBERATELY THROWAWAY. `r1.s1.w5` replaces this file
// wholesale with the ported design system. Its only job is to prove, by running, that
// the three halves of the command bus work end to end through the GENERATED bindings:
//
//   1. a round-trip command returns typed data       -> commands.shellInfo()
//   2. a domain error crosses as ONE renderable shape -> commands.busSelftestFailure()
//   3. a run streams over a PER-INVOCATION channel    -> commands.startDemoStream()
//
// Everything here goes through `./bindings`, never a raw `invoke("...")` with a string
// command name. That is the point of the typed seam: renaming a command or changing a
// payload is a TypeScript error at build time, not a runtime `undefined`.

import { Channel } from "@tauri-apps/api/core";

import { commands, type BusError, type BusEvent, type ShellInfo } from "./bindings";
import { ROUTES } from "./routes";

/** Look up an element that the HTML guarantees exists, and say so loudly if it does not. */
function element(id: string): HTMLElement {
  const found = document.getElementById(id);
  if (found === null) {
    throw new Error(`placeholder page is missing #${id}`);
  }
  return found;
}

/** Render a BusError the way every screen should: code first, then the message. */
function renderBusError(error: BusError): string {
  return `[${error.code}] ${error.message}`;
}

async function renderShellInfo(): Promise<void> {
  const target = element("shell-info");
  const result = await commands.shellInfo();
  if (result.status === "error") {
    target.textContent = `FAILED: ${renderBusError(result.error)}`;
    return;
  }
  const info: ShellInfo = result.data;
  target.textContent = [
    `app version:        ${info.appVersion}`,
    `engine fingerprint: ${info.engineFingerprint}`,
    `target triple:      ${info.targetTriple}`,
    `strategies:         ${info.strategyCount}`,
  ].join("\n");
}

async function renderDeliberateFailure(): Promise<void> {
  const target = element("bus-error");
  const result = await commands.busSelftestFailure();
  if (result.status === "error") {
    // The EXPECTED branch: this command always fails, so the error path is exercised
    // by anyone who opens the app rather than only when something is broken.
    target.textContent = `as expected, one renderable error shape -> ${renderBusError(result.error)}`;
    return;
  }
  target.textContent = "UNEXPECTED: the deliberate-failure command succeeded";
}

async function runStream(): Promise<void> {
  const target = element("stream-log");
  const lines: string[] = [];

  // A FRESH channel per invocation. This is the correlation mechanism (grill A2): a
  // second run gets a second channel, so its events cannot arrive here. Nothing
  // filters by run id, because there is no shared bus to filter.
  const channel = new Channel<BusEvent>();
  channel.onmessage = (event: BusEvent) => {
    const detail = event.payload.kind === "started" ? "" : ` — ${event.payload.message}`;
    lines.push(`#${event.seq} ${event.payload.kind}${detail}  (run ${event.runId})`);
    target.textContent = lines.join("\n");
  };

  target.textContent = "running…";
  const result = await commands.startDemoStream(6, channel);
  if (result.status === "error") {
    target.textContent = `${lines.join("\n")}\nFAILED: ${renderBusError(result.error)}`;
    return;
  }
  const outcome = result.data;
  lines.push(
    `done: emitted=${outcome.emitted} cancelled=${outcome.cancelled} run=${outcome.runId}`,
  );
  target.textContent = lines.join("\n");
}

function renderRoutes(): void {
  element("routes").textContent = ROUTES.map((r) => `${r.path}  ${r.title}`).join("\n");
}

function main(): void {
  renderRoutes();
  element("start-stream").addEventListener("click", () => {
    void runStream();
  });
  void renderShellInfo();
  void renderDeliberateFailure();
}

main();
