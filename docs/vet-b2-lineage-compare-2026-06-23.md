---
id: vet-b2-lineage-compare
date: 2026-06-23
target: plans/2026-06-23-plan-b2-lineage-compare.md
verb: vet
lens: strategic · critique · brief
verdict: SOUND WITH FIXES (both applied in-plan)
---

# Vet — B2 Lineage side-by-side compare

**Scope of the change.** A pure **frontend read-projection** in the **Review**
context: the CardDrawer Lineage tab lets the operator pick two artifact-bearing
lineage entries and renders both artifacts' markdown side-by-side, with a
tasteful per-side line-diff badge. It reads through two *already-published*
surfaces — Workspace's `read_artifact` OHS command and the `Task` lineage chain
(`parent_artifact` / `review_artifact`, derived by `buildLineage`). No backend,
no schema change, no new persisted concept.

## Room verdict (brief)

- **Architect:** The seams hold. The view consumes Workspace's published
  `read_artifact` and Runtime's `Task` pointers exactly as the existing single
  pane already does (`App.tsx` `lineagePath` → `readArtifact`). Compare is the
  *same* read path, run twice. No context reaches past another's public surface;
  no concept is placed in a context that doesn't own it. Review owns
  reading-artifacts — compare is squarely a Review read-surface.
- **Engineer:** Refactor-before-add is honored where it counts: Task 2 extracts
  the shared `renderBlock` rather than duplicating the ArtifactView switch into
  CompareView. `lineDiff` and `CompareView` are genuinely new (no existing
  equivalent) — a justified add, not a refactor dodge. Buildable; no Rust
  touched.
- **Engineer (Review) on the diff:** the one thing to guard — the line-diff must
  stay **display-only** and not drift into B1 territory (comment re-anchoring
  across versions IS a Review domain concept with markers; this diff is not).
  Flagged as F1.

## Findings

### F1 [low] off-language naming — keep the diff display-only, distinct from B1 re-anchoring

**What.** The plan adds `lineDiff` (`src/lib/lineDiff.ts`) and a per-side change
badge. DOMAIN.md → Review lists **Comment / Thread / Send back** and notes
artifacts are *versioned and immutable per version*; the **B1** backlog item
("comment re-anchoring across artifact versions") is the *domain* concept that
relates two versions. There's a risk a future reader conflates this presentational
line-diff with B1's semantic re-anchoring.

**Cited plan section.** Decisions DD2; Task 1 (`lineDiff`).

**Why it matters.** If the diff acquired domain meaning (deciding which comments
carry forward), it would be Review business logic living in a `lib/` helper — an
anaemic-model / logic-in-the-wrong-place smell. As written it is pure line
identity for *display*, which is correct.

**Amendment (applied).** Task 1's doc-comment states the classifier is "Pure …
no markdown semantics, just line identity" and Task 3's CompareView is documented
"read-only … No comment/selection affordance … a pure read projection over
read_artifact." This keeps it display-only and explicitly *not* B1. **Resolved** —
the plan's own comments already draw the line; no code change needed beyond
confirming the wording, which is present.

**Status:** resolved

### F2 [low] unowned shared type — `renderBlock` extraction stays Review-internal

**What.** Task 2 extracts `renderBlock` into `src/lib/renderBlock.tsx`, shared by
`ArtifactView` and `CompareView`. A "shared" helper added for two-plus consumers
is the cue for an *accidental shared kernel*.

**Cited plan section.** Decisions DD4; Task 2.

**Why it matters.** A shared module across *contexts* would be an unowned kernel.
Here both consumers (`ArtifactView`, `CompareView`) are the **Review** context's
own frontend surface (artifact rendering), so it's an intra-context UI helper, not
a cross-context kernel — acceptable, provided it stays Review-only and never
becomes a generic app-wide renderer other contexts import.

**Amendment (applied).** Task 2's `renderBlock.tsx` doc-comment scopes it: "Shared
CSP-safe block renderer for agent-artifact markdown. Used by the single-pane
ArtifactView and the two-column CompareView (B2)." Both are Review surfaces; the
comment names the two intended consumers, bounding the sharing. **Resolved** — the
extraction is a within-context DRY move, correctly scoped; no split needed.

**Status:** resolved

## Confirmed clean (no finding)

- **No boundary bypass.** Compare reads via `readArtifact` (Workspace OHS) +
  `buildLineage(task)` over Runtime's `Task` pointers — the identical surfaces the
  single pane uses. No new IPC, no new Rust command.
- **No new persisted concept / no schema change.** `comparePaths` /
  `compareMarkdown` are transient `App.tsx` state; nothing is written.
- **No business logic in the view.** `lineDiff` is line-identity display
  computation; `CompareView` is presentational; no verdict / version / anchoring
  decision lives here.
- **No backend added.** If one had been needed it would have to stay in the owning
  context (Workspace for reads, Review for artifact concepts) — the plan correctly
  avoids the need by reusing `read_artifact`.
- **Naming.** `CompareView` / `ComparePane` are UI-component names, not domain
  types; they don't contradict the Review ubiquitous language. The relation between
  two artifacts is the existing **version** concept, which the labels (basenames,
  e.g. `T-1-v1.md`) surface naturally.

**Verdict: SOUND WITH FIXES — F1/F2 both low, both resolved in-plan (doc-comment
scoping only). Cleared to build.**
