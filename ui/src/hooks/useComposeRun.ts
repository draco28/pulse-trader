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

import { useState } from "react";
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

export function useComposeRun() {
  const [messages, setMessages] = useState<Message[]>([]);
  const [target, setTarget] = useState("");
  const [running, setRunning] = useState(false);

  /** Patch the newest agent turn (the run in flight), leaving older runs untouched. */
  const patchLastTurn = (patch: (turn: AgentTurn) => AgentTurn) => {
    setMessages((current) => {
      const last = current[current.length - 1];
      if (last === undefined || last.kind !== "agent") {
        return current;
      }
      const copy = current.slice();
      copy[copy.length - 1] = { ...last, turn: patch(last.turn) };
      return copy;
    });
  };

  /** Fold one streamed event into the running turn. */
  const handleEvent = (event: BusEvent) => {
    patchLastTurn((turn) => applyEvent(turn, event.payload));
  };

  const submit = () => {
    const text = target.trim();
    if (text === "" || running) {
      return;
    }
    setTarget("");
    setRunning(true);

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
        if (result.status === "ok") {
          patchLastTurn((turn) => ({
            ...turn,
            status: result.data.cancelled ? "cancelled" : "finalized",
            summary: result.data.strategy ?? undefined,
          }));
        } else {
          patchLastTurn((turn) => ({
            ...turn,
            status: "error",
            error: result.error.message,
          }));
        }
      })
      .catch((error: unknown) => {
        // A non-serialized failure (an unreachable command, a broken bridge)
        // still renders — the no-silent-failure rule covers this path too.
        patchLastTurn((turn) => ({
          ...turn,
          status: "error",
          error: error instanceof Error ? error.message : String(error),
        }));
      })
      .finally(() => {
        setRunning(false);
      });
  };

  return { messages, target, setTarget, running, submit };
}
