import { type FormEvent, type CSSProperties, useState } from "react";
import { createProject, type Project } from "../ipc/workspace";

export interface ProjectWizardProps {
  open: boolean;
  onClose: () => void;
  onCreated: (p: Project) => void;
}

export function ProjectWizard({ open, onClose, onCreated }: ProjectWizardProps) {
  const [name, setName] = useState("");
  const [rootPath, setRootPath] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  if (!open) return null;

  const canSubmit = name.trim().length > 0 && rootPath.trim().length > 0 && !submitting;

  async function handleSubmit(e: FormEvent) {
    e.preventDefault();
    if (!canSubmit) return;
    setSubmitting(true);
    setError(null);
    try {
      const project = await createProject({ name: name.trim(), root_path: rootPath.trim() });
      onCreated(project);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <div
      role="dialog"
      aria-modal="true"
      style={{
        position: "fixed",
        inset: 0,
        background: "rgba(0,0,0,0.5)",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        zIndex: 100,
      }}
      onClick={onClose}
    >
      <form
        onSubmit={handleSubmit}
        onClick={(e) => e.stopPropagation()}
        style={{
          background: "var(--surface)",
          border: "1px solid var(--border)",
          borderRadius: "var(--r-md)",
          padding: "var(--sp-8)",
          minWidth: 420,
          boxShadow: "var(--shadow-card)",
        }}
      >
        <h2 style={{ margin: 0, fontSize: 14, color: "var(--text)" }}>New project</h2>
        <p style={{ color: "var(--text-3)", fontSize: 11, marginTop: "var(--sp-1)", marginBottom: "var(--sp-7)" }}>
          Pick a name and a root directory. The directory will hold pipelines, artifacts, and worktrees.
        </p>

        <label style={{ display: "block", fontSize: 11, color: "var(--text-3)", marginBottom: 4 }}>
          Project name
        </label>
        <input
          aria-label="Project name"
          value={name}
          onChange={(e) => setName(e.target.value)}
          style={inputStyle}
        />

        <label style={{ display: "block", fontSize: 11, color: "var(--text-3)", marginBottom: 4, marginTop: "var(--sp-4)" }}>
          Root path
        </label>
        <input
          aria-label="Root path"
          value={rootPath}
          onChange={(e) => setRootPath(e.target.value)}
          placeholder="/Users/you/projects/example"
          style={inputStyle}
        />

        {error && (
          <div style={{ color: "var(--danger)", fontSize: 11, marginTop: "var(--sp-3)" }}>{error}</div>
        )}

        <div style={{ display: "flex", justifyContent: "flex-end", gap: "var(--sp-2)", marginTop: "var(--sp-7)" }}>
          <button type="button" onClick={onClose} style={ghostButton}>Cancel</button>
          <button type="submit" disabled={!canSubmit} style={canSubmit ? primaryButton : disabledButton}>
            Create project
          </button>
        </div>
      </form>
    </div>
  );
}

const inputStyle: CSSProperties = {
  width: "100%",
  background: "var(--bg-2)",
  border: "1px solid var(--border)",
  color: "var(--text)",
  padding: "var(--sp-2) var(--sp-3)",
  fontFamily: "inherit",
  fontSize: 12,
  borderRadius: "var(--r-sm)",
};

const primaryButton: CSSProperties = {
  background: "var(--accent)",
  border: "1px solid var(--accent)",
  color: "oklch(15% 0.04 55)",
  padding: "var(--sp-2) var(--sp-4)",
  fontFamily: "inherit",
  fontSize: 12,
  borderRadius: "var(--r-sm)",
  cursor: "pointer",
  fontWeight: 500,
};

const ghostButton: CSSProperties = {
  background: "transparent",
  border: "1px solid var(--border)",
  color: "var(--text-2)",
  padding: "var(--sp-2) var(--sp-4)",
  fontFamily: "inherit",
  fontSize: 12,
  borderRadius: "var(--r-sm)",
  cursor: "pointer",
};

const disabledButton: CSSProperties = {
  ...primaryButton,
  opacity: 0.5,
  cursor: "not-allowed",
};
