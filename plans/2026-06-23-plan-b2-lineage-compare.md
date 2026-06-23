# B2 — Lineage Side-by-Side Compare Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** In the CardDrawer Lineage tab, let the operator pick two artifact-bearing lineage entries and see both artifacts' rendered markdown side-by-side, with a tasteful line-level added/removed diff highlight.

**Architecture:** Pure frontend read-projection over the existing Workspace `read_artifact` OHS command + the lineage chain already derived from `Task` (`parent_artifact`/`review_artifact`). No backend, no new persisted concept, no business logic in the view. A new pure `lib/lineDiff.ts` (LCS line classifier) and a `CompareView` presentational component reuse the existing `parseMarkdown` block renderer. `LineageTab` gains a select-two compare mode and raises `onCompare(a, b)`; `App.tsx` reads the second artifact via `readArtifact` (mirroring the existing single-pane load) and passes both bodies to `CompareView`.

**Tech Stack:** React + TypeScript, Vitest + React Testing Library, existing theme CSS custom properties (`--bg-2`, `--border`, `--accent`, `--text-*`, `--r-sm`). Markdown via existing `src/lib/markdown.ts`.

---

## Decisions

- **DD1 (compare UX) = A (recommended):** Compare lives *inside the Lineage tab*, not as a new top-level tab or drawer. A "compare" toggle turns each artifact-bearing lineage row into a selectable item (checkbox-style button); selecting exactly two renders a two-column `CompareView` in the same scroll area. Rationale: lineage is already the place you reason about versions; keeps the drawer's 4-tab IA unchanged; the impeccable pass will polish.
- **DD2 (diff) = A (recommended, nice-to-have, tasteful):** A line-level added/removed highlight via a pure LCS classifier (`lib/lineDiff.ts`). Each pane's raw lines are tinted: removed (only-in-left) and added (only-in-right) get a subtle token-driven background; unchanged lines render normally. We do NOT diff the rendered blocks (too clever) — we tint the *source lines* shown in a monospace diff column that sits alongside the rendered markdown is rejected as over-built; instead CompareView shows the rendered markdown per pane (primary), and the diff tint is applied to a lightweight per-line source view toggle. **Refined for v1.1 (keep simple):** CompareView renders each pane as rendered markdown by default; the line-diff is exposed as the same component computing changed-line counts and tinting is applied at the *block* level only where a block's source text changed. To avoid gold-plating, the shipped diff is: compute `lineDiff(leftLines, rightLines)`; expose `changed` counts in a small header badge per pane ("+3 / -1"); tint nothing structurally beyond the badge. A full inline line-tint is left for the impeccable pass. This keeps the diff *present and useful* (the badge tells you how different two versions are) without a fragile inline-highlight that the polish pass would redo.
- **DD3 (data flow) = A (recommended):** No backend. `App.tsx` holds a `comparePath` + `compareMarkdown` alongside the existing `lineagePath`/`artifactMarkdown`, loaded by the same `readArtifact` effect pattern. `LineageTab` raises `onCompare(pathA, pathB)` and `onExitCompare()`. Reuse the lineage chain; only entries with a `path` are selectable.
- **DD4 (where the two-column render lives) = A:** A new `CompareView` presentational component (`src/components/CompareView.tsx`) takes `left`/`right` `{ label, markdown }` and renders two columns reusing `parseMarkdown` + a shared block renderer extracted from `ArtifactView`. To avoid duplicating the renderer, extract the pure `renderBlock` switch into `src/lib/renderBlock.tsx` and have BOTH `ArtifactView` and `CompareView` import it (DRY; no behavior change to ArtifactView). Compare panes are read-only (no comment popover) — that's an ArtifactView-only concern.
- **DD5 (selection cap) = A:** Exactly two selections enable compare. Selecting a third replaces the *oldest* selection (FIFO of size 2) so the UI never dead-ends.

---

## File Structure

