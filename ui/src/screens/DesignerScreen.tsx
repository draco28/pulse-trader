// The Strategy Designer screen (r1.s1.w4) — the release's core journey's
// frontend half.
//
// The screen is a conversation: a user bubble per submitted natural-language
// target, one agent message per compose run whose step list renders the
// structured events the `compose_strategy` command streams over its
// per-invocation channel — each tool call appearing the moment its
// `toolCallStarted` arrives and its outcome appending when the matching
// `toolCallResult` does (that one-by-one rendering is `d1`'s observable).
//
// What this screen deliberately does NOT do (the r1-honest composition):
//
//   - No live DSL-preview pane. The composer exposes no draft DSL mid-run —
//     `Finalized` is the first moment a version exists — so a "live · synced"
//     preview would be a fabricated intermediate state. The finalize summary
//     card, rendered from the command's RETURN value, is the honest
//     presentation.
//   - No cost pill, no session metering, no mutation flow (clone/draft/compare)
//     — none of that machinery exists this round; this compose creates a NEW
//     strategy + its initial version, and the card renders exactly the fields
//     the outcome carries (name, version id, created-by, LlmCall count, DSL
//     lines) and omits what it does not.
//   - No cancel button. Cancellation is wired on the BACKEND: unmounting drops
//     the channel, the next send fails, and the run stops (a dead cancel
//     affordance is the broken-link defect at button scale). A genuinely
//     wired cancel needs channel-side support this round does not build.
//   - No credential is requested, displayed, or echoed anywhere. The banner
//     (`w5`) states the no-credential condition globally; an unresolvable
//     credential surfaces HERE as the run's rendered `BusError` message —
//     no silent failure.

import { useState } from "react";
import { Channel } from "@tauri-apps/api/core";

import { commands } from "../bindings";
import type { BusEvent, ComposeResult } from "../bindings";

/** The finalize payload, minus the null a cancelled run carries. */
type StrategySummary = NonNullable<ComposeResult["strategy"]>;

/** One streamed composer step: opened by `toolCallStarted`, closed by its result. */
interface StepState {
  name: string;
  preview: string;
  outcome: string | undefined;
}

type RunStatus = "streaming" | "finalized" | "cancelled" | "error";

/** One agent message's run state — the step list and how it ended. */
interface AgentTurn {
  status: RunStatus;
  steps: StepState[];
  summary: StrategySummary | undefined;
  error: string | undefined;
}

type Message =
  | { kind: "user"; when: string; text: string }
  | { kind: "agent"; when: string; turn: AgentTurn };

