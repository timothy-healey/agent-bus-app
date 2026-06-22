import type { Task } from "../ipc/runtime";

export type FilterPill =
  | "all"
  | "needs you"
  | "running"
  | "revising"
  | "braked"
  | "cost > 100k";

export const FILTER_PILLS: FilterPill[] = [
  "all", "needs you", "running", "revising", "braked", "cost > 100k",
];

function matchesPill(task: Task, pill: FilterPill, tokens: Record<string, number>): boolean {
  switch (pill) {
    case "all": return true;
    case "needs you": return task.state === "gated" || task.state === "needs_human";
    case "running": return task.state === "running";
    case "revising": return task.state === "revising";
    case "braked": return task.state === "braked";
    case "cost > 100k": return (tokens[task.id] ?? 0) > 100_000;
  }
}

export function filterTasks(
  tasks: Task[],
  pill: FilterPill,
  search: string,
  tokens: Record<string, number>,
): Task[] {
  const q = search.trim().toLowerCase();
  return tasks.filter((task) => {
    if (!matchesPill(task, pill, tokens)) return false;
    if (!q) return true;
    return (
      task.id.toLowerCase().includes(q) || task.topic.toLowerCase().includes(q)
    );
  });
}