- **Create `src/lib/lineDiff.ts`** — pure LCS line classifier: `lineDiff(left: string, right: string): { added: number; removed: number; lines: DiffLine[] }`. One responsibility: compare two markdown bodies by line.
- **Create `src/lib/lineDiff.test.ts`** — unit tests for the classifier.
- **Create `src/lib/renderBlock.tsx`** — the block-render switch extracted verbatim from `ArtifactView` (DRY; shared by ArtifactView + CompareView).
- **Modify `src/components/ArtifactView.tsx`** — import `renderBlock` from the new module instead of its local copy (no behavior change).
- **Create `src/components/CompareView.tsx`** — two-column side-by-side render of two `{label, markdown}` panes, each with a `+N / -N` diff badge.
- **Create `src/components/CompareView.test.tsx`** — RTL tests.
- **Modify `src/components/LineageTab.tsx`** — add compare mode (toggle, select-two, raise `onCompare`/`onExitCompare`).
- **Modify `src/components/LineageTab.test.tsx`** — cover compare mode (keep existing tests green).
- **Modify `src/components/CardDrawer.tsx`** — thread `compareMarkdown` + `onCompare`/`onExitCompare` into `LineageTab`/`CompareView`.
- **Modify `src/App.tsx`** — `comparePath`/`compareMarkdown` state + a `readArtifact` effect; pass to CardDrawer.

---

## Task 1: Pure line-diff classifier

**Files:**
- Create: `src/lib/lineDiff.ts`
- Test: `src/lib/lineDiff.test.ts`

- [ ] **Step 1: Write the failing test**

```ts
// src/lib/lineDiff.test.ts
import { describe, expect, it } from "vitest";
import { lineDiff } from "./lineDiff";

describe("lineDiff", () => {
  it("reports zero changes for identical bodies", () => {
    const d = lineDiff("a\nb\nc", "a\nb\nc");
    expect(d.added).toBe(0);
    expect(d.removed).toBe(0);
    expect(d.lines.every((l) => l.kind === "same")).toBe(true);
  });

  it("counts added and removed lines", () => {
    const d = lineDiff("a\nb\nc", "a\nx\nc\nd");
    // b removed, x + d added
    expect(d.removed).toBe(1);
    expect(d.added).toBe(2);
  });

  it("classifies each line with its source side", () => {
    const d = lineDiff("keep\nold", "keep\nnew");
    const kinds = d.lines.map((l) => `${l.kind}:${l.text}`);
    expect(kinds).toContain("same:keep");
    expect(kinds).toContain("removed:old");
    expect(kinds).toContain("added:new");
  });

  it("normalises CRLF and treats trailing newline as no extra line", () => {
    const d = lineDiff("a\r\nb\r\n", "a\nb\n");
    expect(d.added).toBe(0);
    expect(d.removed).toBe(0);
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `/opt/homebrew/bin/bun vitest run src/lib/lineDiff.test.ts`
Expected: FAIL — "Failed to resolve import './lineDiff'".

- [ ] **Step 3: Write minimal implementation**

```ts
// src/lib/lineDiff.ts

/// One classified line of a two-version comparison.
export interface DiffLine {
  kind: "same" | "added" | "removed";
  text: string;
}

export interface LineDiff {
  added: number;
  removed: number;
  lines: DiffLine[];
}

function split(src: string): string[] {
  const norm = src.replace(/\r\n/g, "\n");
  const lines = norm.split("\n");
  // A single trailing newline yields a final "" — drop it so it isn't a "line".
  if (lines.length > 1 && lines[lines.length - 1] === "") lines.pop();
  return lines;
}

