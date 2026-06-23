# B1 — Comment Re-anchoring Across Artifact Versions — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Upgrade Review's v1 "comments pinned to their authored version" behaviour to **structured re-anchoring**: when a writer addresses a comment in version N+1 by writing a `<!-- addressed: <comment-id> -->` marker into the new artifact, the app parses those markers and re-attaches the structured comment to the new version (marked addressed, anchored where the marker sits) instead of stranding it on the old version. Unaddressed prior comments still carry over.

**Architecture:** Pure, **read-derived** projection inside the **Review** bounded context. No schema/persistence change — the `comments` table already stores the structured comment (`id`, `task_id`, `artifact_path`, `anchor_text`, `anchor_offset`, `note`, `kind`). A pure Rust parser extracts `addressed: <comment-id>` markers + their byte offset from a version's markdown; a pure re-anchoring projector overlays those markers onto the stored comments to derive, per comment, a **status** (`open` | `addressed`) and an **effective anchor offset** (the marker position when addressed, else the stored offset). A new `reanchor_comments(task_id, version_markdown)` Review OHS command returns the projection; the frontend `useComments` hook passes the currently-viewed artifact body, and `CommentRail` renders addressed comments distinctly from carried-over ones. The marker convention is documented Review ubiquitous language. Conformist seam to Runtime is untouched (no verdict/state change). Distinct from B2's display-only line-diff.

**Tech Stack:** Rust (`src-tauri/review`, sqlx, serde, regex-free hand parser), Tauri commands, TypeScript/React frontend (`src/ipc/review.ts`, `src/hooks/useComments.ts`, `src/components/CommentRail.tsx`, `src/components/CardDrawer.tsx`), vitest.

---

## Decisions

