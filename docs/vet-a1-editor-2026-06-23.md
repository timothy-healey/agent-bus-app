---
id: vet-a1-editor-2026-06-23
target: plans/2026-06-23-plan-a1-editor.md
item: A1 — Pipeline editor write-mode
date: 2026-06-23
verb: vet
lens: strategic · vet · brief
verdict: SOUND WITH FIXES
---

# Vet — A1 Pipeline editor write-mode

Reviewed the plan against `DOMAIN.md` and the affected code (`pipeline/src/draft.rs`,
`pipeline/src/model.rs`, `pipeline/src/resolve.rs`, `pipeline/src/store.rs`,
`workspace/src/api.rs`, `app/src/lib.rs` create-from-draft path, the wizard
frontend + `PipelineView`/`App`). DDD soundness only.

## Summary

The design is sound and refactor-first where it matters most: it **reuses the
wizard step editors** rather than forking a parallel editor (the central risk for
this item), and it **keeps Pipeline Authoring off the filesystem** — the converter
receives injected prompt bodies, the root reads them via Workspace, and the save
reuses `write_project_pipeline_inner`. `DraftPipeline` stays distinct from
`Pipeline`; `from_pipeline` is the faithful inverse of `to_pipeline` with a
round-trip test contract. The create flow is untouched.

Two low-severity findings, both applied as plan + DOMAIN.md amendments.

---

### F1 [low] Off-language naming — the in-app editor is not a "Design Session"

**What:** The plan reuses the wizard step components and the `DraftPipeline`
draft-editing machinery for in-app editing (plan Tasks 4–6, Decision D6). The
ubiquitous language reserves **Design Session** for *the ephemeral AI-assisted
authoring dialogue conducted over the LLM Chat ACL* (`DOMAIN.md` → Pipeline
Authoring → "Design Session": one `dialogue_id` per wizard step, drives the
5-step wizard). The new edit-mode deliberately has **no chat** (D6: it does NOT
embed `ChatDraftPanel`) — it is a *manual* `DraftPipeline` edit. Without a named
distinction, a future reader could mistake `PipelineEditor` for a second Design
Session and wire chat/`dialogue_id` into it, eroding the boundary that keeps the
LLM Chat ACL the only chat seam.

**Cited:** plan Decision D6 + Task 4 (`PipelineEditor.tsx` "no Design Session
chat — editing is manual"); `DOMAIN.md` → Pipeline Authoring → "Design Session".

**Why it matters:** The language should put the right concept in the code. The
edit surface is the same *draft-editing* half of the wizard (the `DraftPipeline` +
step editors + best-effort validation) *minus* the Design Session (chat) half.
Naming that explicitly keeps the LLM Chat ACL seam clean.

**Amendment:** Register a DOMAIN.md ubiquitous-language entry under Pipeline
Authoring — **"Pipeline edit-mode"** — naming the in-app editor as a *manual*
`DraftPipeline` edit seeded from the active pipeline (via `from_pipeline`), reusing
the wizard's step editors + best-effort validation but **not** the Design Session
chat; it saves through `save_pipeline_edits` (hard-validate → Workspace overwrite).
And add a one-line note to the plan's `## Decisions` (D6) that this is deliberately
*not* a Design Session.

**Status:** resolved — DOMAIN.md "Pipeline edit-mode" entry added; plan D6 note added.

---

### F2 [low] Adds where a refactor fits — create and save duplicate the validate→serialize→write core

**What:** `save_pipeline_edits_inner` (plan Task 2) repeats the
`to_pipeline()` → `validate::validate` → `to_yaml` → `prompt_files` →
`pipelines/<id>.yaml` sequence verbatim from `create_project_from_draft_inner`
(`app/src/lib.rs:596–612`). The two orchestrators legitimately differ at the ends
(create inserts a Project row + tilde-expands the root; save does neither), but the
**hard-validate + serialize + prompt-files + yaml-path** core is identical. Left
duplicated, the two can drift — e.g. a future change to the prompt-path convention
or the validation gate would have to be made twice (the exact *Refactor before you
add* smell).

**Cited:** plan Task 2 (`save_pipeline_edits_inner` body) vs
`app/src/lib.rs:596–612` (`create_project_from_draft_inner`).

**Why it matters:** The validate-before-any-write invariant ("nothing is written
when invalid") is the safety contract shared by both flows; a single source keeps
it honest.

**Amendment:** Extract a small pure helper in Pipeline Authoring —
`fn prepared_write(draft: &DraftPipeline) -> Result<(String /*yaml_rel*/, String /*yaml*/, Vec<(String,String)> /*prompts*/), String>`
(name it `prepare_pipeline_write` in `pipeline::draft`) that does
`to_pipeline → validate → to_yaml → prompt_files → yaml_rel`, returning the bytes
to write or the validation error. Both `create_project_from_draft_inner` and
`save_pipeline_edits_inner` call it, so the validate-then-write gate lives in one
place. This is a Pipeline-Authoring-owned function (it serializes; Workspace still
writes), so no boundary moves.

**Status:** resolved — plan Task 2 amended to add `prepare_pipeline_write` and have
both create + save use it; create's inline block is refactored to call it.

---

## Boundaries confirmed clean (no finding)

- **Workspace owns the write; Pipeline Authoring serializes** — save reuses
  `write_project_pipeline_inner` (no new write path); `from_pipeline` takes
  injected bodies (no PA filesystem touch). The prompt-body read happens at the
  composition root (`app`) via `workspace::api::resolve_under_root`. ✓
- **`DraftPipeline` ≠ `Pipeline`** — `from_pipeline` returns a `DraftPipeline`;
  the create flow's `to_pipeline` + hard-validate is preserved unchanged. ✓
- **Round-trip faithfulness** — `from_pipeline(p, bodies).to_pipeline() == p`
  is the test contract (plan Task 1), including the P2/P3 join options
  (`cancel_on_reject` / `quorum`). ✓
- **Overwrite is safe/idempotent** — `std::fs::write` truncates; re-activation is
  idempotent; the edited pipeline keeps its id and overwrites the same paths
  (plan D4/D5). Orphan prompt files from removed teams are noted as out-of-scope
  (harmless, unreferenced by the YAML). ✓
- **No parallel editor** — the editor reuses `TeamsStep`/`PromptsStep`/`WiringStep`
  (plan D6), satisfying *Refactor before you add* for the central risk. ✓
