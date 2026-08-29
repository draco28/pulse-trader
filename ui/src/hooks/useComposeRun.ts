// The compose-run state machine (r1.s1.w4) — the channel wiring, the
// event-to-step reduction, and the running / finalize / error state behind the
// Strategy Designer screen. Extracted from `DesignerScreen.tsx` (dispatch 2)
// so the streaming logic is unit-testable directly and the screen is markup
// plus wiring.
//
// One channel per invocation (grill A2): the channel IS the correlation
// between a submitted target and its agent turn. Each tool call opens a step
// the moment its `toolCallStarted` arrives and its outcome appends when the
// matching `toolCallResult` does — that one-by-one fold is `d1`'s observable.
// The finalize summary comes from the command's RETURN value, not the event
// channel; `started` / `finished` carry no step-list mutation of their own.
//
// The channel alone is NOT enough correlation, which is why `inFlight` below
// exists. Three things it fixes, all of them cases where the channel's identity
// and the run's identity come apart:
//
//   1. **A late event patches the wrong turn.** The old fold patched whichever
//      agent turn was newest. A straggler from run A arriving after run B has
//      started would therefore append a step to B. Every event now carries a
//      `runId`; the run binds it on its first event and drops anything else.
//   2. **A dropped event goes unnoticed.** `seq` is contiguous from 0. A gap
//      means the step list no longer describes the run, so the turn ends as a
//      stream error instead of silently rendering a partial history as complete.
//   3. **Unmounting did not stop the run.** A `Channel`'s callback stays
//      registered with Tauri for the life of the webview, so navigating away
//      mid-compose left every backend send SUCCEEDING: the failed-send guard
//      never tripped and the remaining billable LLM calls ran to completion and
//      persisted a strategy nobody was waiting for. Cancellation is now
//      explicit — the cleanup calls `composeCancel(runId)`, which trips the
//      run's latch in the backend registry.

import { useEffect, useRef, useState } from "react";
import { Channel } from "@tauri-apps/api/core";

import { commands } from "../bindings";
import type { BusEvent, ComposeResult } from "../bindings";

/** The finalize payload, minus the null a cancelled run carries. */
export type StrategySummary = NonNullable<ComposeResult["strategy"]>;

/** One streamed composer step: opened by `toolCallStarted`, closed by its result. */
export interface StepState {
  name: string;
  preview: string;
  outcome: string | undefined;
}

export type RunStatus = "streaming" | "finalized" | "cancelled" | "error";

/** One agent message's run state — the step list and how it ended. */
export interface AgentTurn {
  status: RunStatus;
  steps: StepState[];
  summary: StrategySummary | undefined;
  error: string | undefined;
}

export type Message =
  | { kind: "user"; when: string; text: string }
  | { kind: "agent"; when: string; turn: AgentTurn };