- **DD1 — Read-derived vs persisted.** **Read-derived (recommended).** The re-anchoring is a pure projection computed from (stored comments) × (current version markdown). No new column, no migration, no `SCHEMA_VERSION` bump. The `comments` table already holds the structured comment. Rationale: derivation is clean and total; persisting `status` would create a write path that must stay consistent with artifact text and risks drift. AUTO-DECIDED.
- **DD2 — Marker convention.** `<!-- addressed: <comment-id> -->` — an HTML comment so it is invisible in rendered markdown and survives our CSP-safe parser as plain text. The `<comment-id>` is the Review `Comment.id` (UUID string). Whitespace around the colon and id is tolerated (`addressed:\s*<id>\s*`). Documented in DOMAIN.md Review language. AUTO-DECIDED.
- **DD3 — Where the parser/projector live.** In the Review crate (`src-tauri/review/src/reanchor.rs`), pure functions, unit-tested. Review owns Comment/Artifact/Thread, so the marker convention and re-anchoring are Review's ubiquitous language. No boundary bypass. AUTO-DECIDED.
- **DD4 — Status vocabulary.** A derived `CommentStatus` = `open` | `addressed`. We use `addressed` (Review's Thread language: "a comment + its resolutions across versions") rather than Runtime's `resolved`/verdict vocabulary, keeping the Conformist seam clean. Direction comments are never re-anchored (no inline anchor) and are always `open`. AUTO-DECIDED.
- **DD5 — Effective anchor.** When a comment is addressed in the viewed version, its **effective offset** is the marker's byte offset in that version's markdown (so the rail can order/locate it against the new text); `anchor_text` is preserved from the stored comment (the original quote). When not addressed, effective offset = stored `anchor_offset`. The stored row is never mutated. AUTO-DECIDED.
- **DD6 — Frontend projection seam.** `useComments(taskId, artifactPath, versionMarkdown?)` gains an optional current-version markdown; when present it calls `reanchor_comments` and exposes `ReanchoredComment[]` (Comment + `status` + `effective_offset`); when absent (or markdown empty) it falls back to the plain `list_comments` carry-over behaviour (back-compat for callers that don't pass markdown). AUTO-DECIDED.
- **DD7 — No new event / no Runtime change.** Re-anchoring is read-only Review-internal; it emits nothing to Runtime and does not touch Task state. AUTO-DECIDED.

---

## File Structure

- **Create** `src-tauri/review/src/reanchor.rs` — pure parser (`parse_addressed_markers`) + projector (`reanchor_comments`) + `CommentStatus` + `ReanchoredComment`. Unit-tested.
- **Modify** `src-tauri/review/src/lib.rs` — add `pub mod reanchor;`.
- **Modify** `src-tauri/review/src/api.rs` — add `reanchor_comments` Tauri command + OHS `ToolSpec`.
- **Modify** `src-tauri/review/src/contract_tests.rs` — lock the `ReanchoredComment` key set + `CommentStatus` strings against the TS interface.
- **Modify** `src/ipc/review.ts` — add `CommentStatus`, `ReanchoredComment`, `reanchorComments()`.
- **Modify** `src/hooks/useComments.ts` — optional `versionMarkdown`, derive `reanchored`.
- **Modify** `src/components/CommentRail.tsx` — render addressed vs carried-over distinctly; accept `ReanchoredComment[]`.
- **Modify** `src/components/CardDrawer.tsx` — pass the viewed artifact markdown into `useComments`; feed reanchored list to the rail.
- **Modify** `DOMAIN.md` — register the marker convention + re-anchoring + `addressed`/`open` status under Review.
- **Create/append** registration in `src-tauri/app/src/lib.rs` command list (if commands are centrally registered) — verify and add `reanchor_comments`.

No migration files. No `SCHEMA_VERSION` change.

---

## Task 1: Pure addressed-marker parser (Review)

**Files:**
- Create: `src-tauri/review/src/reanchor.rs`
- Modify: `src-tauri/review/src/lib.rs`

- [ ] **Step 1: Write the failing test**

In a new `src-tauri/review/src/reanchor.rs`, start with the parser and its tests:

```rust
//! Pure, read-derived comment re-anchoring for the Review context (B1).
//!
//! When a writer addresses a comment in a later artifact version they embed a
//! marker `<!-- addressed: <comment-id> -->` into the new markdown. These pure
//! functions parse those markers and project the stored structured comments
//! onto the viewed version: an addressed comment is re-anchored to the marker's
//! position (status `addressed`); an unaddressed prior comment carries over
//! (status `open`). Nothing is persisted — the projection is derived on read
//! from (stored comments) x (version markdown). The marker convention is part
//! of Review's ubiquitous language (see DOMAIN.md).

use crate::comment::{Comment, CommentKind};
use serde::{Deserialize, Serialize};

/// One parsed `<!-- addressed: <comment-id> -->` marker and its byte offset
/// (start of the marker) within the version markdown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddressedMarker {
    pub comment_id: String,
    pub offset: usize,
}

/// Parse all `<!-- addressed: <comment-id> -->` markers from a version's
/// markdown, in source order. Tolerates arbitrary internal whitespace:
/// `<!--addressed:c1-->`, `<!-- addressed:  c1  -->` both parse to `c1`.
/// Ignores any other HTML comment. Returns each marker's start byte offset.
pub fn parse_addressed_markers(markdown: &str) -> Vec<AddressedMarker> {
    let mut out = Vec::new();
    let bytes = markdown.as_bytes();
    let mut search_from = 0usize;
    while let Some(rel) = markdown[search_from..].find("<!--") {
        let start = search_from + rel;
        let Some(end_rel) = markdown[start..].find("-->") else {
            break;
        };
        let end = start + end_rel; // index of "-->"
        let inner = markdown[start + 4..end].trim();
        if let Some(rest) = inner.strip_prefix("addressed:") {
            let id = rest.trim();
            if !id.is_empty() {
                out.push(AddressedMarker {
                    comment_id: id.to_string(),
                    offset: start,
                });
            }
        }
        search_from = end + 3; // past "-->"
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_marker_with_offset() {
        let md = "intro\n<!-- addressed: c1 -->\nrest";
        let m = parse_addressed_markers(md);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].comment_id, "c1");
        assert_eq!(m[0].offset, md.find("<!--").unwrap());
    }

    #[test]
    fn tolerates_whitespace_variants() {
        let md = "<!--addressed:c1--> x <!-- addressed:   c2   -->";
        let m = parse_addressed_markers(md);
        assert_eq!(m.iter().map(|x| x.comment_id.as_str()).collect::<Vec<_>>(), vec!["c1", "c2"]);
    }

    #[test]
    fn ignores_non_addressed_comments_and_empty_ids() {
        let md = "<!-- TODO: nope --> <!-- addressed: --> <!-- addressed: real -->";
        let m = parse_addressed_markers(md);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].comment_id, "real");
    }

    #[test]
    fn handles_no_markers_and_unterminated_comment() {
        assert!(parse_addressed_markers("plain text").is_empty());
        assert!(parse_addressed_markers("<!-- addressed: c1 (never closed").is_empty());
    }
}
```

Add to `src-tauri/review/src/lib.rs` after the existing `pub mod store;`:

```rust
pub mod reanchor;
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test -p agent-bus-review reanchor::tests 2>&1 | tail -20`
Expected: compiles and PASSES if pasted whole; if you staged the test before the impl it FAILS with "cannot find function `parse_addressed_markers`". (Paste impl + tests together as shown; then this step confirms PASS.)

- [ ] **Step 3: (impl already included above)**

No further code — the parser is complete in Step 1.

- [ ] **Step 4: Run test to verify it passes**

Run: `cd src-tauri && cargo test -p agent-bus-review reanchor::tests 2>&1 | tail -20`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/review/src/reanchor.rs src-tauri/review/src/lib.rs
git commit -m "feat(review): pure addressed-marker parser (B1)"
```

---

## Task 2: Pure re-anchoring projector (Review)

**Files:**
- Modify: `src-tauri/review/src/reanchor.rs`

- [ ] **Step 1: Write the failing test**

Append to `src-tauri/review/src/reanchor.rs` (above the `#[cfg(test)]` module, add the types + function; then add tests inside the test module):

```rust
/// Derived status of a comment relative to the viewed version. `Open` = still
/// carried over (not yet addressed in this version); `Addressed` = the version
/// contains an `addressed: <id>` marker for it (re-anchored to the marker).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CommentStatus {
    Open,
    Addressed,
}

/// A stored comment projected onto the viewed version: the structured comment
/// plus its derived `status` and `effective_offset` (the marker position when
/// addressed, else the stored anchor offset). The stored row is never mutated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReanchoredComment {
    #[serde(flatten)]
    pub comment: Comment,
    pub status: CommentStatus,
    /// Effective anchor offset in the viewed version (i64 to match the wire
    /// type of `anchor_offset`; `None` for direction comments).
    pub effective_offset: Option<i64>,
}

/// Project the stored comments onto `version_markdown`: a comment whose id is
/// marked `addressed` in the version is re-anchored to the marker's offset and
/// marked `Addressed`; every other inline/direction comment carries over as
/// `Open` at its stored offset. Pure — derives from inputs, persists nothing.
/// NOTE: the no-marker default here (no marker ⇒ `Open` at stored offset) is
/// mirrored by the `useComments` frontend fallback for the no-version-body
/// case; the two must change in lockstep (vet F3). This Rust projector is the
/// single authoritative path whenever a version body is present (the only path
/// that ever yields `Addressed`).
/// Order: source-order by effective offset (addressed first follow the new
/// text), then by `created_at` as a stable tiebreak, mirroring the store's
/// `created_at, anchor_offset` list order for carried-over comments.
pub fn reanchor_comments(comments: &[Comment], version_markdown: &str) -> Vec<ReanchoredComment> {
    let markers = parse_addressed_markers(version_markdown);
    let mut out: Vec<ReanchoredComment> = comments
        .iter()
        .map(|c| {
            let marker = markers.iter().find(|m| m.comment_id == c.id);
            match (marker, c.kind) {
                // Direction comments have no inline anchor; never re-anchored.
                (Some(m), CommentKind::Inline) => ReanchoredComment {
                    comment: c.clone(),
                    status: CommentStatus::Addressed,
                    effective_offset: Some(m.offset as i64),
                },
                (Some(_), CommentKind::Direction) | (None, _) => ReanchoredComment {
                    comment: c.clone(),
                    status: CommentStatus::Open,
                    effective_offset: c.anchor_offset,
                },
            }
        })
        .collect();
    out.sort_by(|a, b| {
        a.effective_offset
            .cmp(&b.effective_offset)
            .then(a.comment.created_at.cmp(&b.comment.created_at))
    });
    out
}
```

Add tests inside the existing `#[cfg(test)] mod tests`:

```rust
    fn cmt(id: &str, offset: Option<i64>, kind: CommentKind, created: i64) -> Comment {
        Comment {
            id: id.into(),
            task_id: "T-1".into(),
            artifact_path: "artifacts/specs/T-1-v1.md".into(),
            anchor_text: Some("quote".into()),
            anchor_offset: offset,
            note: "note".into(),
            kind,
            created_at: created,
        }
    }

    #[test]
    fn addressed_comment_is_reanchored_to_marker_offset() {
        let comments = vec![cmt("c1", Some(5), CommentKind::Inline, 100)];
        let md = "header\n\n<!-- addressed: c1 --> resolved here";
        let out = reanchor_comments(&comments, md);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].status, CommentStatus::Addressed);
        assert_eq!(out[0].effective_offset, Some(md.find("<!--").unwrap() as i64));
    }

    #[test]
    fn unaddressed_comment_carries_over_at_stored_offset() {
        let comments = vec![cmt("c1", Some(42), CommentKind::Inline, 100)];
        let out = reanchor_comments(&comments, "no markers here");
        assert_eq!(out[0].status, CommentStatus::Open);
        assert_eq!(out[0].effective_offset, Some(42));
    }

    #[test]
    fn direction_comment_never_reanchored_even_if_marked() {
        let comments = vec![cmt("c1", None, CommentKind::Direction, 100)];
        let md = "<!-- addressed: c1 -->";
        let out = reanchor_comments(&comments, md);
        assert_eq!(out[0].status, CommentStatus::Open);
        assert_eq!(out[0].effective_offset, None);
    }

    #[test]
    fn mixed_set_orders_by_effective_offset() {
        let comments = vec![
            cmt("c1", Some(50), CommentKind::Inline, 100), // unaddressed, stays at 50
            cmt("c2", Some(99), CommentKind::Inline, 101), // addressed -> marker offset 0
        ];
        let md = "<!-- addressed: c2 -->\nlater text past offset fifty.....................";
        let out = reanchor_comments(&comments, md);
        // c2 re-anchored to offset 0 sorts before c1 at 50.
        assert_eq!(out[0].comment.id, "c2");
        assert_eq!(out[0].status, CommentStatus::Addressed);
        assert_eq!(out[1].comment.id, "c1");
        assert_eq!(out[1].status, CommentStatus::Open);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test -p agent-bus-review reanchor::tests 2>&1 | tail -20`
Expected: PASS if impl pasted with tests. (If tests were staged first: FAIL "cannot find function `reanchor_comments`".)

- [ ] **Step 3: (impl included in Step 1)**

- [ ] **Step 4: Run to verify pass**

Run: `cd src-tauri && cargo test -p agent-bus-review reanchor::tests 2>&1 | tail -20`
Expected: PASS (8 tests total in module).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/review/src/reanchor.rs
git commit -m "feat(review): pure comment re-anchoring projector (B1)"
```

---

## Task 3: `reanchor_comments` Tauri command + OHS tool

**Files:**
- Modify: `src-tauri/review/src/api.rs`
- Modify: `src-tauri/app/src/lib.rs` (command registration — verify)

- [ ] **Step 1: Write the failing test**

In `src-tauri/review/src/api.rs`, add the command after `list_comments`:

```rust
use crate::reanchor::{reanchor_comments as reanchor_project, ReanchoredComment};

/// Re-anchor a task's comments onto a viewed artifact version (B1). Pure,
/// read-derived: lists the stored comments, parses `addressed: <id>` markers
/// from `version_markdown`, and returns each comment with its derived status
/// (`open`/`addressed`) and effective offset. Persists nothing; the Conformist
/// seam to Runtime is untouched (no verdict, no Task-state change).
#[tauri::command(rename_all = "snake_case")]
pub async fn reanchor_comments(
    state: tauri::State<'_, ReviewState>,
    task_id: String,
    version_markdown: String,
) -> Result<Vec<ReanchoredComment>, String> {
    let comments = state
        .comments
        .list_for_task(&task_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(reanchor_project(&comments, &version_markdown))
}
```

Add an OHS `ToolSpec` to the `tools()` vec (after `list_comments`):

```rust
        ToolSpec {
            name: "reanchor_comments".into(),
            description: "Re-anchor a task's comments onto a viewed artifact version using \
                          <!-- addressed: <comment-id> --> markers; returns each comment with \
                          a derived status (open/addressed) and effective offset."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "task_id": {"type": "string"},
                    "version_markdown": {"type": "string"}
                },
                "required": ["task_id", "version_markdown"]
            }),
            supplier_context: "review".into(),
        },
