import { useCallback, useEffect, useRef, useState, type CSSProperties, type KeyboardEvent } from "react";
import { listDir, type DirEntry } from "../ipc/workspace";
import { toggleSelection } from "./fileTree";

/// FileTreePicker (G7) — a themed, in-app file/folder browser that lazily
/// expands directories via the `list_dir` OHS command. Two modes:
///   - "folder" (single-select, directories only) for root/target picks
///   - "files"  (multi-select, files + folders) for scope reads/writes
/// Rooted at `root`. Returns absolute paths; the caller maps them to repo-
/// relative where needed (scope). It depends on `listDir`, never std/fs.
/// Accessible: a real ARIA tree (role="tree"/"treeitem"/"group"), keyboard
/// reachable, focus-visible inherited from the global ring.

export type FileTreeMode = "folder" | "files";

export interface FileTreePickerProps {
  /// Absolute base path the tree is rooted at (e.g. the home dir, or the
  /// project's target_repo for scope).
  root: string;
  mode: FileTreeMode;
  /// Currently-selected absolute paths (controlled).
  selected: string[];
  onChange: (selected: string[]) => void;
  /// Accessible label for the tree.
  label?: string;
}

interface NodeState {
  entries: DirEntry[] | null; // null = not loaded
  loading: boolean;
  error: string | null;
}

export function FileTreePicker({ root, mode, selected, onChange, label = "file tree" }: FileTreePickerProps) {
  const multi = mode === "files";
  // Per-directory loaded children + expand state, keyed by absolute path.
  const [nodes, setNodes] = useState<Record<string, NodeState>>({});
  const [expanded, setExpanded] = useState<Record<string, boolean>>({});
  // Guard against setState after unmount during the async load.
  const alive = useRef(true);
  useEffect(() => () => { alive.current = false; }, []);

  const load = useCallback(async (path: string) => {
    setNodes((n) => ({ ...n, [path]: { entries: null, loading: true, error: null } }));
    try {
      const entries = await listDir(path);
      if (!alive.current) return;
      setNodes((n) => ({ ...n, [path]: { entries, loading: false, error: null } }));
    } catch (e) {
      if (!alive.current) return;
      setNodes((n) => ({ ...n, [path]: { entries: null, loading: false, error: String(e) } }));
    }
  }, []);

  // Load the root on mount / when it changes.
  useEffect(() => {
    setNodes({});
    setExpanded({});
    if (root) void load(root);
  }, [root, load]);

  const toggleExpand = useCallback((path: string) => {
    setExpanded((e) => {
      const next = !e[path];
      if (next && !nodes[path]) void load(path);
      return { ...e, [path]: next };
    });
  }, [nodes, load]);

  function pick(entry: DirEntry) {
    // folder mode: only directories are selectable.
    if (mode === "folder" && !entry.is_dir) return;
    onChange(toggleSelection(selected, entry.path, multi));
  }

  function rowKeyDown(e: KeyboardEvent, entry: DirEntry) {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      pick(entry);
    } else if (e.key === "ArrowRight" && entry.is_dir && !expanded[entry.path]) {
      e.preventDefault();
      toggleExpand(entry.path);
    } else if (e.key === "ArrowLeft" && entry.is_dir && expanded[entry.path]) {
      e.preventDefault();
      toggleExpand(entry.path);
    }
  }

  const renderLevel = (path: string, depth: number) => {
    const node = nodes[path];
    if (!node) return null;
    if (node.loading) return <li role="none" style={hint(depth)}>Loading…</li>;
    if (node.error) return <li role="none" style={{ ...hint(depth), color: "var(--danger)" }}>{node.error}</li>;
    const entries = node.entries ?? [];
    if (entries.length === 0) return <li role="none" style={hint(depth)}>empty</li>;
    return entries.map((entry) => {
      const isSel = selected.includes(entry.path);
      const isExp = !!expanded[entry.path];
      const selectable = mode === "files" || entry.is_dir;
      return (
        <li key={entry.path} role="none">
          <div
            role="treeitem"
            aria-expanded={entry.is_dir ? isExp : undefined}
            aria-selected={selectable ? isSel : undefined}
            tabIndex={0}
            style={row(depth, isSel)}
            onKeyDown={(e) => rowKeyDown(e, entry)}
          >
            {entry.is_dir ? (
              <button
                type="button"
                aria-label={isExp ? `collapse ${entry.name}` : `expand ${entry.name}`}
                onClick={(e) => { e.stopPropagation(); toggleExpand(entry.path); }}
                style={twisty}
              >
                {isExp ? "▾" : "▸"}
              </button>
            ) : (
              <span style={twisty} aria-hidden />
            )}
            {multi && selectable && (
              <input
                type="checkbox"
                aria-label={`select ${entry.name}`}
                checked={isSel}
                onChange={() => pick(entry)}
                onClick={(e) => e.stopPropagation()}
                style={{ margin: 0 }}
              />
            )}
            <span
              onClick={() => pick(entry)}
              style={{
                cursor: selectable ? "pointer" : "default",
                flex: 1,
                fontFamily: "var(--font-mono)",
                color: entry.is_dir ? "var(--text-2)" : "var(--text-3)",
              }}
            >
              {entry.name}{entry.is_dir ? "/" : ""}
            </span>
          </div>
          {entry.is_dir && isExp && (
            <ul role="group" style={group}>
              {renderLevel(entry.path, depth + 1)}
            </ul>
          )}
        </li>
      );
    });
  };

  return (
    <div style={shell}>
      <ul role="tree" aria-label={label} aria-multiselectable={multi} style={treeRoot}>
        {renderLevel(root, 0)}
      </ul>
    </div>
  );
}

const shell: CSSProperties = {
  border: "1px solid var(--border)",
  borderRadius: "var(--r-sm)",
  background: "var(--bg-2)",
  maxHeight: 280,
  overflowY: "auto",
  padding: "var(--sp-2)",
};
const treeRoot: CSSProperties = { listStyle: "none", margin: 0, padding: 0 };
const group: CSSProperties = { listStyle: "none", margin: 0, padding: 0 };
function row(depth: number, selected: boolean): CSSProperties {
  return {
    display: "flex",
    alignItems: "center",
    gap: "var(--sp-2)",
    padding: "2px var(--sp-2)",
    paddingLeft: `calc(${depth} * var(--sp-4) + var(--sp-2))`,
    borderRadius: "var(--r-sm)",
    fontSize: "var(--ts-base)",
    background: selected ? "var(--accent-2)" : "transparent",
  };
}
function hint(depth: number): CSSProperties {
  return {
    paddingLeft: `calc(${depth + 1} * var(--sp-4) + var(--sp-2))`,
    color: "var(--text-3)",
    fontSize: "var(--ts-sm)",
    fontStyle: "italic",
    listStyle: "none",
  };
}
const twisty: CSSProperties = {
  width: 16,
  background: "transparent",
  border: "none",
  color: "var(--text-3)",
  cursor: "pointer",
  fontSize: 11,
  padding: 0,
  fontFamily: "inherit",
};