/** A short local timestamp for the message metas. */
function clockLabel(): string {
  return new Date().toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

export default function DesignerScreen() {
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
    const { payload } = event;
    if (payload.kind === "toolCallStarted") {
      patchLastTurn((turn) => ({
        ...turn,
        steps: [
          ...turn.steps,
          { name: payload.name, preview: payload.argumentsPreview, outcome: undefined },
        ],
      }));
    } else if (payload.kind === "toolCallResult") {
      patchLastTurn((turn) => ({
        ...turn,
        steps: attachOutcome(turn.steps, payload.name, payload.outcome),
      }));
    }
    // `started` / `finished` carry no step-list mutation of their own: the
    // streaming state belongs to the run as a whole, and the finalize summary
    // arrives with the command's return value, not on the event channel.
  };

  const submit = () => {
    const text = target.trim();
    if (text === "" || running) {
      return;
    }
    setTarget("");
    setRunning(true);

    // One channel per invocation (grill A2): it is the whole correlation
    // between this run's steps and this agent message.
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

  return (
    <div className="designer">
      <div className="dsg-scroll">
        <div className="dsg-chat">
          {messages.length === 0 && (
            <div className="dsg-empty">
              <p className="dsg-empty-title">Describe a strategy in natural language.</p>
              <p className="dsg-empty-body">
                The composer builds it with its builder tools; each tool call streams in
                here as it happens, and the finished strategy is saved to your library
                as a new version.
              </p>
            </div>
          )}
          {messages.map((message, index) =>
            message.kind === "user" ? (
              <div key={index} className="dsg-msg dsg-msg-user">
                <div className="dsg-msg-meta">
                  <span>you</span>
                  <span className="dsg-when">{message.when}</span>
                </div>
                <div className="dsg-msg-body">{message.text}</div>
              </div>
            ) : (
              <div key={index} className="dsg-msg dsg-msg-agent">
                <div className="dsg-rail" />
                <div className="dsg-agent-body">
                  <div className="dsg-msg-meta">
                    <span className="dsg-agent-name">✦ Composer</span>
                    <span className="dsg-when">{message.when}</span>
                  </div>
                  <StepList turn={message.turn} />
                  {message.turn.summary !== undefined && (
                    <SummaryCard summary={message.turn.summary} />
                  )}
                  {message.turn.status === "error" && message.turn.error !== undefined && (
                    <div className="dsg-error" role="alert">
                      {message.turn.error}
                    </div>
                  )}
                  {message.turn.status === "cancelled" && (
                    <div className="dsg-error" role="alert">
                      The run was cancelled — this screen went away before it finished.
                    </div>
                  )}
                </div>
              </div>
            ),
          )}
        </div>
      </div>

      <div className="dsg-input">
        <textarea
          className="dsg-textarea"
          placeholder="Describe a strategy in natural language…"
          value={target}
          onChange={(event) => setTarget(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !event.shiftKey) {
              event.preventDefault();
              submit();
            }
          }}
        />
        <div className="dsg-input-foot">
          <span className="dsg-hint">enter to send · shift-enter for newline</span>
          <button type="button" className="dsg-send" disabled={running} onClick={submit}>
            {running ? (
              <>
                <span className="spinner-sm" /> Composer is running…
              </>
            ) : (
              "Send"
            )}
          </button>
        </div>
      </div>
    </div>
  );
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

/** The step list of one run, with its honest running/completed header. */
function StepList({ turn }: { turn: AgentTurn }) {
  if (turn.steps.length === 0 && turn.status !== "streaming") {
    return null;
  }
  return (
    <div className="dsg-steps">
      <div className="dsg-steps-head">
        {turn.status === "streaming" ? (
          <>
            <span className="spinner-sm" /> streaming · {turn.steps.length}{" "}
            {turn.steps.length === 1 ? "step" : "steps"} so far
          </>
        ) : (
          <>✓ completed · {turn.steps.length} tool {turn.steps.length === 1 ? "call" : "calls"}</>
        )}
      </div>
      <div className="dsg-steps-body">
        {turn.steps.map((step, index) => {
          const open = step.outcome === undefined && turn.status === "streaming";
          return (
            <div key={index} className={`dsg-step${open ? " is-running" : " is-done"}`}>
              <div className="dsg-step-marker">
                {step.outcome !== undefined ? (
                  "✓"
                ) : open ? (
                  <span className="spinner-sm" />
                ) : (
                  "·"
                )}
              </div>
              <div className="dsg-step-body">
                <div className="dsg-step-row">
                  <code className="dsg-step-name">{step.name}</code>
                  <code className="dsg-step-preview">{step.preview}</code>
                </div>
                {step.outcome !== undefined && (
                  <div className="dsg-step-outcome">{step.outcome}</div>
                )}
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}

/** One titled block of the summary card. */
function SscBlock({
  title,
  lines,
}: {
  title: string;
  lines: readonly string[];
}) {
  return (
    <div className="dsg-ssc-block">
      <div className="dsg-ssc-block-head">{title}</div>
      <div className="dsg-ssc-block-body">
        {lines.map((line, index) => (
          <code key={index}>{line}</code>
        ))}
      </div>
    </div>
  );
}

/** The finalize summary card — rendered from the command's return value only. */
function SummaryCard({ summary }: { summary: StrategySummary }) {
  return (
    <div className="dsg-ssc">
      <div className="dsg-ssc-head">
        <span className="dsg-ssc-tag">strategy preview</span>
        <span className="dsg-ssc-name">{summary.strategyName}</span>
        <span className="dsg-ssc-ver">{summary.versionId}</span>
      </div>
      <div className="dsg-ssc-grid">
        <SscBlock title="Setup" lines={[summary.dsl.direction]} />
        <SscBlock title="Entries" lines={[summary.dsl.entry]} />
        {summary.dsl.filters.length > 0 && (
          <SscBlock title="Filters" lines={summary.dsl.filters} />
        )}
        <SscBlock title="Exits" lines={summary.dsl.exits} />
        <SscBlock title="Risk" lines={summary.dsl.risk} />
      </div>
      <div className="dsg-ssc-foot">
        <span className="dsg-ssc-meta">
          {summary.createdBy} · {summary.llmCallCount} LLM{" "}
          {summary.llmCallCount === 1 ? "call" : "calls"}
        </span>
      </div>
    </div>
  );
}
