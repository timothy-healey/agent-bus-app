import type { CSSProperties } from "react";
import { useState } from "react";
import type { Project } from "../ipc/workspace";
import { Button } from "./ui/Button";

export interface ProjectSwitcherProps {
  projects: Project[];
  activeProjectId: string | null;
  /// Switch the active project. Omit to render the switcher read-only (e.g. the
  /// new-project flow where there is nothing to switch *to* yet).
  onSelect?: (id: string) => void;
  /// Delete a project (with the confirm handled here). Calls the existing
  /// `removeProject` path (G13). Omit to hide the delete affordance.
  onDelete?: (id: string) => void | Promise<void>;
}

/// The project switcher (G13/G14): a labelled select of the operator's projects
/// (active marked) plus a destructive delete with an inline confirm. Lives in the
/// authoring layout's left nav column so projects are switched/deleted where they
/// are actually seen, and is kept in Settings too (the Settings list is the other
/// home for remove).
export function ProjectSwitcher({ projects, activeProjectId, onSelect, onDelete }: ProjectSwitcherProps) {
  const [confirming, setConfirming] = useState(false);
  const active = projects.find((p) => p.id === activeProjectId) ?? null;

  async function doDelete() {
    if (!active || !onDelete) return;
    // onDelete surfaces its own failure (App's handleDeleteProject catches +
    // shows the alert); we swallow here so a rejecting caller can't produce an
    // unhandled rejection, and reset the confirm row either way.
    try {
      await onDelete(active.id);
    } catch {
      // surfaced by the caller; nothing to do here.
    } finally {
      setConfirming(false);
    }
  }

  return (
    <div style={wrap}>
      <span id="project-switcher-label" style={label}>project</span>
      {projects.length === 0 ? (
        <span style={empty}>no projects yet</span>
      ) : (
        <select
          aria-labelledby="project-switcher-label"
          style={select}
          value={activeProjectId ?? ""}
          disabled={!onSelect}
          onChange={(e) => onSelect?.(e.target.value)}
        >
          {projects.map((p) => (
            <option key={p.id} value={p.id}>
              {p.name}{p.id === activeProjectId ? " (active)" : ""}
            </option>
          ))}
        </select>
      )}

      {onDelete && active && (
        confirming ? (
          <div style={confirmRow}>
            <span style={confirmText}>
              Delete “{active.name}”? This also deletes the project's files
              (prompts, pipelines, artifacts, worktrees). The target repo is NOT
              touched.
            </span>
            <Button variant="danger" size="sm" onClick={doDelete} aria-label={`confirm delete ${active.name}`}>delete</Button>
            <Button variant="ghost" size="sm" onClick={() => setConfirming(false)}>cancel</Button>
          </div>
        ) : (
          <Button
            variant="ghost"
            size="sm"
            onClick={() => setConfirming(true)}
            aria-label={`delete project ${active.name}`}
          >
            delete project
          </Button>
        )
      )}
    </div>
  );
}

const wrap: CSSProperties = { display: "flex", flexDirection: "column", gap: "var(--sp-2)" };
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
  width: "100%",
};
const empty: CSSProperties = { fontSize: "var(--ts-sm)", color: "var(--text-3)" };
const confirmRow: CSSProperties = { display: "flex", flexWrap: "wrap", alignItems: "center", gap: "var(--sp-2)" };
const confirmText: CSSProperties = { fontSize: "var(--ts-sm)", color: "var(--text-2)" };