```

Add tests to the `#[cfg(test)] mod tests` in `api.rs`:

```rust
    #[test]
    fn tools_publishes_reanchor_under_review_context() {
        let specs = tools();
        let names: Vec<&str> = specs.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"reanchor_comments"));
        assert!(specs.iter().all(|s| s.supplier_context == "review"));
    }
```

- [ ] **Step 2: Run to verify it fails / compiles**

Run: `cd src-tauri && cargo test -p agent-bus-review api::tests 2>&1 | tail -20`
Expected: PASS for the new tools test.

- [ ] **Step 3: Register the command at the composition root**

Inspect `src-tauri/app/src/lib.rs` for the `tauri::generate_handler!` macro list. Add `agent_bus_review::api::reanchor_comments` alongside `add_comment`, `list_comments`, `delete_comment`, `record_verdict`. (Use the exact module path the existing review commands use.)

Run: `cd src-tauri && cargo check --workspace 2>&1 | tail -15`
Expected: clean.

- [ ] **Step 4: Run review tests**

Run: `cd src-tauri && cargo test -p agent-bus-review 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/review/src/api.rs src-tauri/app/src/lib.rs
git commit -m "feat(review): reanchor_comments command + OHS tool (B1)"
```

---

## Task 4: Contract test — lock `ReanchoredComment` wire shape

