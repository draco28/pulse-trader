// Focused unit tests over the compose-run state machine (r1.s1.w4, dispatch 2).
//
// The rendered tests in `DesignerScreen.test.tsx` exercise this reduction
// through the whole screen; these drive `useComposeRun` directly so the
// event-to-step fold is asserted at the state it actually owns — including the
// repeated-tool-name case the rendered layer cannot isolate (two `add_filter`
// calls in one run must each keep their own outcome).
//
// Same discipline as the screen tests: `../bindings` is mocked so no IPC
// bridge is needed, events arrive through the REAL `Channel` the hook
// constructed, and the pending invoke resolves under the test's control.

import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Channel } from "@tauri-apps/api/core";

import type { BusEvent } from "../bindings";

const composeStrategyMock = vi.fn();

vi.mock("../bindings", () => ({
  commands: {
    composeStrategy: (...args: unknown[]) => composeStrategyMock(...args),
  },
}));

import { useComposeRun } from "./useComposeRun";

/** The hook's current snapshot, for helper signatures. */
type Hook = ReturnType<typeof useComposeRun>;

/** One streamed event, at `seq`, on the run under test. */
function event(seq: number, payload: BusEvent["payload"]): BusEvent {
  return { runId: "run-1", seq, payload };
}

/** A finalize return value carrying the fields the summary state keeps. */
function finalizeResult() {
  return {
    runId: "run-1",
    emitted: 3,
    cancelled: false,
    strategy: {
      strategyId: "strat-1",
      strategyName: "RSI Oversold",
      versionId: "ver-1",
      createdBy: "composer_llm",
      llmCallCount: 6,
      dsl: {
        direction: "long",
        entry: "rsi(14) < 30",
        filters: ["close > ema(200)"],
        exits: ["stop_loss 5%"],
        risk: ["risk_per_trade 1%"],
      },
    },
  };
}

/** Type a target and send it, as the screen's input area does. */
function submitTarget(result: { current: Hook }, target: string) {
  act(() => {
    result.current.setTarget(target);
  });
  act(() => {
    result.current.submit();
  });
}

/** The newest agent turn — the run in flight. */
function lastTurn(result: { current: Hook }) {
  const last = result.current.messages[result.current.messages.length - 1];
  if (last === undefined || last.kind !== "agent") {
    throw new Error("no agent turn is open");
  }
  return last.turn;
}

// The pending invoke's resolver, captured per test by the mock implementation.
let resolveInvoke: (value: unknown) => void = () => {};

beforeEach(() => {
  composeStrategyMock.mockReset();
  composeStrategyMock.mockImplementation(
    () =>
      new Promise((resolve) => {
        resolveInvoke = resolve;
      }),
  );
});