/// Classic LCS line diff. Pure: classifies every line of `left`/`right` as
/// same / removed (only in left) / added (only in right). Read-only over the
/// two markdown bodies — no markdown semantics, just line identity.
export function lineDiff(left: string, right: string): LineDiff {
  const a = split(left);
  const b = split(right);
  const n = a.length;
  const m = b.length;

  // LCS length table.
  const lcs: number[][] = Array.from({ length: n + 1 }, () =>
    new Array<number>(m + 1).fill(0),
  );
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      lcs[i][j] = a[i] === b[j]
        ? lcs[i + 1][j + 1] + 1
        : Math.max(lcs[i + 1][j], lcs[i][j + 1]);
    }
  }

  const lines: DiffLine[] = [];
  let added = 0;
  let removed = 0;
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (a[i] === b[j]) {
      lines.push({ kind: "same", text: a[i] });
      i++;
      j++;
    } else if (lcs[i + 1][j] >= lcs[i][j + 1]) {
      lines.push({ kind: "removed", text: a[i] });
      removed++;
      i++;
    } else {
      lines.push({ kind: "added", text: b[j] });
      added++;
      j++;
    }
  }
  while (i < n) {
    lines.push({ kind: "removed", text: a[i] });
    removed++;
    i++;
  }
  while (j < m) {
    lines.push({ kind: "added", text: b[j] });
    added++;
    j++;
  }

  return { added, removed, lines };
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `/opt/homebrew/bin/bun vitest run src/lib/lineDiff.test.ts`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add src/lib/lineDiff.ts src/lib/lineDiff.test.ts
git commit -m "feat(lineage): pure LCS line-diff classifier for version compare (B2)"
```

---

## Task 2: Extract the block renderer (DRY) — no behavior change

**Files:**
- Create: `src/lib/renderBlock.tsx`
- Modify: `src/components/ArtifactView.tsx` (replace local `renderBlock` with the import)

- [ ] **Step 1: Create the shared renderer module**

Copy the existing `renderBlock` function VERBATIM from `ArtifactView.tsx` (lines 18-94, the `function renderBlock(b: Block, i: number) { ... }` switch) into a new module, exporting it. Include the `Block` import.

```tsx
// src/lib/renderBlock.tsx
import type { Block } from "./markdown";

/// Shared CSP-safe block renderer for agent-artifact markdown. Used by the
/// single-pane ArtifactView and the two-column CompareView (B2). Pure render —
/// no comment / selection behaviour (that stays ArtifactView-only).
export function renderBlock(b: Block, i: number) {
  const codeFont = "'Berkeley Mono','JetBrains Mono',ui-monospace,monospace";
  switch (b.type) {
    case "frontmatter":
      return (
        <pre
          key={i}
          style={{
            background: "var(--bg-2)",
            border: "1px solid var(--border)",
            borderRadius: "var(--r-sm)",
            padding: "10px 12px",
            fontFamily: codeFont,
            fontSize: 12,
            color: "var(--text-3)",
            marginBottom: 18,
            whiteSpace: "pre-wrap",
          }}
        >
          {b.text}
        </pre>
      );
    case "heading": {
      const sizes = { 1: 19, 2: 16, 3: 14 } as const;
      const Tag = (`h${b.level}` as unknown) as "h1";
      return (
        <Tag
          key={i}
          style={{
            color: "var(--text)",
            fontWeight: 600,
            fontSize: sizes[b.level],
            lineHeight: 1.3,
            margin: b.level === 1 ? "0 0 4px" : "22px 0 8px",
          }}
        >
          {b.text}
        </Tag>
      );
    }
    case "code":
      return (
        <pre
          key={i}
          style={{
            background: "var(--bg-2)",
            border: "1px solid var(--border)",
            borderRadius: "var(--r-sm)",
            padding: "10px 12px",
            fontFamily: codeFont,
            fontSize: 12,
            color: "var(--text-2)",
            overflowX: "auto",
            margin: "8px 0 14px",
          }}
        >
          {b.text}
        </pre>
      );
    case "list":
      return (
        <ul key={i} style={{ margin: "8px 0 12px", paddingLeft: 22 }}>
          {b.items.map((it, j) => (
            <li key={j} style={{ margin: "3px 0" }}>
              {it}
            </li>
          ))}
        </ul>
      );
    case "paragraph":
      return (
        <p key={i} style={{ margin: "0 0 10px" }}>
          {b.text}
        </p>
      );
  }
}
```

- [ ] **Step 2: Rewire ArtifactView to import it**

In `src/components/ArtifactView.tsx`: delete the local `function renderBlock(...) { ... }` (lines 18-94) and add the import near the top:

```tsx
import { parseMarkdown, type Block } from "../lib/markdown";
import { renderBlock } from "../lib/renderBlock";
import { SelectionPopover } from "./SelectionPopover";
```

Note: `Block` may no longer be referenced in ArtifactView after the extraction — if `bun run build` (tsc) reports `Block` unused, drop it from the import. (It is still used by `parseMarkdown`'s return type implicitly; verify in Step 3.)

- [ ] **Step 3: Run the existing ArtifactView tests to verify no behavior change**

Run: `/opt/homebrew/bin/bun vitest run src/components/ArtifactView.test.tsx`
Expected: PASS (3 tests, unchanged).

- [ ] **Step 4: Typecheck**

Run: `/opt/homebrew/bin/bun run build`
Expected: build succeeds. If tsc flags `Block` as unused in ArtifactView, change the import to `import { parseMarkdown } from "../lib/markdown";` and re-run.

- [ ] **Step 5: Commit**

```bash
git add src/lib/renderBlock.tsx src/components/ArtifactView.tsx
git commit -m "refactor(artifact): extract shared renderBlock module (B2 prep, no behavior change)"
```

---

## Task 3: CompareView two-column component

**Files:**
- Create: `src/components/CompareView.tsx`
- Test: `src/components/CompareView.test.tsx`

- [ ] **Step 1: Write the failing test**

```tsx
// src/components/CompareView.test.tsx
import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { CompareView } from "./CompareView";