**Files:**
- Modify: `src-tauri/review/src/contract_tests.rs`

- [ ] **Step 1: Write the failing test**

Append to `src-tauri/review/src/contract_tests.rs`:

```rust
use crate::reanchor::{reanchor_comments, CommentStatus};

/// Locks the `ReanchoredComment` wire shape: a flattened `Comment` plus
/// `status` (open/addressed) and `effective_offset` (number | null). Matches
/// `src/ipc/review.ts` `interface ReanchoredComment`.
#[test]
fn reanchored_comment_key_set_matches_ts() {
    let c = Comment {
        id: "c1".into(),
        task_id: "T-1".into(),
        artifact_path: "artifacts/specs/T-1-v1.md".into(),
        anchor_text: Some("quote".into()),
        anchor_offset: Some(5),
        note: "note".into(),
        kind: CommentKind::Inline,
        created_at: 1000,
    };
    let md = "<!-- addressed: c1 -->";
    let out = reanchor_comments(std::slice::from_ref(&c), md);
    let v = serde_json::to_value(&out[0]).unwrap();
    assert_eq!(
        keys(&v),
        set(&[
            "id",
            "task_id",
            "artifact_path",
            "anchor_text",
            "anchor_offset",
            "note",
            "kind",
            "created_at",
            "status",
            "effective_offset",
        ]),
    );
    assert_eq!(v["status"], Value::String("addressed".into()));
    assert!(v["effective_offset"].is_number());
}

#[test]
fn comment_status_matches_ts_string_union() {
    assert_eq!(serde_json::to_value(CommentStatus::Open).unwrap(), Value::String("open".into()));
    assert_eq!(
        serde_json::to_value(CommentStatus::Addressed).unwrap(),
        Value::String("addressed".into())
    );
}
```

