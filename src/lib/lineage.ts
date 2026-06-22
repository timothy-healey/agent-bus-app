import type { Task } from "../ipc/runtime";

export type LineageKind = "topic" | "artifact" | "review";

export interface LineageEntry {
  label: string;
  /// project-root-relative artifact path, or null for the topic root.
  path: string | null;
  kind: LineageKind;
}

/// Derive the upstream chain for a task from its artifact pointers (D8). v1:
/// topic (root) -> parent_artifact (the produced spec/plan) -> review_artifact
/// (the review of it). A full per-version history is v1.1.
export function buildLineage(task: Task): LineageEntry[] {
  const chain: LineageEntry[] = [{ label: "topic", path: null, kind: "topic" }];
  const seen = new Set<string>();
  function push(path: string | null, kind: LineageKind, label: string) {
    if (!path || seen.has(path)) return;
    seen.add(path);
    chain.push({ label, path, kind });
  }
  push(task.parent_artifact, "artifact", basename(task.parent_artifact));
  push(task.review_artifact, "review", basename(task.review_artifact));
  return chain;
}

function basename(path: string | null): string {
  if (!path) return "";
  const parts = path.split("/");
  return parts[parts.length - 1] || path;
}
