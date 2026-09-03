// The Backtest Lab (r1.s3.w4) — the rendering half of the spine's journey.
//
// A trader selects a persisted strategy version from the REAL library catalog
// (`commands.libraryOverview` — the Library's own read, no second catalog) and
// presses Run: `commands.runBacktestVersion({ versionId })` executes W3's
// fixed request/response command and this screen renders only the fresh
// `BacktestRunDto` it returns. Every decimal and epoch value is the DTO's own
// exact string — display truth is the string itself. The ONLY numeric
// conversion in this file is `parseFloat` for chart geometry (pixel
// positions), never for display, derivation or comparison; the screen does no
// binning, no money math, no deltas, no re-backtest, and invents no value the
// DTO does not carry. No sample option, no mock run, no progress percentage,
// no cancel affordance — the command is request/response and r1's target is
// <5s.
//
// Convention (UnbuiltScreen.tsx is the worked example): one file per screen,
// default export, zero props — the shell mounts it from the route table and
// all state is its own.

import { useEffect, useMemo, useRef, useState } from "react";

import { commands } from "../bindings";
import type {
  BacktestRunDto,
  BusError,
  HistogramDto,
  LibraryOverview,
  LibraryVersion,
  RegimeCellDto,
  TradeRowDto,
} from "../bindings";

/** The em dash every null renders — a statement that no value exists, never
 * a zero dressed up as data (the Library's grill A1 rule, applied here). */
const EM_DASH = "—";

/** The one window into the chart geometry exception: a DTO string becomes a
 * number ONLY to place a pixel. Never displayed, never compared, never
 * re-formatted. */
function num(value: string): number {
  return Number.parseFloat(value);
}

// ---------------------------------------------------------------------------
// Catalog state (the selector's four explicit states)
// ---------------------------------------------------------------------------

/** One selectable option: a real (strategy, version) pair from the payload. */
interface Option {
  /** The version's id — the run request's `versionId`. */
  value: string;
  /** `Strategy name · vN`, N the version's position in its parent-ordered list. */
  label: string;
}

type CatalogState =
  | { kind: "loading" }
  | { kind: "error"; code: string; message: string }
  | { kind: "empty" }
  | { kind: "ready"; options: Option[] };

/** Flatten the catalog's strategies into the selector's options — the real
 * versions only; a version not in the payload cannot appear here. */
function catalogOptions(overview: LibraryOverview): Option[] {
  const options: Option[] = [];
  for (const strategy of overview.strategies) {
    strategy.versions.forEach((version: LibraryVersion, index: number) => {
      options.push({ value: version.id, label: `${strategy.name} · v${index + 1}` });
    });
  }
  return options;
}

// ---------------------------------------------------------------------------
// Run state (the Run button is the sole trigger)
// ---------------------------------------------------------------------------

type RunState =
  | { kind: "idle" }
  | { kind: "running" }
  | { kind: "failed"; error: BusError }
  | { kind: "done"; dto: BacktestRunDto };

// ---------------------------------------------------------------------------
// Fixed display constants (pinned by the spec, asserted by the tests)
// ---------------------------------------------------------------------------

/** The regime labels in the DTO's fixed order — hue fixed by slot, sign by bar
 * position, never a second hue. */
const REGIME_LABELS: Record<string, string> = {
  trending_up: "Trending up",
  trending_down: "Trending down",
  ranging: "Ranging",
  unknown: "Unknown",
};

/** The 18 trade columns in the DTO's own field order. */
const TRADE_FIELDS = [
  "direction", "qty", "entryPrice", "exitPrice", "entrySignalTime", "entryFillTime",
  "exitSignalTime", "exitFillTime", "feesTotal", "fundingTotal", "slippageTotal",
  "realizedPnl", "realizedR", "mfeR", "maeR", "exitReason", "source", "regime",
] as const;

// ---------------------------------------------------------------------------
// Screen
// ---------------------------------------------------------------------------