- [ ] **Step 2: Run to verify**

Run: `cd src-tauri && cargo test -p agent-bus-review contract_tests 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/review/src/contract_tests.rs
git commit -m "test(review): lock ReanchoredComment wire contract (B1)"
```

---

## Task 5: Frontend IPC — `reanchorComments`

**Files:**
- Modify: `src/ipc/review.ts`

- [ ] **Step 1: Add types + function**

Append to `src/ipc/review.ts`:

```ts
export type CommentStatus = "open" | "addressed";

/// A stored comment projected onto the viewed artifact version (B1): the
/// `Comment` fields are flattened in, plus a derived `status` and the
/// `effective_offset` (the marker position when addressed, else the stored
/// anchor offset). Read-derived — nothing new is persisted.
export interface ReanchoredComment extends Comment {
  status: CommentStatus;
  effective_offset: number | null;
}

/// Re-anchor a task's comments onto the markdown of the currently viewed
/// artifact version using `<!-- addressed: <comment-id> -->` markers.
export async function reanchorComments(
  taskId: string,
  versionMarkdown: string,
): Promise<ReanchoredComment[]> {
  return await invoke<ReanchoredComment[]>("reanchor_comments", {
    task_id: taskId,
    version_markdown: versionMarkdown,
  });
}
```

- [ ] **Step 2: Typecheck**

Run: `cd /Users/tim/projects/agent-bus-app && bun tsc --noEmit 2>&1 | tail -15` (or `bun run build` later).
Expected: no new errors.

- [ ] **Step 3: Commit**

```bash
git add src/ipc/review.ts
git commit -m "feat(ipc): reanchorComments + ReanchoredComment type (B1)"
```

---

## Task 6: `useComments` exposes reanchored projection

**Files:**
- Modify: `src/hooks/useComments.ts`

- [ ] **Step 1: Write the failing test**

Create `src/hooks/useComments.test.ts` (mock the IPC layer):