describe("useComposeRun", () => {
  it("submitting a target invokes the command with it and opens a streaming turn", () => {
    const { result } = renderHook(() => useComposeRun());
    expect(result.current.messages).toHaveLength(0);

    submitTarget(result, "RSI oversold bounce on BTC");

    expect(composeStrategyMock).toHaveBeenCalledTimes(1);
    expect(composeStrategyMock.mock.calls[0][0]).toBe("RSI oversold bounce on BTC");
    // The input is cleared for the next message; the run is the busy state.
    expect(result.current.target).toBe("");
    expect(result.current.running).toBe(true);
    expect(result.current.messages[0]).toMatchObject({
      kind: "user",
      text: "RSI oversold bounce on BTC",
    });
    expect(result.current.messages[1]).toMatchObject({
      kind: "agent",
      turn: { status: "streaming", steps: [] },
    });
  });

  it("submitting while a run streams is a no-op — one run at a time", () => {
    const { result } = renderHook(() => useComposeRun());
    submitTarget(result, "first");
    submitTarget(result, "second");

    expect(composeStrategyMock).toHaveBeenCalledTimes(1);
    // Only the first run's user bubble + agent turn exist.
    expect(result.current.messages).toHaveLength(2);
  });

  it("folds streamed events into the running turn one by one", async () => {
    const { result } = renderHook(() => useComposeRun());
    submitTarget(result, "t");
    const channel = composeStrategyMock.mock.calls[0][1] as Channel<BusEvent>;

    // First step opens the moment its ToolCallStarted arrives.
    await act(async () => {
      channel.onmessage?.(
        event(1, {
          kind: "toolCallStarted",
          name: "add_entry_signal",
          argumentsPreview: "rsi(14) < 30",
        }),
      );
    });
    expect(lastTurn(result).steps).toEqual([
      { name: "add_entry_signal", preview: "rsi(14) < 30", outcome: undefined },
    ]);

    // Its result closes the same step.
    await act(async () => {
      channel.onmessage?.(
        event(2, { kind: "toolCallResult", name: "add_entry_signal", outcome: "entry signal added" }),
      );
    });
    expect(lastTurn(result).steps[0]?.outcome).toBe("entry signal added");

    // A repeated tool name opens a SECOND step of that name; each result must
    // attach to the LAST open one, leaving the earlier step untouched.
    await act(async () => {
      channel.onmessage?.(
        event(3, { kind: "toolCallStarted", name: "add_filter", argumentsPreview: "close > ema(200)" }),
      );
    });
    await act(async () => {
      channel.onmessage?.(
        event(4, { kind: "toolCallStarted", name: "add_filter", argumentsPreview: "volume > ma(20)" }),
      );
    });
    expect(lastTurn(result).steps.map((step) => step.preview)).toEqual([
      "rsi(14) < 30",
      "close > ema(200)",
      "volume > ma(20)",
    ]);

    await act(async () => {
      channel.onmessage?.(
        event(5, { kind: "toolCallResult", name: "add_filter", outcome: "filter added (volume)" }),
      );
    });
    expect(lastTurn(result).steps[2]?.outcome).toBe("filter added (volume)");
    expect(lastTurn(result).steps[1]?.outcome).toBeUndefined();

    await act(async () => {
      channel.onmessage?.(
        event(6, { kind: "toolCallResult", name: "add_filter", outcome: "filter added (trend)" }),
      );
    });
    expect(lastTurn(result).steps[1]?.outcome).toBe("filter added (trend)");
  });

  it("a finalized run keeps the returned summary and clears the running state", async () => {
    const { result } = renderHook(() => useComposeRun());
    submitTarget(result, "t");
    const channel = composeStrategyMock.mock.calls[0][1] as Channel<BusEvent>;

    await act(async () => {
      channel.onmessage?.(
        event(1, {
          kind: "toolCallStarted",
          name: "add_entry_signal",
          argumentsPreview: "rsi(14) < 30",
        }),
      );
    });
    await act(async () => {
      resolveInvoke({ status: "ok", data: finalizeResult() });
    });

    const turn = lastTurn(result);
    expect(turn.status).toBe("finalized");
    expect(turn.summary?.versionId).toBe("ver-1");
    expect(result.current.running).toBe(false);
  });

  it("a cancelled run ends cancelled with no summary — a normal ending, not an error", async () => {
    const { result } = renderHook(() => useComposeRun());
    submitTarget(result, "t");

    await act(async () => {
      resolveInvoke({ status: "ok", data: { runId: "run-1", emitted: 1, cancelled: true, strategy: null } });
    });

    const turn = lastTurn(result);
    expect(turn.status).toBe("cancelled");
    expect(turn.summary).toBeUndefined();
    expect(turn.error).toBeUndefined();
    expect(result.current.running).toBe(false);
  });

  it("a rejected command records its message on the turn — no silent failure", async () => {
    const { result } = renderHook(() => useComposeRun());
    submitTarget(result, "t");

    await act(async () => {
      resolveInvoke({
        status: "error",
        error: {
          code: "llm",
          message: "no usable LLM credential found (searched: env, config dir, .env, app data dir)",
        },
      });
    });

    const turn = lastTurn(result);
    expect(turn.status).toBe("error");
    expect(turn.error).toContain("no usable LLM credential found");
    expect(result.current.running).toBe(false);
  });
});