export default function BacktestLabScreen() {
  const [catalog, setCatalog] = useState<CatalogState>({ kind: "loading" });
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [run, setRun] = useState<RunState>({ kind: "idle" });
  /** Bumped on every selector change so a run whose result lands after the
   * selection moved cannot attach itself to the newly selected version. */
  const runToken = useRef(0);

  useEffect(() => {
    let alive = true;
    commands
      .libraryOverview()
      .then((result) => {
        if (!alive) return;
        if (result.status === "ok") {
          const options = catalogOptions(result.data);
          setCatalog(
            options.length === 0 ? { kind: "empty" } : { kind: "ready", options },
          );
        } else {
          setCatalog({
            kind: "error",
            code: result.error.code,
            message: result.error.message,
          });
        }
      })
      .catch(() => {
        // The IPC call itself failing (no app handle under a non-Tauri
        // preview) — the honest error state, never fabricated options.
        if (alive) {
          setCatalog({ kind: "error", code: "internal", message: "The library read failed." });
        }
      });
    return () => {
      alive = false;
    };
  }, []);

  const options = catalog.kind === "ready" ? catalog.options : [];
  const currentId = selectedId ?? options[0]?.value ?? null;

  /** Selecting a version clears any rendered result: a stale result must not
   * appear attributable to the newly selected version. The selector is disabled
   * while a run is in flight (see the `<select>` below): a mid-run change here
   * would reset the state to `idle` and re-enable Run while the original,
   * uncancellable request was still executing — a second click would launch an
   * overlapping engine run whose persisted result the token check then drops. */
  function onSelectorChange(id: string) {
    runToken.current += 1;
    setSelectedId(id);
    setRun({ kind: "idle" });
  }

  /** The screen's ONLY invocation of the backtest command — the Run button's
   * click handler. No mount effect reaches this. */
  function onRun() {
    if (currentId === null || run.kind === "running") return;
    const token = ++runToken.current;
    setRun({ kind: "running" });
    commands
      .runBacktestVersion({ versionId: currentId })
      .then((result) => {
        if (token !== runToken.current) return;
        if (result.status === "ok") {
          setRun({ kind: "done", dto: result.data });
        } else {
          setRun({ kind: "failed", error: result.error });
        }
      })
      .catch(() => {
        if (token !== runToken.current) return;
        setRun({
          kind: "failed",
          error: { code: "internal", message: "The backtest run failed.", run_id: null },
        });
      });
  }

  return (
    <div className="bt-lab">
      <div className="ctool">
        <div className="ctool-left">
          <h1 className="ctool-title">Backtest Lab</h1>
          <span className="ctool-count">
            Select a persisted strategy version and run it against the persisted
            BTCUSDT candle snapshots
          </span>
        </div>
      </div>

      {catalog.kind === "loading" && <div className="bt-state">Loading the strategy library…</div>}

      {catalog.kind === "error" && (
        <div className="bt-state bt-error" role="alert">
          <span className="mono">{catalog.code}</span> {catalog.message}
        </div>
      )}

      {catalog.kind === "empty" && (
        // A designed first run, not a void — one line naming the single next
        // action (the Library's G4 pattern).
        <div className="bt-state bt-empty">
          <h2 className="bt-empty-title">No strategies yet</h2>
          <p className="bt-empty-body">
            Describe a strategy in the Strategy Designer — its versions will
            appear here to backtest.
          </p>
        </div>
      )}

      {catalog.kind === "ready" && (
        <div className="bt-toolbar">
          <label className="bt-select-label" htmlFor="bt-version">
            Strategy version
          </label>
          <select
            id="bt-version"
            className="bt-select"
            value={currentId ?? ""}
            onChange={(event) => onSelectorChange(event.target.value)}
            disabled={run.kind === "running"}
          >
            {options.map((option) => (
              <option key={option.value} value={option.value}>
                {option.label}
              </option>
            ))}
          </select>
          <button
            type="button"
            className="bt-run btn-prim"
            onClick={onRun}
            disabled={run.kind === "running" || currentId === null}
          >
            {run.kind === "running" ? "Running…" : "Run backtest"}
          </button>
          {/* No percentage, no cancel — the run is request/response. */}
        </div>
      )}

      {run.kind === "running" && (
        <div className="bt-state dim" role="status">
          Running the backtest…
        </div>
      )}

      {run.kind === "failed" && (
        <div className="bt-run-error" role="alert">
          <div>
            <span className="mono">{run.error.code}</span> {run.error.message}
          </div>
          {/* The structured saved-run truth: the DTO's `run_id` FIELD, read
              directly — never parsed out of the message. */}
          {run.error.run_id !== null && (
            <p className="bt-saved-run mono">Saved as run {run.error.run_id}</p>
          )}
        </div>
      )}

      {run.kind === "done" && (
        <div className="bt-result" key={run.dto.runId}>
          <ProvenanceBand dto={run.dto} />
          <KpiTiles dto={run.dto} />
          <EquityChart
            points={run.dto.equity}
            startingEquity={run.dto.startingEquity}
          />
          <RegimeBars cells={run.dto.regimes} />
          <Histogram title="MFE (R)" className="bt-mfe" hist={run.dto.mfe} />
          <Histogram title="MAE (R)" className="bt-mae" hist={run.dto.mae} />
          <TradeTable trades={run.dto.trades} />
          <ChartTwins dto={run.dto} />
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Provenance band
// ---------------------------------------------------------------------------

/** One label/value pair in the provenance band's grid. */
function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="bt-field">
      <span className="bt-field-label">{label}</span>
      <span className="bt-field-value mono">{children}</span>
    </div>
  );
}

function ProvenanceBand({ dto }: { dto: BacktestRunDto }) {
  return (
    <section className="bt-section bt-provenance" aria-label="Run provenance">
      <Field label="pair">{dto.pair}</Field>
      <Field label="primary">
        {dto.primaryTimeframe} · {dto.primaryDataVersion}
      </Field>
      <Field label="htf">
        {dto.htfTimeframe !== null ? `${dto.htfTimeframe} · ${dto.htfDataVersion}` : EM_DASH}
      </Field>
      <Field label="date range">
        {dto.firstOpenTimeMs} → {dto.lastCloseTimeMs}
      </Field>
      <Field label="starting equity">{dto.startingEquity}</Field>
      <Field label="taker fee">{dto.takerFeeBps} bps</Field>
      <Field label="slippage">{dto.slippageBps} bps</Field>
      <Field label="funding">{dto.funding}</Field>
      <Field label="engine">
        {dto.engineFingerprint} · {dto.engineTarget}
      </Field>
      <Field label="content hash">{dto.resultContentHash}</Field>
      <Field label="run">{dto.runId}</Field>
      <Field label="version">{dto.strategyVersionId}</Field>
      <Field label="schema">{dto.schemaVersion}</Field>
      <Field label="created">{dto.createdAt}</Field>
      {/* The DTO's one control-metadata exception: the pre-save comparison's
          own warning, visibly flagged as such — present only when it exists. */}
      {dto.fingerprintWarning !== null && (
        <p className="bt-fp-warning" role="note">
          Fingerprint warning (pre-save comparison): {dto.fingerprintWarning}
        </p>
      )}
    </section>
  );
}

// ---------------------------------------------------------------------------
// KPI tiles
// ---------------------------------------------------------------------------

function Kpi({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="bt-kpi">
      <span className="bt-kpi-lab">{label}</span>
      <span className="bt-kpi-val mono">{children}</span>
    </div>
  );
}

function KpiTiles({ dto }: { dto: BacktestRunDto }) {
  return (
    <section className="bt-section bt-kpis" aria-label="Headline metrics">
      {/* Exact strings; null renders an em dash. No deltas, no comparisons,
          no derived percentages — the coach rail that compares runs is r1.s4. */}
      <Kpi label="Net P&L">{dto.netPnl}</Kpi>
      <Kpi label="Expectancy">{dto.expectancy}</Kpi>
      <Kpi label="Win rate">{dto.winRate}</Kpi>
      <Kpi label="Profit factor">{dto.profitFactor ?? EM_DASH}</Kpi>
      <Kpi label="Trades">{dto.tradeCount}</Kpi>
      <Kpi label="Wins">{dto.winCount}</Kpi>
      <Kpi label="Losses">{dto.lossCount}</Kpi>
      <Kpi label="Max win streak">{dto.maxWinStreak}</Kpi>
      <Kpi label="Max loss streak">{dto.maxLossStreak}</Kpi>
      <Kpi label="Max drawdown">{dto.maxDrawdown}</Kpi>
      <Kpi label="Sharpe">{dto.sharpe ?? EM_DASH}</Kpi>
      <Kpi label="Sortino">{dto.sortino ?? EM_DASH}</Kpi>
      <Kpi label="Skipped sub-lot">{dto.skippedSubLot}</Kpi>
      <Kpi label="Skipped sub-notional">{dto.skippedSubNotional}</Kpi>
      <Kpi label="Skipped leverage-capped">{dto.skippedLeverageCapped}</Kpi>
      <Kpi label="Fees">{dto.feesTotal}</Kpi>
      <Kpi label="Funding">{dto.fundingTotal}</Kpi>
      <Kpi label="Slippage">{dto.slippageTotal}</Kpi>
      <Kpi label="Gross profit">{dto.grossProfit}</Kpi>
      <Kpi label="Gross loss">{dto.grossLoss}</Kpi>
      <Kpi label="Avg win">{dto.avgWin}</Kpi>
      <Kpi label="Avg loss">{dto.avgLoss}</Kpi>
    </section>
  );
}

// ---------------------------------------------------------------------------
// Equity chart (one series, focusable, pointer/keyboard parity)
// ---------------------------------------------------------------------------

const EQ_W = 720;
const EQ_H = 240;
const EQ_PAD = 28;
/** The padded plot fraction — keeps the extreme points off the frame. */
const EQ_INSET = 0.08;

function EquityChart({
  points,
  startingEquity,
}: {
  points: BacktestRunDto["equity"];
  startingEquity: string;
}) {
  // Geometry only: the strings themselves are what renders.
  const values = points.map((p) => num(p.equity));
  const refValue = num(startingEquity);
  const lo = Math.min(...values, refValue);
  const hi = Math.max(...values, refValue);
  const span = hi - lo;
  const min = lo - span * EQ_INSET;
  const max = hi + span * EQ_INSET;

  const x = (i: number) => EQ_PAD + (i / Math.max(points.length - 1, 1)) * (EQ_W - 2 * EQ_PAD);
  const y = (v: number) =>
    EQ_H - EQ_PAD - ((v - min) / Math.max(max - min, 1e-9)) * (EQ_H - 2 * EQ_PAD);

  const [active, setActive] = useState(0);
  const clamped = Math.min(active, points.length - 1);
  const point = points[clamped];

  const linePoints = points.map((p, i) => `${x(i)},${y(num(p.equity))}`).join(" ");
  const areaPoints = `${linePoints} ${x(points.length - 1)},${y(lo)} ${x(0)},${y(lo)}`;

  /** Keyboard traversal: Left/Right step point-to-point, Home/End jump. */
  function onKeyDown(event: React.KeyboardEvent<HTMLDivElement>) {
    if (event.key === "ArrowRight") {
      event.preventDefault();
      setActive((i) => Math.min(i + 1, points.length - 1));
    } else if (event.key === "ArrowLeft") {
      event.preventDefault();
      setActive((i) => Math.max(i - 1, 0));
    } else if (event.key === "Home") {
      event.preventDefault();
      setActive(0);
    } else if (event.key === "End") {
      event.preventDefault();
      setActive(points.length - 1);
    }
  }

  return (
    <section className="bt-section bt-equity" aria-label="Equity curve">
      <h3 className="bt-h">Equity curve</h3>
      <div
        className="bt-eq-plot"
        role="img"
        aria-label={`Equity curve, ${points.length} points`}
        tabIndex={0}
        onKeyDown={onKeyDown}
      >
        <svg viewBox={`0 0 ${EQ_W} ${EQ_H}`} preserveAspectRatio="none">
          {/* One series: a 2px line with a quiet area fill beneath it. */}
          <polygon className="bt-eq-area" points={areaPoints} />
          <polyline className="bt-eq-line" points={linePoints} />
          {/* The starting-equity reference line, labelled with the DTO's own
              string — the reference is never colour-only. */}
          <line
            className="bt-eq-ref"
            x1={EQ_PAD}
            x2={EQ_W - EQ_PAD}
            y1={y(refValue)}
            y2={y(refValue)}
          />
          {points.map((p, i) => (
            <circle
              key={p.timeMs}
              className="bt-eq-hit"
              cx={x(i)}
              cy={y(num(p.equity))}
              r={9}
              onMouseOver={() => setActive(i)}
            />
          ))}
        </svg>
      </div>
      {/* One readout for both pointer and keyboard — parity is structural:
          the same state drives the same exact strings either way. */}
      <p className="bt-eq-readout mono" aria-live="polite">
        <span className="bt-eq-readout-time">{point.timeMs}</span> ·{" "}
        <span className="bt-eq-readout-equity">{point.equity}</span>
      </p>
      <p className="bt-eq-ref-label">
        starting equity <span className="mono">{startingEquity}</span>
      </p>
    </section>
  );
}

// ---------------------------------------------------------------------------
// Regime bars (one zero baseline, hue fixed by slot)
// ---------------------------------------------------------------------------

function RegimeBars({ cells }: { cells: RegimeCellDto[] }) {
  // Geometry only: the widest |netPnl| sets the scale; sign chooses the side
  // of the one zero baseline — never a second hue.
  const maxAbs = Math.max(...cells.map((c) => Math.abs(num(c.netPnl))), 1e-9);

  return (
    <section className="bt-section bt-regimes" aria-label="Regime breakdown">
      <h3 className="bt-h">Regimes</h3>
      {/* Compact fixed-order legend beside the direct row labels — both, so
          the hue mapping is legible with or without the geometry. */}
      <div className="bt-legend">
        {cells.map((cell, i) => (
          <span key={cell.regime} className="bt-legend-item">
            <span className={`bt-swatch bt-slot-${i + 1}`} />
            {REGIME_LABELS[cell.regime] ?? cell.regime}
          </span>
        ))}
      </div>
      <div className="bt-regime-rows">
        {cells.map((cell, i) => {
          const width = (Math.abs(num(cell.netPnl)) / maxAbs) * 50;
          const positive = num(cell.netPnl) >= 0;
          return (
            <div key={cell.regime} className="bt-regime-row" style={{ minHeight: 24 }}>
              <span className={`bt-swatch bt-slot-${i + 1}`} />
              <span className="bt-regime-name">{REGIME_LABELS[cell.regime] ?? cell.regime}</span>
              <span className="bt-regime-track">
                <span className="bt-regime-baseline" />
                <span
                  className={`bt-regime-bar bt-slot-${i + 1}`}
                  style={positive ? { left: "50%", width: `${width}%` } : { right: "50%", width: `${width}%` }}
                />
              </span>
              <span className="bt-regime-count mono">{cell.tradeCount}</span>
              <span className="bt-regime-pnl mono">{cell.netPnl}</span>
            </div>
          );
        })}
      </div>
    </section>
  );
}

// ---------------------------------------------------------------------------
// MFE / MAE histograms (single hue, pinned column order)
// ---------------------------------------------------------------------------

/** One histogram column: underflow | finite bin | overflow, in fixed visual
 * order. The label interpolates the DTO's own bound strings verbatim — the
 * screen computes no binning. */
function histogramColumns(hist: HistogramDto): { label: string; count: number }[] {
  return [
    { label: "< 0 R", count: hist.underflow },
    ...hist.bins.map((b) => ({ label: `[${b.lower}, ${b.upper}) R`, count: b.count })),
    { label: "≥ 3 R", count: hist.overflow },
  ];
}

function Histogram({ title, className, hist }: { title: string; className: string; hist: HistogramDto }) {
  const columns = histogramColumns(hist);
  const maxCount = Math.max(...columns.map((c) => c.count), 1);

  return (
    <section className={`bt-section bt-histogram ${className}`} aria-label={title}>
      <h3 className="bt-h">{title}</h3>
      <div className="bt-hist-cols">
        {columns.map((column, i) => (
          // The interactive dimension (the pointer's horizontal travel) is at
          // least 24px regardless of the painted mark.
          <div key={i} className="bt-col" style={{ minWidth: 24 }}>
            <span className="bt-col-count mono">{column.count}</span>
            <div className="bt-col-stack">
              <div
                className="bt-col-bar bt-slot-1"
                style={{
                  height: `${(column.count / maxCount) * 100}%`,
                  minHeight: column.count > 0 ? 2 : 0,
                }}
              />
            </div>
            <span className="bt-col-label">{column.label}</span>
          </div>
        ))}
      </div>
    </section>
  );
}

// ---------------------------------------------------------------------------
// Trade table (all 18 fields, seq order, focusable scroll region)
// ---------------------------------------------------------------------------

function TradeTable({ trades }: { trades: TradeRowDto[] }) {
  return (
    <section className="bt-section bt-trades-section" aria-label="Trades">
      <h3 className="bt-h">Trades</h3>
      {/* Keyboard-focusable scroll region: the table scrolls within it rather
          than being clipped by the pane. */}
      <div className="bt-table-scroll" role="region" aria-label="Trade rows" tabIndex={0}>
        <table className="bt-trades">
          <thead>
            <tr>
              {TRADE_FIELDS.map((field) => (
                <th key={field} scope="col">
                  {field}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {trades.map((trade, i) => (
              <tr key={i}>
                {TRADE_FIELDS.map((field) => (
                  <td key={field} className="mono">
                    {trade[field]}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </section>
  );
}

// ---------------------------------------------------------------------------
// Table twins (no data reachable only through the graphic)
// ---------------------------------------------------------------------------

/** Every chart's in-DOM accessible twin carrying the same exact strings. */
function ChartTwins({ dto }: { dto: BacktestRunDto }) {
  const mfeColumns = useMemo(() => histogramColumns(dto.mfe), [dto.mfe]);
  const maeColumns = useMemo(() => histogramColumns(dto.mae), [dto.mae]);

  return (
    <section className="bt-twins" aria-label="Chart data tables">
      <table className="bt-twin bt-twin-equity">
        <caption>Equity curve</caption>
        <thead>
          <tr>
            <th scope="col">time (epoch ms)</th>
            <th scope="col">equity</th>
          </tr>
        </thead>
        <tbody>
          {dto.equity.map((p) => (
            <tr key={p.timeMs}>
              <td className="mono">{p.timeMs}</td>
              <td className="mono">{p.equity}</td>
            </tr>
          ))}
        </tbody>
      </table>

      <table className="bt-twin bt-twin-regimes">
        <caption>Regime breakdown</caption>
        <thead>
          <tr>
            <th scope="col">regime</th>
            <th scope="col">trades</th>
            <th scope="col">net P&L</th>
          </tr>
        </thead>
        <tbody>
          {dto.regimes.map((cell) => (
            <tr key={cell.regime} data-regime={cell.regime}>
              <td>{REGIME_LABELS[cell.regime] ?? cell.regime}</td>
              <td className="mono">{cell.tradeCount}</td>
              <td className="mono">{cell.netPnl}</td>
            </tr>
          ))}
        </tbody>
      </table>

      {(
        [
          ["bt-twin-mfe", "MFE (R)", mfeColumns],
          ["bt-twin-mae", "MAE (R)", maeColumns],
        ] as const
      ).map(([twinClass, caption, columns]) => (
        <table key={twinClass} className={`bt-twin ${twinClass}`}>
          <caption>{caption}</caption>
          <thead>
            <tr>
              <th scope="col">column</th>
              <th scope="col">count</th>
            </tr>
          </thead>
          <tbody>
            {columns.map((column) => (
              <tr key={column.label}>
                <td>{column.label}</td>
                <td className="mono">{column.count}</td>
              </tr>
            ))}
          </tbody>
        </table>
      ))}
    </section>
  );
}