/** A short local timestamp for the message metas. */
function clockLabel(): string {
  return new Date().toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

/** Append a tool result to the LAST open step of that name (names can repeat). */
function attachOutcome(steps: StepState[], name: string, outcome: string): StepState[] {
  let target = -1;
  for (let i = steps.length - 1; i >= 0; i -= 1) {
    if (steps[i].name === name && steps[i].outcome === undefined) {
      target = i;
      break;
    }
  }
  if (target === -1) {
    return steps;
  }
  const copy = steps.slice();
  copy[target] = { ...copy[target], outcome };
  return copy;
}

/** Fold one streamed event's payload into a run's turn — pure. */
function applyEvent(turn: AgentTurn, payload: BusEvent["payload"]): AgentTurn {
  if (payload.kind === "toolCallStarted") {
    return {
      ...turn,
      steps: [
        ...turn.steps,
        { name: payload.name, preview: payload.argumentsPreview, outcome: undefined },
      ],
    };
  }
  if (payload.kind === "toolCallResult") {
    return { ...turn, steps: attachOutcome(turn.steps, payload.name, payload.outcome) };
  }
  return turn;
}

/** The run currently streaming: which turn it owns, and where its stream is. */
interface InFlight {
  /** The index of this run's agent turn in `messages` — fixed at submit. */
  index: number;
  /** The run id, bound from the first event that arrives (`null` until then). */
  runId: string | null;
  /**
   * The `seq` the next accepted event must carry, or `null` before the first one.
   *
   * The backend numbers a run's events contiguously from 0 (`Started`), but the
   * BASELINE is taken from whatever arrives first rather than asserted to be 0.
   * A dropped `Started` costs nothing — it carries no step-list mutation — and
   * hard-coding the origin would couple this fold to the backend's numbering
   * choice for no gain. Every gap after the first observed event is caught.
   */
  nextSeq: number | null;
  /** Set once a gap is seen: the step list is no longer a faithful history. */
  broken: boolean;
}

export function useComposeRun() {
  const [messages, setMessages] = useState<Message[]>([]);
  const [target, setTarget] = useState("");
  const [running, setRunning] = useState(false);

  // Refs, not state: the event callback is registered on a `Channel` at submit
  // and must read the CURRENT run without re-registering, and none of this is
  // rendered.
  const inFlight = useRef<InFlight | null>(null);
  const unmounted = useRef(false);

  // Cancellation-on-unmount, for real. The run id is known from the first event
  // (`Started`, seq 0, sent before the composer is invoked), so any unmount a
  // user can actually perform has one to send. A cancel that loses the race to
  // the last event is reported by the backend as a completed run, which is
  // honest: the strategy was persisted.
  useEffect(
    () => () => {
      unmounted.current = true;
      const run = inFlight.current;
      if (run !== null && run.runId !== null) {
        void commands.composeCancel(run.runId).catch(() => {
          // Nothing to render — the screen is already gone. The backend run
          // ends on its own guard either way.
        });
      }
    },
    [],
  );

  /** Patch the turn at `index`, leaving every other run untouched. */
  const patchTurn = (index: number, patch: (turn: AgentTurn) => AgentTurn) => {
    if (unmounted.current) {
      return;
    }
    setMessages((current) => {
      const message = current[index];
      if (message === undefined || message.kind !== "agent") {
        return current;
      }
      const copy = current.slice();
      copy[index] = { ...message, turn: patch(message.turn) };
      return copy;
    });
  };

  /** Fold one streamed event into the run it belongs to — or drop it. */
  const handleEvent = (event: BusEvent) => {
    const run = inFlight.current;
    if (run === null) {
      return;
    }
    if (run.runId === null) {
      run.runId = event.runId;
    } else if (run.runId !== event.runId) {
      // A straggler from an earlier run. Dropping it is the whole point: it
      // would otherwise append a step to a run that never made that tool call.
      return;
    }
    if (run.nextSeq !== null && event.seq !== run.nextSeq) {
      // A gap. The steps rendered so far are a partial history, and nothing
      // downstream can tell which ones are missing, so the run ends as an
      // error rather than presenting an incomplete list as the whole story.
      if (!run.broken) {
        const expected = run.nextSeq;
        run.broken = true;
        patchTurn(run.index, (turn) => ({
          ...turn,
          status: "error",
          error: `stream error: expected event ${expected}, received ${event.seq} — the step list below is incomplete`,
        }));
      }
      return;
    }
    run.nextSeq = event.seq + 1;
    patchTurn(run.index, (turn) => applyEvent(turn, event.payload));
  };

  const submit = () => {
    const text = target.trim();
    if (text === "" || running) {
      return;
    }
    setTarget("");
    setRunning(true);

    // The agent turn's index is deterministic: `submit` is the only thing that
    // appends, it appends the user message then the agent message, and the
    // `running` guard means no other run is in flight to move them.
    const index = messages.length + 1;
    inFlight.current = { index, runId: null, nextSeq: null, broken: false };

    const channel = new Channel<BusEvent>();
    channel.onmessage = handleEvent;

    const when = clockLabel();
    setMessages((current) => [
      ...current,
      { kind: "user", when, text },
      { kind: "agent", when, turn: { status: "streaming", steps: [], summary: undefined, error: undefined } },
    ]);

    void commands
      .composeStrategy(text, channel)
      .then((result) => {
        // A run whose stream broke keeps its stream error: the command's return
        // value describes work the rendered step list cannot account for, so
        // overwriting the error with `✓ completed` would hide the gap.
        if (inFlight.current?.index === index && inFlight.current.broken) {
          return;
        }
        if (result.status === "ok") {
          patchTurn(index, (turn) => ({
            ...turn,
            status: result.data.cancelled ? "cancelled" : "finalized",
            summary: result.data.strategy ?? undefined,
          }));
        } else {
          patchTurn(index, (turn) => ({
            ...turn,
            status: "error",
            error: result.error.message,
          }));
        }
      })
      .catch((error: unknown) => {
        // A non-serialized failure (an unreachable command, a broken bridge)
        // still renders — the no-silent-failure rule covers this path too.
        patchTurn(index, (turn) => ({
          ...turn,
          status: "error",
          error: error instanceof Error ? error.message : String(error),
        }));
      })
      .finally(() => {
        setRunning(false);
        if (inFlight.current?.index === index) {
          inFlight.current = null;
        }
      });
  };

  return { messages, target, setTarget, running, submit };
}
