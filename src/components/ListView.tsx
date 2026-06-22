import { useState, type CSSProperties } from "react";
import type { Task, TaskState } from "../ipc/runtime";
import { filterTasks, FILTER_PILLS, type FilterPill } from "../lib/listFilter";
import { formatAge } from "../lib/age";
import { formatTokens, costColorVar, costBand } from "../lib/cost";

export interface ListViewProps {
  tasks: Task[];
  tokensByTask: Record<string, number>;
  now: number;
  onOpenCard: (taskId: string) => void;
}

const stateDot: Record<TaskState, string> = {
  queued: "var(--text-3)", running: "var(--running)", gated: "var(--accent)",
  revising: "var(--revise)", needs_human: "var(--danger)", done: "var(--text-3)", braked: "var(--text-4)",
};

export function ListView({ tasks, tokensByTask, now, onOpenCard }: ListViewProps) {
  const [pill, setPill] = useState<FilterPill>("all");
  const [search, setSearch] = useState("");
  const rows = filterTasks(tasks, pill, search, tokensByTask);

  const filters: CSSProperties = { display: "flex", gap: 8, alignItems: "center", padding: "var(--sp-3) var(--sp-8)", flexWrap: "wrap" };
  function pillStyle(p: FilterPill): CSSProperties {
    const active = p === pill;
    return { fontSize: 11, padding: "3px 10px", borderRadius: "var(--r-pill)", border: "1px solid", borderColor: active ? "var(--accent-bd)" : "var(--border)", background: active ? "var(--accent-2)" : "transparent", color: active ? "var(--accent)" : "var(--text-3)", cursor: "pointer", fontFamily: "inherit" };
  }
  const th: CSSProperties = { textAlign: "left", fontSize: 11, fontWeight: 400, color: "var(--text-3)", textTransform: "lowercase", letterSpacing: "0.02em", padding: "6px 10px", borderBottom: "1px solid var(--border)", background: "var(--bg-2)" };
  const td: CSSProperties = { padding: "8px 10px", fontSize: 12, borderBottom: "1px solid var(--border)" };
  const numTd: CSSProperties = { ...td, fontVariantNumeric: "tabular-nums" };

  return (
    <div>
      <div style={filters}>
        {FILTER_PILLS.map((p) => (
          <button key={p} style={pillStyle(p)} onClick={() => setPill(p)}>{p}</button>
        ))}
        <input placeholder="filter tasks…" value={search} onChange={(e) => setSearch(e.target.value)}
          style={{ marginLeft: "auto", background: "var(--surface)", border: "1px solid var(--border)", color: "var(--text)", borderRadius: "var(--r-sm)", padding: "4px 10px", fontSize: 11, fontFamily: "inherit", minWidth: 180 }} />
      </div>
      {rows.length === 0 ? (
        <div style={{ padding: "var(--sp-8)", color: "var(--text-3)", fontSize: 12 }}>no tasks match this filter.</div>
      ) : (
        <table style={{ width: "100%", borderCollapse: "collapse" }}>
          <thead>
            <tr>{["id", "topic", "stage", "state", "age", "tokens", "att", "last"].map((h) => (<th key={h} style={th}>{h}</th>))}</tr>
          </thead>
          <tbody>
            {rows.map((task) => {
              const needsYou = task.state === "gated" || task.state === "needs_human";
              const tokens = tokensByTask[task.id] ?? 0;
              const rowStyle: CSSProperties = { cursor: "pointer", background: needsYou ? "oklch(18% 0.025 55)" : undefined };
              const firstTd: CSSProperties = needsYou ? { ...td, borderLeft: "2px solid var(--accent)", color: "var(--text-3)" } : { ...td, color: "var(--text-3)" };
              return (
                <tr key={task.id} style={rowStyle} onClick={() => onOpenCard(task.id)}>
                  <td style={firstTd}>{task.id}</td>
                  <td style={{ ...td, color: "var(--text)" }}>{task.topic}</td>
                  <td style={td}>{task.current_stage}</td>
                  <td style={td}><span title={task.state} style={{ display: "inline-block", width: 7, height: 7, borderRadius: "50%", background: stateDot[task.state] }} /></td>
                  <td style={numTd}>{formatAge(task.created_at, now)}</td>
                  <td style={{ ...numTd, color: costColorVar[costBand(tokens)] }}>{formatTokens(tokens)}</td>
                  <td style={numTd}>a{task.attempts}</td>
                  <td style={numTd}>{formatAge(task.updated_at, now)}</td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}
    </div>
  );
}