describe("CompareView", () => {
  it("renders both pane labels and bodies side by side", () => {
    render(
      <CompareView
        left={{ label: "T-1-v1.md", markdown: "# Spec\n\noriginal body" }}
        right={{ label: "T-1-v2.md", markdown: "# Spec\n\nrevised body" }}
      />,
    );
    expect(screen.getByText("T-1-v1.md")).toBeInTheDocument();
    expect(screen.getByText("T-1-v2.md")).toBeInTheDocument();
    expect(screen.getByText(/original body/)).toBeInTheDocument();
    expect(screen.getByText(/revised body/)).toBeInTheDocument();
    // Two distinct rendered headings, one per pane.
    expect(screen.getAllByRole("heading", { level: 1, name: "Spec" })).toHaveLength(2);
  });

  it("shows a per-side change badge derived from the line diff", () => {
    render(
      <CompareView
        left={{ label: "v1", markdown: "a\nb\nc" }}
        right={{ label: "v2", markdown: "a\nx\nc\nd" }}
      />,
    );
    // right pane: +2 added (x, d), left pane: -1 removed (b)
    expect(screen.getByTestId("compare-badge-right")).toHaveTextContent("+2");
    expect(screen.getByTestId("compare-badge-left")).toHaveTextContent("-1");
  });

  it("renders empty-state text for an empty pane body", () => {
    render(
      <CompareView
        left={{ label: "v1", markdown: "" }}
        right={{ label: "v2", markdown: "# X" }}
      />,
    );
    expect(screen.getByText(/no artifact/i)).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `/opt/homebrew/bin/bun vitest run src/components/CompareView.test.tsx`
Expected: FAIL — cannot resolve `./CompareView`.

- [ ] **Step 3: Write minimal implementation**

```tsx
// src/components/CompareView.tsx
import type { CSSProperties } from "react";
import { parseMarkdown } from "../lib/markdown";
import { renderBlock } from "../lib/renderBlock";
import { lineDiff } from "../lib/lineDiff";

export interface ComparePane {
  label: string;
  markdown: string;
}

export interface CompareViewProps {
  left: ComparePane;
  right: ComparePane;
}

/// Read-only two-column side-by-side render of two artifact versions (B2).
/// Reuses the shared markdown block renderer; a per-side change badge is
/// derived from the pure line diff. No comment/selection affordance (that is
/// ArtifactView-only) — this is a pure read projection over read_artifact.
export function CompareView({ left, right }: CompareViewProps) {
  const diff = lineDiff(left.markdown, right.markdown);

  const wrap: CSSProperties = {
    flex: 1,
    display: "flex",
    minHeight: 0,
    overflow: "hidden",
  };
  const col: CSSProperties = {
    flex: 1,
    minWidth: 0,
    display: "flex",
    flexDirection: "column",
  };
  const divider: CSSProperties = {
    width: 1,
    background: "var(--border)",
    flexShrink: 0,
  };
  const colHead: CSSProperties = {
    display: "flex",
    alignItems: "center",
    gap: 8,
    padding: "8px 14px",
    borderBottom: "1px solid var(--border)",
    background: "var(--bg-2)",
  };
  const body: CSSProperties = {
    flex: 1,
    overflowY: "auto",
    padding: "14px 18px",
    fontFamily: "ui-sans-serif, -apple-system, system-ui, sans-serif",
    fontSize: 13,
    lineHeight: 1.6,
    color: "var(--text-2)",
  };

  function badge(side: "left" | "right", added: number, removed: number): CSSProperties {
    void side;
    return {
      marginLeft: "auto",
      fontSize: 10,
      fontFamily: "'Berkeley Mono','JetBrains Mono',ui-monospace,monospace",
      color: added + removed > 0 ? "var(--accent)" : "var(--text-4)",
    };
  }

  function pane(p: ComparePane, side: "left" | "right") {
    // Per-side badge: the left pane "owns" removals, the right pane additions.
    const added = side === "right" ? diff.added : 0;
    const removed = side === "left" ? diff.removed : 0;
    return (
      <div style={col}>
        <div style={colHead}>
          <span style={{ color: "var(--text-2)", fontSize: 11.5 }}>{p.label}</span>
          <span data-testid={`compare-badge-${side}`} style={badge(side, added, removed)}>
            {removed > 0 ? `-${removed}` : ""}
            {removed > 0 && added > 0 ? " " : ""}
            {added > 0 ? `+${added}` : ""}
            {added + removed === 0 ? "no changes" : ""}
          </span>
        </div>
        {p.markdown.trim() ? (
          <div style={body}>{parseMarkdown(p.markdown).map(renderBlock)}</div>
        ) : (
          <div style={{ ...body, color: "var(--text-4)", fontStyle: "italic", fontSize: 12 }}>
            no artifact at this version.
          </div>
        )}
      </div>
    );
  }

  return (
    <div style={wrap} data-testid="compare-view">
      {pane(left, "left")}
      <div style={divider} />
      {pane(right, "right")}
    </div>
  );
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `/opt/homebrew/bin/bun vitest run src/components/CompareView.test.tsx`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add src/components/CompareView.tsx src/components/CompareView.test.tsx
git commit -m "feat(lineage): CompareView two-column side-by-side artifact render (B2)"
```

---

## Task 4: LineageTab compare mode (select two)

**Files:**
- Modify: `src/components/LineageTab.tsx`
- Modify: `src/components/LineageTab.test.tsx`

- [ ] **Step 1: Write the failing tests (append to LineageTab.test.tsx)**

Add these to the existing `describe("LineageTab", ...)` block (keep all existing tests):

```tsx
  it("enters compare mode and calls onCompare once two entries are selected", () => {
    const onCompare = vi.fn();
    render(
      <LineageTab
        task={t({ parent_artifact: "artifacts/specs/T-1-v1.md", review_artifact: "artifacts/reviews/T-1-rev.md" })}
        onOpenArtifact={() => {}}
        onCompare={onCompare}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: /^compare/i }));
    // In compare mode each artifact entry is a selectable checkbox.
    fireEvent.click(screen.getByRole("checkbox", { name: /T-1-v1\.md/ }));
    expect(onCompare).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("checkbox", { name: /T-1-rev\.md/ }));
    expect(onCompare).toHaveBeenCalledWith("artifacts/specs/T-1-v1.md", "artifacts/reviews/T-1-rev.md");
  });

  it("does not offer compare when fewer than two artifacts exist", () => {
    render(
      <LineageTab
        task={t({ parent_artifact: "artifacts/specs/T-1-v1.md" })}
        onOpenArtifact={() => {}}
        onCompare={() => {}}
      />,
    );
    expect(screen.queryByRole("button", { name: /^compare/i })).not.toBeInTheDocument();
  });
```

- [ ] **Step 2: Run to verify failure**

Run: `/opt/homebrew/bin/bun vitest run src/components/LineageTab.test.tsx`
Expected: FAIL — `onCompare` prop missing / no "compare" button / no checkboxes.

- [ ] **Step 3: Implement compare mode in LineageTab**

Replace the entire body of `src/components/LineageTab.tsx` with:

```tsx
import { useState, type CSSProperties } from "react";
import type { Task } from "../ipc/runtime";
import { buildLineage } from "../lib/lineage";

export interface LineageTabProps {
  task: Task;
  /// Open a single artifact path in the artifact pane (D5: single-pane).
  onOpenArtifact: (path: string) => void;
  /// Enter side-by-side compare for two chosen artifact paths (B2).
  onCompare?: (pathA: string, pathB: string) => void;
}

export function LineageTab({ task, onOpenArtifact, onCompare }: LineageTabProps) {
  const chain = buildLineage(task);
  const hasArtifacts = chain.some((e) => e.path);
  const artifactCount = chain.filter((e) => e.path).length;
  const canCompare = onCompare != null && artifactCount >= 2;

  const [comparing, setComparing] = useState(false);
  // Up to two selected paths, FIFO (DD5): a third pick drops the oldest.
  const [picked, setPicked] = useState<string[]>([]);

  function toggle(path: string) {
    setPicked((prev) => {
      if (prev.includes(path)) return prev.filter((p) => p !== path);
      const next = [...prev, path].slice(-2);
      if (next.length === 2) onCompare?.(next[0], next[1]);
      return next;
    });
  }

  function exitCompare() {
    setComparing(false);
    setPicked([]);
  }

  const row: CSSProperties = {
    display: "flex", alignItems: "baseline", gap: 10, padding: "8px 0",
    borderBottom: "1px solid var(--border)",
  };
  const dot: CSSProperties = { color: "var(--text-4)", fontSize: 10 };

  return (
    <div style={{ flex: 1, padding: "14px 18px", overflowY: "auto" }}>
      {canCompare && (
        <div style={{ display: "flex", alignItems: "center", gap: 10, marginBottom: 8 }}>
          <button
            onClick={() => (comparing ? exitCompare() : setComparing(true))}
            style={{
              background: comparing ? "var(--accent)" : "none",
              border: "1px solid var(--border)",
              borderRadius: "var(--r-sm)",
              padding: "3px 10px", cursor: "pointer",
              color: comparing ? "var(--bg)" : "var(--text-2)",
              fontSize: 11, fontFamily: "inherit",
            }}
          >
            {comparing ? "exit compare" : "compare versions"}
          </button>
          {comparing && (
            <span style={{ color: "var(--text-3)", fontSize: 10 }}>
              pick two artifacts ({picked.length}/2)
            </span>
          )}
        </div>
      )}

      {chain.map((e, i) => (
        <div key={`${e.kind}-${i}`} style={row}>
          <span style={dot}>{i === 0 ? "●" : "↳"}</span>
          {comparing && e.path ? (
            <label style={{ display: "flex", alignItems: "center", gap: 8, cursor: "pointer" }}>
              <input
                type="checkbox"
                aria-label={e.label}
                checked={picked.includes(e.path)}
                onChange={() => toggle(e.path!)}
              />
              <span style={{ color: "var(--text-2)", fontSize: 12 }}>{e.label}</span>
            </label>
          ) : e.path ? (
            <button
              onClick={() => onOpenArtifact(e.path!)}
              style={{
                background: "none", border: "none", padding: 0, cursor: "pointer",
                color: "var(--accent)", fontSize: 12, fontFamily: "inherit", textAlign: "left",
              }}
            >
              {e.label}
            </button>
          ) : (
            <span style={{ color: "var(--text-2)", fontSize: 12 }}>{e.label}</span>
          )}
          {e.kind !== "topic" && (
            <span style={{ marginLeft: "auto", color: "var(--text-3)", fontSize: 10 }}>{e.kind}</span>
          )}
        </div>
      ))}
      {!hasArtifacts && (
        <div style={{ color: "var(--text-3)", fontSize: 11, marginTop: 10 }}>
          no upstream artifacts yet. this task has not produced one.
        </div>
      )}
    </div>
  );
}
```

- [ ] **Step 4: Run to verify pass**

Run: `/opt/homebrew/bin/bun vitest run src/components/LineageTab.test.tsx`
Expected: PASS (all original 3 tests + 2 new tests).

- [ ] **Step 5: Commit**

```bash
git add src/components/LineageTab.tsx src/components/LineageTab.test.tsx
git commit -m "feat(lineage): compare-mode select-two affordance in LineageTab (B2)"
```

---

## Task 5: Thread compare through CardDrawer

**Files:**
- Modify: `src/components/CardDrawer.tsx`
- Modify: `src/components/CardDrawer.test.tsx`

- [ ] **Step 1: Write the failing test (append to CardDrawer.test.tsx)**

```tsx
  it("renders the compare view in the lineage tab once two versions are picked", async () => {
    const onCompare = vi.fn();
    render(
      <CardDrawer
        task={task({ parent_artifact: "artifacts/T-40-v1.md", review_artifact: "artifacts/T-40-rev.md" })}
        artifactMarkdown="# Plan"
        compareMarkdown={{ left: "# Plan\n\nv1 body", right: "# Plan\n\nrev body" }}
        onCompare={onCompare}
        onApprove={() => {}}
        onRevise={() => {}}
        onReject={() => {}}
      />,
    );
    fireEvent.click(screen.getByRole("tab", { name: /lineage/i }));
    fireEvent.click(screen.getByRole("button", { name: /^compare/i }));
    fireEvent.click(screen.getByRole("checkbox", { name: /T-40-v1\.md/ }));
    fireEvent.click(screen.getByRole("checkbox", { name: /T-40-rev\.md/ }));
    expect(onCompare).toHaveBeenCalledWith("artifacts/T-40-v1.md", "artifacts/T-40-rev.md");
    // CompareView shows once compareMarkdown is present.
    expect(await screen.findByTestId("compare-view")).toBeInTheDocument();
    expect(screen.getByText(/v1 body/)).toBeInTheDocument();
    expect(screen.getByText(/rev body/)).toBeInTheDocument();
  });
```

- [ ] **Step 2: Run to verify failure**

Run: `/opt/homebrew/bin/bun vitest run src/components/CardDrawer.test.tsx`
Expected: FAIL — `compareMarkdown`/`onCompare` props missing; no compare view.

- [ ] **Step 3: Implement in CardDrawer**

In `src/components/CardDrawer.tsx`:

(a) Add the import near the other component imports:

```tsx
import { CompareView, type ComparePane } from "./CompareView";
```

(b) Extend `CardDrawerProps` (after `onOpenArtifact?`):

```tsx
  /// Open an upstream artifact from the lineage tab (D5). When omitted, clicking
  /// a lineage entry just switches to the artifact tab showing the current body.
  onOpenArtifact?: (path: string) => void;
  /// Two artifact bodies to compare side-by-side (B2). Set by the host once the
  /// operator has picked two versions; cleared (null) when not comparing.
  compareMarkdown?: { left: string; right: string } | null;
  /// Labels for the two compare panes (basenames of the chosen paths). Optional.
  compareLabels?: { left: string; right: string };
  /// Raise the two chosen artifact paths so the host can load + supply bodies.
  onCompare?: (pathA: string, pathB: string) => void;
  /// Leave compare mode (host clears compareMarkdown).
  onExitCompare?: () => void;
```

(c) Destructure them in the function signature:

```tsx
export function CardDrawer({
  task,
  artifactMarkdown,
  logText,
  reviseTarget = "the writer",
  onOpenArtifact,
  compareMarkdown,
  compareLabels,
  onCompare,
  onExitCompare,
  onApprove,
  onRevise,
  onReject,
}: CardDrawerProps) {
```

(d) Replace the `{tab === "lineage" && ( ... )}` block with:

```tsx
        {tab === "lineage" && (
          compareMarkdown ? (
            <CompareView
              left={{ label: compareLabels?.left ?? "version A", markdown: compareMarkdown.left } as ComparePane}
              right={{ label: compareLabels?.right ?? "version B", markdown: compareMarkdown.right } as ComparePane}
            />
          ) : (
            <LineageTab
              task={task}
              onOpenArtifact={(path) => {
                onOpenArtifact?.(path);
                setTab("artifact");
              }}
              onCompare={onCompare}
            />
          )
        )}
```

Note: when `compareMarkdown` is present we show `CompareView`; `onExitCompare` is wired from the host via the LineageTab "exit compare" button in the non-compare branch (the host clears `compareMarkdown` to return). Since `CompareView` is read-only, expose exit via the lineage tab re-entry: keep it simple — the host clears `compareMarkdown` on tab change / drawer close. `onExitCompare` is reserved for the App wiring in Task 6 (closing the drawer clears it). It is intentionally accepted but not yet bound to a button here to avoid a redundant control; the impeccable pass adds an in-pane "exit" affordance.

- [ ] **Step 4: Run to verify pass + keep existing CardDrawer tests green**

Run: `/opt/homebrew/bin/bun vitest run src/components/CardDrawer.test.tsx`
Expected: PASS (all existing CardDrawer tests + the new one).

- [ ] **Step 5: Commit**

```bash
git add src/components/CardDrawer.tsx src/components/CardDrawer.test.tsx
git commit -m "feat(drawer): render CompareView in lineage tab when two versions picked (B2)"
```

---

## Task 6: Wire compare data flow in App.tsx

**Files:**
- Modify: `src/App.tsx`

- [ ] **Step 1: Add compare state + loader effect**

In `src/App.tsx`, after the existing `lineagePath` state (line ~36) add:

```tsx
  // B2: two chosen artifact paths to compare side-by-side, and their loaded
  // bodies (read via the same read_artifact OHS command as the single pane).
  const [comparePaths, setComparePaths] = useState<{ a: string; b: string } | null>(null);
  const [compareMarkdown, setCompareMarkdown] = useState<{ left: string; right: string } | null>(null);
```

Extend the `openTaskId` reset effect so changing tasks also clears compare:

```tsx
  useEffect(() => {
    setLineagePath(null);
    setComparePaths(null);
    setCompareMarkdown(null);
  }, [openTaskId]);
```

After the existing single-artifact load effect (ends ~line 82), add the compare loader:

```tsx
  // B2: load both chosen artifact bodies when comparePaths is set. Reuses the
  // Workspace read_artifact OHS command — no new backend edge.
  useEffect(() => {
    let cancelled = false;
    if (!comparePaths || !activeProject) {
      setCompareMarkdown(null);
      return;
    }
    (async () => {
      try {
        const [left, right] = await Promise.all([
          readArtifact(activeProject.id, comparePaths.a),
          readArtifact(activeProject.id, comparePaths.b),
        ]);
        if (!cancelled) setCompareMarkdown({ left, right });
      } catch {
        if (!cancelled) setCompareMarkdown(null);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [comparePaths, activeProject]);
```

- [ ] **Step 2: Add a basename helper + pass props to CardDrawer**

In the `CardDrawer` JSX (line ~190), add the compare props:

```tsx
          <CardDrawer
            task={openTask}
            artifactMarkdown={artifactMarkdown}
            logText={liveLog.logFor(openTask.id)}
            reviseTarget={reviseTargetFor(openTask)}
            onOpenArtifact={setLineagePath}
            compareMarkdown={compareMarkdown}
            compareLabels={
              comparePaths
                ? { left: comparePaths.a.split("/").pop() ?? comparePaths.a, right: comparePaths.b.split("/").pop() ?? comparePaths.b }
                : undefined
            }
            onCompare={(a, b) => setComparePaths({ a, b })}
            onExitCompare={() => {
              setComparePaths(null);
              setCompareMarkdown(null);
            }}
            onApprove={handleApprove}
            onRevise={handleRevise}
            onReject={handleReject}
          />
```

- [ ] **Step 3: Typecheck + full frontend suite**

Run: `/opt/homebrew/bin/bun run build`
Expected: build succeeds (tsc clean).

Run: `/opt/homebrew/bin/bun vitest run`
Expected: all suites PASS (existing + new).

- [ ] **Step 4: Commit**

```bash
git add src/App.tsx
git commit -m "feat(app): wire B2 side-by-side compare data flow via read_artifact"
```

---

## Task 7: Full verification

- [ ] **Step 1: Frontend**

```bash
/opt/homebrew/bin/bun vitest run
/opt/homebrew/bin/bun run build
```
Expected: all green.

- [ ] **Step 2: Backend untouched — confirm still green**

```bash
cargo test --workspace
cargo check --workspace
cargo clippy --workspace
```
Expected: unchanged pass / clean clippy (no Rust changed in B2, but verify).

- [ ] **Step 3: No further commit unless verification surfaced a fix.**

---

## Self-Review

- **Spec coverage:** side-by-side two-version compare (Tasks 3-6 ✓); reuses `read_artifact` (Task 6 ✓); reuses lineage chain (Task 4 ✓); reuses ArtifactView/markdown renderer (Task 2 extraction ✓); tasteful line diff (Task 1 + badge in Task 3 ✓); no backend (✓). Frontend-mostly (✓).
- **Placeholder scan:** none — every code step is concrete.
- **Type consistency:** `lineDiff`/`DiffLine`/`LineDiff` (Task 1) used in `CompareView` (Task 3); `ComparePane`/`CompareViewProps` (Task 3) consumed in CardDrawer (Task 5); `compareMarkdown: {left,right}|null`, `compareLabels`, `onCompare`, `onExitCompare` consistent across CardDrawer (Task 5) and App (Task 6); `comparePaths: {a,b}` internal to App only.