```ts
import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";

vi.mock("../ipc/review", () => ({
  listComments: vi.fn(),
  addComment: vi.fn(),
  deleteComment: vi.fn(),
  reanchorComments: vi.fn(),
}));

import { listComments, reanchorComments } from "../ipc/review";
import { useComments } from "./useComments";

const baseComment = {
  id: "c1",
  task_id: "T-1",
  artifact_path: "artifacts/T-1-v1.md",
  anchor_text: "q",
  anchor_offset: 5,
  note: "n",
  kind: "inline" as const,
  created_at: 1,
};

describe("useComments", () => {
  beforeEach(() => vi.clearAllMocks());

  it("returns plain comments (status open) when no version markdown", async () => {
    (listComments as any).mockResolvedValue([baseComment]);
    const { result } = renderHook(() => useComments("T-1", "artifacts/T-1-v1.md"));
    await waitFor(() => expect(result.current.reanchored.length).toBe(1));
    expect(result.current.reanchored[0].status).toBe("open");
    expect(reanchorComments).not.toHaveBeenCalled();
  });

  it("uses reanchorComments when version markdown is provided", async () => {
    (listComments as any).mockResolvedValue([baseComment]);
    (reanchorComments as any).mockResolvedValue([
      { ...baseComment, status: "addressed", effective_offset: 0 },
    ]);
    const { result } = renderHook(() =>
      useComments("T-1", "artifacts/T-1-v2.md", "<!-- addressed: c1 -->"),
    );
    await waitFor(() => expect(result.current.reanchored[0]?.status).toBe("addressed"));
    expect(reanchorComments).toHaveBeenCalledWith("T-1", "<!-- addressed: c1 -->");
  });
});
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd /Users/tim/projects/agent-bus-app && bun vitest run src/hooks/useComments.test.ts 2>&1 | tail -25`
Expected: FAIL — `result.current.reanchored` is undefined (hook doesn't expose it yet).

- [ ] **Step 3: Implement**

Rewrite `src/hooks/useComments.ts` to additionally derive `reanchored`:

```ts
import { useCallback, useEffect, useState } from "react";
import {
  addComment,
  deleteComment,
  listComments,
  reanchorComments,
  type Comment,
  type CommentKind,
  type ReanchoredComment,
} from "../ipc/review";

export interface AddCommentInput {
  note: string;
  anchorText?: string;
  anchorOffset?: number;
  kind: CommentKind;
}

export interface UseComments {
  comments: Comment[];
  /// Comments projected onto the viewed version (B1). When `versionMarkdown`
  /// is provided, addressed comments are re-anchored; otherwise every comment
  /// is `status: "open"` at its stored offset (carry-over, back-compat).
  reanchored: ReanchoredComment[];
  add: (input: AddCommentInput) => Promise<void>;
  remove: (commentId: string) => Promise<void>;
}

export function useComments(
  taskId: string,
  artifactPath: string,
  versionMarkdown?: string,
): UseComments {
  const [comments, setComments] = useState<Comment[]>([]);
  const [reanchored, setReanchored] = useState<ReanchoredComment[]>([]);

  const reload = useCallback(() => {
    listComments(taskId).then((list) => {
      setComments(list);
      if (versionMarkdown && versionMarkdown.trim()) {
        reanchorComments(taskId, versionMarkdown).then(setReanchored);
      } else {
        // Carry-over fallback when no version body is supplied. This mirrors
        // the Rust projector's no-marker default (no marker => status "open"
        // at the stored offset; see reanchor::reanchor_comments) and must
        // change in lockstep with it (vet F3). The backend projector remains
        // authoritative whenever a version body is present.
        setReanchored(
          list.map((c) => ({ ...c, status: "open", effective_offset: c.anchor_offset })),
        );
      }
    });
  }, [taskId, versionMarkdown]);

  useEffect(() => {
    reload();
  }, [reload]);

  const add = useCallback(
    async (input: AddCommentInput) => {
      await addComment({
        taskId,
        artifactPath,
        note: input.note,
        anchorText: input.anchorText,
        anchorOffset: input.anchorOffset,
        kind: input.kind,
      });
      reload();
    },
    [taskId, artifactPath, reload],
  );

  const remove = useCallback(
    async (commentId: string) => {
      await deleteComment(commentId);
      reload();
    },
    [reload],
  );

  return { comments, reanchored, add, remove };
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cd /Users/tim/projects/agent-bus-app && bun vitest run src/hooks/useComments.test.ts 2>&1 | tail -25`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add src/hooks/useComments.ts src/hooks/useComments.test.ts
git commit -m "feat(hooks): useComments derives reanchored projection (B1)"
```

---

## Task 7: `CommentRail` renders addressed vs carried-over

**Files:**
- Modify: `src/components/CommentRail.tsx`

- [ ] **Step 1: Write the failing test**

Create `src/components/CommentRail.test.tsx`:

```tsx
import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { CommentRail } from "./CommentRail";
import type { ReanchoredComment } from "../ipc/review";

function rc(id: string, status: "open" | "addressed"): ReanchoredComment {
  return {
    id,
    task_id: "T-1",
    artifact_path: "a.md",
    anchor_text: "quote",
    anchor_offset: 0,
    note: `note-${id}`,
    kind: "inline",
    created_at: 1,
    status,
    effective_offset: 0,
  };
}

describe("CommentRail", () => {
  it("marks addressed comments as addressed", () => {
    render(
      <CommentRail
        comments={[rc("c1", "addressed"), rc("c2", "open")]}
        onSelect={vi.fn()}
        onDelete={vi.fn()}
      />,
    );
    expect(screen.getByText("note-c1").closest("[data-status]")).toHaveAttribute(
      "data-status",
      "addressed",
    );
    expect(screen.getByTestId("rail-count")).toHaveTextContent("1 of 2 addressed");
  });
});
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd /Users/tim/projects/agent-bus-app && bun vitest run src/components/CommentRail.test.tsx 2>&1 | tail -25`
Expected: FAIL — `CommentRail` still takes `Comment[]` and has no `data-status` / "addressed" count.

- [ ] **Step 3: Implement**

Change `CommentRail` to accept `ReanchoredComment[]` and render status. Replace the props type + body. Key changes:

- Import: `import type { ReanchoredComment } from "../ipc/review";`
- Props: `comments: ReanchoredComment[];`
- Filter inline as before: `const inline = comments.filter((c) => c.kind === "inline");`
- Count header: compute `const addressed = inline.filter((c) => c.status === "addressed").length;` and render the count testid as `{addressed > 0 ? \`${addressed} of ${inline.length} addressed\` : \`${inline.length} comment${inline.length === 1 ? "" : "s"}\`}` (keep the empty-state branch unchanged).
- Each entry root `<div>` gets `data-status={c.status}` and, when `c.status === "addressed"`, a visual treatment: a small "addressed" pill and a dimmed/struck note. Concretely, inside the entry add near the "you" label:

```tsx
{c.status === "addressed" && (
  <span
    style={{
      marginLeft: 6,
      fontSize: 9.5,
      color: "var(--accent)",
      border: "1px solid var(--accent)",
      borderRadius: 6,
      padding: "0 5px",
      textTransform: "lowercase",
    }}
  >
    addressed
  </span>
)}
```

and set the entry `background` to `active ? "var(--accent-2)" : c.status === "addressed" ? "var(--bg-3, var(--bg-2))" : "transparent"` and the note style `textDecoration: c.status === "addressed" ? "line-through" : "none"`, `opacity: c.status === "addressed" ? 0.7 : 1`. Add `data-status={c.status}` to the entry div.

Full replacement of the entry `<div>` opening and rail-count span:

```tsx
        <span data-testid="rail-count" style={{ color: "var(--accent)" }}>
          {addressed > 0
            ? `${addressed} of ${inline.length} addressed`
            : `${inline.length} comment${inline.length === 1 ? "" : "s"}`}
        </span>
```

```tsx
          <div key={c.id} data-status={c.status} style={entry} onClick={() => onSelect(c.id)}>
```

(where `entry` now folds in the addressed background as above), and the note `<div style={noteStyle}>` becomes:

```tsx
            <div
              style={{
                ...noteStyle,
                textDecoration: c.status === "addressed" ? "line-through" : "none",
                opacity: c.status === "addressed" ? 0.7 : 1,
              }}
            >
              {c.note}
            </div>
```

Keep the empty-state branch and all other styling unchanged.

- [ ] **Step 4: Run to verify pass**

Run: `cd /Users/tim/projects/agent-bus-app && bun vitest run src/components/CommentRail.test.tsx 2>&1 | tail -25`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/components/CommentRail.tsx src/components/CommentRail.test.tsx
git commit -m "feat(rail): render addressed vs carried-over comments (B1)"
```

---

## Task 8: Wire `CardDrawer` to feed the viewed version into the rail

**Files:**
- Modify: `src/components/CardDrawer.tsx`

- [ ] **Step 1: Implement (wiring)**

In `CardDrawer`, pass the viewed artifact body to `useComments` and feed the rail the reanchored list:

- Change the hook call:

```tsx
  const { reanchored, add, remove } = useComments(task.id, artifactPath, artifactMarkdown);
```

- Replace `comments` usages: `inlineCount` becomes from `reanchored`:

```tsx
  const inlineCount = reanchored.filter((c) => c.kind === "inline").length;
```

- The `CommentRail` now receives `comments={reanchored}`:

```tsx
              <CommentRail
                comments={reanchored}
                activeId={activeComment}
                onSelect={setActiveComment}
                onDelete={remove}
              />
```

(`add` is unchanged.)

- [ ] **Step 2: Typecheck + targeted tests**

Run: `cd /Users/tim/projects/agent-bus-app && bun vitest run src/App.test.tsx 2>&1 | tail -25`
Expected: PASS (App still renders; comments flow unchanged where markdown drives reanchoring).

- [ ] **Step 3: Commit**

```bash
git add src/components/CardDrawer.tsx
git commit -m "feat(drawer): feed viewed version into comment re-anchoring (B1)"
```

---

## Task 9: DOMAIN.md — register marker convention + re-anchoring language

**Files:**
- Modify: `DOMAIN.md`

- [ ] **Step 1: Edit the Review section**

Under `### Review`, after the `**Thread**` line, add:

```markdown
- **Addressed marker** — `<!-- addressed: <comment-id> -->`, an HTML comment a writer embeds into a later artifact version to signal it has resolved that comment. Review's ubiquitous-language convention (B1). An addressed marker is a **Thread** resolution made concrete in the artifact text.
- **Re-anchoring** — the read-derived projection (no persistence) that computes a **Thread**'s "resolutions across versions" for a viewed version: it overlays that version's `addressed` markers onto the stored comments — a marked comment is re-anchored to the marker's position with status `addressed`; an unmarked prior comment carries over with status `open`. Distinct from B2's display-only line-diff (which never touches comment identity).
- **Comment status** — the per-version read of a **Thread**: `open` (carried over, not yet addressed in the viewed version) or `addressed` (a marker re-anchored it here). Derived per viewed version, never stored; the Conformist seam to Runtime (verdicts) is untouched.
- **Re-anchored (effective) offset** — a comment's anchor position *in the viewed version*: the `addressed` marker's offset when addressed, else the stored `anchor_offset`. Surfaced on the wire as `effective_offset`; derived, never stored.
```

- [ ] **Step 2: Commit**

```bash
git add DOMAIN.md
git commit -m "docs(domain): register addressed-marker + re-anchoring language (B1)"
```

---

## Task 10: Full verification

- [ ] **Step 1: Backend**

```bash
cd /Users/tim/projects/agent-bus-app/src-tauri
cargo test --workspace 2>&1 | tail -25
cargo check --workspace 2>&1 | tail -10
cargo clippy --workspace --all-targets 2>&1 | tail -20
```
Expected: tests PASS, check clean, clippy clean (no warnings).

- [ ] **Step 2: Frontend**

```bash
cd /Users/tim/projects/agent-bus-app
bun vitest run 2>&1 | tail -25
bun run build 2>&1 | tail -15
```
Expected: vitest all PASS, build succeeds.

- [ ] **Step 3: Commit (only if any fmt/lint fixups were needed)**

```bash
git add -A
git commit -m "chore(b1): verification fixups" || true
```

---

## Self-Review Checklist (ran against spec)

- **Spec coverage:** parser (T1), projector (T2), command+OHS (T3), wire contract (T4), IPC (T5), hook (T6), rail UI (T7), drawer wiring (T8), DOMAIN language (T9), verify (T10). Addressed markers re-anchor; unaddressed carry over — covered. ✓
- **Read-derived, no migration:** confirmed — no `migrations/` file, no `SCHEMA_VERSION` bump (DD1). ✓
- **Conformist seam intact:** `reanchor_comments` reads only; no verdict/Task-state write (DD7). ✓
- **Type consistency:** `ReanchoredComment` (flattened `Comment` + `status` + `effective_offset`), `CommentStatus` (`open`/`addressed`) consistent across Rust ↔ TS ↔ contract test. ✓
- **Distinct from B2:** documented in DOMAIN + projector doc-comment. ✓
