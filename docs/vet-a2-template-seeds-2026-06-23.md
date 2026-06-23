---
id: vet-a2-template-seeds-2026-06-23
verb: vet
target: plans/2026-06-23-plan-a2-template-seeds.md
domain: DOMAIN.md
date: 2026-06-23
lens: strategic · vet · brief
verdict: SOUND WITH FIXES
---

# Vet — A2: Templates as Wizard Seeds

Pre-build DDD soundness review of `plans/2026-06-23-plan-a2-template-seeds.md`, read against `DOMAIN.md` and the affected code (`pipeline/src/{draft.rs,design_session.rs,api.rs}`, `app/src/lib.rs`, `src/wizard/NewProjectWizard.tsx`, `src/ipc/pipeline.ts`). Scope: boundaries, language, refactor-before-add — not decomposition/testability.

## Summary

The design is sound. A bundled-template **seed** is correctly placed in Pipeline Authoring, yields a `DraftPipeline` (not a `Pipeline`), and does **not** resurrect the dropped instantiate-on-create boundary — the wizard still owns create + hard validation, and Workspace still writes only at create via `write_project_pipeline`. Three findings, all low/medium, all amendable in-plan; none require a redesign or a pre-refactor.

The room's read on the focus points:

- **Belongs to Pipeline Authoring** — ✅. The seed registry is a `DraftPipeline` factory; `DraftPipeline`/`Design Session`/`to_pipeline` all live in `pipeline`. No Workspace, Runtime, or Review concept is touched.
- **Does NOT resurrect auto-instantiate-on-create** — ✅. The dropped boundary (sub-project 3, `00ef101`) was *instantiate a validated `Pipeline` and write it on create*. This seeds the *editable draft*; the create path (`create_project_from_draft_inner`) is unchanged and still hard-validates. The two are categorically different — see F1 for the language that keeps them from re-merging.
- **`DraftPipeline` vs `Pipeline` distinction intact** — ✅. `seed_template(id) -> Option<DraftPipeline>` (DD1). The seed deliberately carries inline `prompt_body`, not `prompt:` file paths — i.e. it is shaped as a draft, not a back-converted pipeline (DD2).
- **No Workspace filesystem coupling** — ✅. The seed is pure in-memory data; nothing writes until `create_project_from_draft` → `write_project_pipeline`, exactly as today.
- **Naming reconciled** — partially; F1 sharpens it.

---

## Findings

### F1 [medium] §E-language-drift — "Template" risks re-merging with the dropped instantiate-on-create concept

**What:** DOMAIN.md removed **Template** when sub-project 3 dropped the instantiate-on-create path (line ~116 keeps it only as a struck/"may return" note). The plan re-introduces the word as **Template (seed)** (Task 5, DD7). The risk is the ubiquitous-language one: one word, two models — the *dropped* Template (a validated `Pipeline` written on create) and the *new* Template (seed) (a `DraftPipeline` the Design Session starts from). If DOMAIN.md doesn't draw the line explicitly, a future reader will collapse them and someone will "optimize" the seed straight onto the create path — the exact boundary sub-project 3 removed.

**Cited plan section:** Task 5 (DOMAIN.md edits); DD1, DD7.

**Why it matters:** *One model per bounded context* + *name the real concept*. The seed is a genuinely different concept from the dropped Template; the name must encode that, or the boundary erodes.

**Suggested amendment:** Task 5's "Template (seed)" bullet must say, in DOMAIN.md prose, (a) a seed yields a `DraftPipeline`, (b) it is **distinct from** the dropped instantiate-on-create Template, and (c) the wizard still owns create + hard validation. The plan's Task 5 Step 1 text already does all three. **Accepted as written** — this finding is satisfied by ensuring that exact bullet lands; no further change.

**Status:** resolved — Task 5 Step 1's bullet already encodes (a)/(b)/(c); the council confirms that wording is the required fix.

### F2 [low] §E-seam — seed completeness is an invariant the plan should name, not just test

**What:** The seed's value proposition (DD4) is that it is *immediately a complete, creatable draft* — every team has a non-empty `prompt_body`, every route points at a known node, `to_pipeline()` + hard-validate passes. The plan enforces this via tests (Task 1 Steps 2–3: `ddd_seed_is_a_clean_best_effort_draft`, `ddd_seed_hard_validates_after_to_pipeline`) but doesn't state it as a *property the registry guarantees*. A second seed added later could silently ship empty prompts (the original sub-project-3 bug) and only a test author would catch it.

**Cited plan section:** Task 1 (registry + tests), DD4.

**Why it matters:** *Invariants belong to the thing that owns them.* The seed registry owns "a seed is a complete draft"; that should be a documented contract on `seed_template`, so the test is enforcing a stated rule rather than an implicit one.

**Suggested amendment:** Add one sentence to the `seed_template.rs` module doc (Task 1 Step 2) stating the registry invariant: *every bundled seed is a complete, creatable `DraftPipeline` — non-empty prompt bodies, all routes resolve, passes hard validation after `to_pipeline()`*. The two existing tests then read as enforcing a named contract. Low effort, no code change beyond the doc-comment.

**Status:** resolved — apply the one-sentence invariant to the module doc-comment in Task 1.

### F3 [low] §E-language — "start from a template" UI copy vs the "kickoff" / "seed" domain terms

**What:** The plan's UI copy is "Or start from a template" (Task 4, DD6). The domain terms are **Design Session** / **kickoff** (the describe→generate one-shot) and now **seed**. The two kickoff entry points (describe→generate, start-from-template) are peers; the UI should make that legible without leaking the internal word "seed" but also without implying templates are a different *kind* of thing than generation.

**Cited plan section:** Task 4 Step 3 (picker UI), DD6.

**Why it matters:** *The language lives in the code / UI.* This is the only operator-facing surface of the concept; the copy is where the ubiquitous language meets the user. It need not say "seed" (internal), but it should read as a sibling of Generate, which "Or start from a template" does.

**Suggested amendment:** Accept the copy as proposed — "Or start from a template" correctly frames it as a peer kickoff path beside Generate, and keeps the internal "seed" term out of the UI. No change. (Recorded so the impeccable pass owns any further copy/visual refinement of the picker.)

**Status:** resolved — copy accepted as a peer-kickoff framing; visual polish deferred to the impeccable pass.

---

## Verdict

**SOUND WITH FIXES.** No boundary violation, no resurrection of the dropped instantiate-on-create path, `DraftPipeline`≠`Pipeline` preserved, no Workspace coupling. F1 (sharpen the Template-seed-vs-dropped-Template distinction in DOMAIN.md) and F2 (name the seed-completeness invariant on the registry) are in-plan doc amendments; F3 is accepted as written. All three resolved within the existing plan — proceed to implement.
