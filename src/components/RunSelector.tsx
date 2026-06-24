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
  /// True while a Start is in flight — disables the button + shows progress.
  starting?: boolean;
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
export function RunSelector({ runs, selectedRun, activeRun, onSelect, onStartRun, starting = false }: RunSelectorProps) {
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

  return (
    <div style={bar}>
      <span id="run-selector-label" style={label}>
        run
      </span>
      {runs.length === 0 ? (
        <span style={empty}>no runs yet — start a run</span>
      ) : (
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
      )}
      <div style={{ marginLeft: "auto" }}>
        <Button variant="primary" size="sm" onClick={onStartRun} disabled={starting}>
          {starting ? "starting…" : "Start run"}
        </Button>
      </div>
    </div>
  );
}
