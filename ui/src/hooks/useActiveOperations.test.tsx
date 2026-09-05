// Focused unit tests over the active-operation store (r1.s4.w3, #141).
//
// The rendered tests in `App.test.tsx` exercise this through a real navigation —
// run, leave, come back, and see the result that landed while the screen was
// unmounted. These drive the store directly, so the two properties it exists for
// are asserted where they live: an in-flight key is REFUSED before the bus is
// called, and a settled result outlives the screen that started it.
//
// The store is deliberately NOT mocked here — it calls whatever `invoke` it is
// given, and these hand it a promise the test resolves under its own control.

import { act, render, renderHook, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { ReactNode } from "react";

import {
  ActiveOperationsProvider,
  backtestKey,
  coachKey,
  useActiveOperations,
} from "./useActiveOperations";
import type { BusResult } from "./useActiveOperations";

/** A promise the test settles by hand, plus its resolver. */
function deferred<T>(): { promise: Promise<T>; resolve: (value: T) => void } {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

function wrapper({ children }: { children: ReactNode }) {
  return <ActiveOperationsProvider>{children}</ActiveOperationsProvider>;
}

describe("useActiveOperations (the #141 store)", () => {
  it("keys backtests by version and coach operations by session, and never collides", () => {
    // The two id spaces are separate: a version id and a session id that happen
    // to share text are two different operations.
    expect(backtestKey("x")).not.toBe(coachKey("x"));
  });

  it("marks a started key running until its promise settles, then holds the result", async () => {
    const { result } = renderHook(() => useActiveOperations(), { wrapper });
    const gate = deferred<BusResult<string>>();

    act(() => {
      result.current.start(backtestKey("v1"), () => gate.promise);
    });
    expect(result.current.lookup(backtestKey("v1"))?.running).toBe(true);
    expect(result.current.lookup(backtestKey("v1"))?.outcome).toBeUndefined();

    await act(async () => {
      gate.resolve({ status: "ok", data: "landed" });
      await gate.promise;
    });

    const record = result.current.lookup(backtestKey("v1"));
    expect(record?.running).toBe(false);
    expect(record?.outcome).toEqual({ status: "ok", data: "landed" });
  });

  it("refuses a second start for the same key BEFORE the invocation is made", async () => {
    const { result } = renderHook(() => useActiveOperations(), { wrapper });
    const gate = deferred<BusResult<string>>();
    const invoke = vi.fn(() => gate.promise);

    act(() => {
      result.current.start(backtestKey("v1"), invoke);
      // The same key again, in the SAME tick — the double-click that beats a
      // re-render, which a `running` flag held in rendered state would miss.
      result.current.start(backtestKey("v1"), invoke);
    });

    expect(invoke).toHaveBeenCalledTimes(1);

    await act(async () => {
      gate.resolve({ status: "ok", data: "one" });
      await gate.promise;
    });
    // Released, the same key starts again — the latch refuses overlap, not the
    // operation.
    act(() => {
      result.current.start(backtestKey("v1"), invoke);
    });
    expect(invoke).toHaveBeenCalledTimes(2);
  });

  it("lets a DIFFERENT key run concurrently", () => {
    const { result } = renderHook(() => useActiveOperations(), { wrapper });
    const a = vi.fn(() => deferred<BusResult<string>>().promise);
    const b = vi.fn(() => deferred<BusResult<string>>().promise);

    act(() => {
      result.current.start(backtestKey("v1"), a);
      result.current.start(backtestKey("v2"), b);
    });

    expect(a).toHaveBeenCalledTimes(1);
    expect(b).toHaveBeenCalledTimes(1);
  });

  it("records a rejected invocation as a BusError rather than losing it", async () => {
    const { result } = renderHook(() => useActiveOperations(), { wrapper });

    await act(async () => {
      result.current.start(backtestKey("v1"), () =>
        Promise.reject(new Error("the bridge is gone")),
      );
      await Promise.resolve();
    });

    const record = result.current.lookup(backtestKey("v1"));
    expect(record?.running).toBe(false);
    expect(record?.outcome?.status).toBe("error");
    if (record?.outcome?.status === "error") {
      expect(record.outcome.error.message).toContain("the bridge is gone");
    }
  });

  it("keeps a settled result after the CONSUMER unmounts, and hands it back on remount", async () => {
    // The whole of #141 in one assertion: the provider lives above the route, so
    // the record survives the screen that started it.
    const gate = deferred<BusResult<string>>();

    function Consumer() {
      const operations = useActiveOperations();
      const record = operations.lookup(backtestKey("v1"));
      return (
        <div>
          <button type="button" onClick={() => operations.start(backtestKey("v1"), () => gate.promise)}>
            run
          </button>
          <span data-testid="state">
            {record === undefined
              ? "idle"
              : record.running
                ? "running"
                : JSON.stringify(record.outcome)}
          </span>
        </div>
      );
    }

    function Shell({ mounted }: { mounted: boolean }) {
      return (
        <ActiveOperationsProvider>{mounted ? <Consumer /> : <p>elsewhere</p>}</ActiveOperationsProvider>
      );
    }

    const view = render(<Shell mounted={true} />);
    act(() => {
      screen.getByRole("button", { name: "run" }).click();
    });
    expect(screen.getByTestId("state").textContent).toBe("running");

    // Navigate away — the consumer unmounts, the provider does not.
    view.rerender(<Shell mounted={false} />);
    await act(async () => {
      gate.resolve({ status: "ok", data: "landed while away" });
      await gate.promise;
    });

    // Come back: the result is here, and nothing was invoked a second time.
    view.rerender(<Shell mounted={true} />);
    expect(screen.getByTestId("state").textContent).toContain("landed while away");
  });

  it("clears one key without disturbing another", async () => {
    const { result } = renderHook(() => useActiveOperations(), { wrapper });
    const a = deferred<BusResult<string>>();
    const b = deferred<BusResult<string>>();

    await act(async () => {
      result.current.start(backtestKey("v1"), () => a.promise);
      result.current.start(backtestKey("v2"), () => b.promise);
      a.resolve({ status: "ok", data: "a" });
      b.resolve({ status: "ok", data: "b" });
      await Promise.all([a.promise, b.promise]);
    });

    act(() => {
      result.current.clear(backtestKey("v1"));
    });

    expect(result.current.lookup(backtestKey("v1"))).toBeUndefined();
    expect(result.current.lookup(backtestKey("v2"))?.outcome).toEqual({
      status: "ok",
      data: "b",
    });
  });
});
