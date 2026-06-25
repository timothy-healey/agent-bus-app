import type { CSSProperties } from "react";
import type { Run } from "../ipc/runtime";
import { Button } from "./ui/Button";

export interface RunSelectorProps {
  runs: Run[];
  selectedRun: Run | null;
  /// The active (newest incomplete) run, marked in the list. `null` when none.
  activeRun: Run | null;
  onSelect: (runId: string) => void;
  onStartRun: () => void;
  /// True when the run is stopped (braked) — swaps the control to a Resume affordance.
  braked: boolean;
  /// Halt the active run (brake on). Shown while a run is running and not braked.
  onStop: () => void;
  /// Resume after a Stop (brake off). Shown while braked.
  onResume: () => void;
  /// True while a Start is in flight — disables the button + shows progress.
  starting?: boolean;
  /// True during the initial runs fetch — shows a quiet "loading runs…" instead
  /// of flashing the "no runs yet" empty state before the first result lands.
  loading?: boolean;
}

/// A short, stable label for a run option: the tail of its id + its status. The
/// uuid tail keeps options distinguishable without leaking the whole id.
export function runOptionLabel(run: Run, isActive: boolean): string {
  const shortId = run.id.length > 8 ? `…${run.id.slice(-8)}` : run.id;
  const status = run.completed ? "completed" : isActive ? "active" : "running";
  return `run ${shortId} · ${status}`;
}

/// Board header control (④e): a run selector listing the project's runs (active
/// marked) + a "Start run" button. Start needs no topic — the team prompts are
/// the work (the A6 insight). The topbar brake toggle is Stop. When there are no
/// runs yet, the selector is replaced by an inline "no runs yet" hint so Start is
/// the obvious next action.
export function RunSelector({ runs, selectedRun, activeRun, onSelect, onStartRun, braked, onStop, onResume, starting = false, loading = false }: RunSelectorProps) {
  const bar: CSSProperties = {
    display: "flex",
    alignItems: "center",
    gap: "var(--sp-3)",
    padding: "var(--sp-2) var(--sp-8)",
    borderBottom: "1px solid var(--border)",
    background: "var(--surface)",
  };
  const label: CSSProperties = {
    fontSize: "var(--ts-sm)",
    color: "var(--text-3)",
    textTransform: "lowercase",
    letterSpacing: "0.04em",
  };
  const select: CSSProperties = {
    background: "var(--bg)",
    border: "1px solid var(--border)",
    color: "var(--text)",
    borderRadius: "var(--r-sm)",
    padding: "var(--sp-1) var(--sp-2)",
    fontSize: "var(--ts-sm)",
    fontFamily: "inherit",
    minWidth: 180,
  };
  const empty: CSSProperties = { fontSize: "var(--ts-sm)", color: "var(--text-3)" };
  const status: CSSProperties = {
    fontSize: "var(--ts-sm)",
    color: "var(--text-3)",
    display: "inline-flex",
    alignItems: "center",
    gap: "var(--sp-1)",
  };

  const isRunning = !braked && activeRun != null && !activeRun.completed;

  // LF33: the dropdown is a secondary "history" affordance — surface it only when
  // there are completed (past) runs to inspect. The Start/● Running/Stop cluster
  // is the operating surface; the dropdown is for reviewing finished runs.
  const hasHistory = runs.some((r) => r.completed);

  return (
    <div style={bar}>
      <span id="run-selector-label" style={label}>
        history
      </span>
      {loading && runs.length === 0 ? (
        <span style={empty} aria-live="polite">loading runs…</span>
      ) : runs.length === 0 ? (
        <span style={empty}>no runs yet — start a run</span>
      ) : hasHistory ? (
        <select
          aria-labelledby="run-selector-label"
          style={select}
          value={selectedRun?.id ?? ""}
          onChange={(e) => onSelect(e.target.value)}
        >
          {runs.map((r) => (
            <option key={r.id} value={r.id}>
              {runOptionLabel(r, activeRun?.id === r.id)}
            </option>
          ))}
        </select>
      ) : null}
      <div style={{ marginLeft: "auto", display: "flex", alignItems: "center", gap: "var(--sp-3)" }}>
        {braked ? (
          <>
            <span style={status}>stopped</span>
            <Button variant="primary" size="sm" onClick={onResume}>
              Resume
            </Button>
          </>
        ) : isRunning ? (
          <>
            <span style={status}>
              <span aria-hidden="true" style={{ color: "var(--running)" }}>●</span> Running
            </span>
            <Button variant="danger" size="sm" onClick={onStop}>
              Stop
            </Button>
          </>
        ) : (
          <Button variant="primary" size="sm" onClick={onStartRun} disabled={starting}>
            {starting ? "starting…" : "Start run"}
          </Button>
        )}
      </div>
    </div>
  );
}
