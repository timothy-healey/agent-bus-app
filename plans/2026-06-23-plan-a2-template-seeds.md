# A2 — Templates as Wizard Seeds Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Re-introduce bundled templates as *kickoff seeds* for the new-project wizard: in the Basics step, alongside describe→generate, the user can "start from a template" which seeds the `DraftPipeline`; they then refine it through the normal wizard steps and create as today.

**Architecture:** A small bundled-template registry in Pipeline Authoring that yields a **`DraftPipeline` seed** (NOT the old auto-instantiate-on-create path — templates seed the *draft*; the wizard still owns create + hard-validate). Two new OHS commands: `list_seed_templates()` returns `{id,name,description}` summaries; `seed_template(id) -> DraftPipeline` returns a populated draft (teams with inline `prompt_body`, gates, escalations, routes). The DDD spec→plan→implement content is recovered from git history and reshaped as a `DraftPipeline` seed. Kickoff UI gains a template picker beside describe→generate; both paths end in the same editable draft. `DraftPipeline` stays distinct from `Pipeline`; hard validation at create is unchanged.

**Tech Stack:** Rust (cargo workspace under `src-tauri/`, crate `pipeline`), Tauri commands (`app/src/lib.rs`), React + TypeScript (`src/wizard/`, `src/ipc/pipeline.ts`), vitest.

---

## File Structure

- **Create** `src-tauri/pipeline/src/seed_template.rs` — the bundled-template registry. A `SeedTemplate { id, name, description }` summary type, a `seed_templates()` catalog, and `seed_template(id) -> Option<DraftPipeline>` returning a populated draft. The DDD seed is built in Rust as a `DraftPipeline` (teams carry inline `prompt_body`; gates + escalations + routes). No `include_str!` / YAML file — the seed is a `DraftPipeline`, not a `Pipeline`.
- **Modify** `src-tauri/pipeline/src/lib.rs` — `pub mod seed_template;`.
- **Modify** `src-tauri/app/src/lib.rs` — two new commands `list_seed_templates_cmd` / `seed_template_cmd`; register both in `generate_handler!`.
- **Modify** `src/ipc/pipeline.ts` — `SeedTemplateSummary` type + `listSeedTemplates()` / `seedTemplate(id)` wrappers.
- **Modify** `src/wizard/NewProjectWizard.tsx` — Basics step gains a template picker; selecting a template seeds the draft (carrying name/description) and advances to `teams`.
- **Test** `src/ipc/pipeline.test.ts`, `src/wizard/NewProjectWizard.test.tsx` — frontend wiring tests.

## Decisions

- **DD1 — Seed shape: a `DraftPipeline`, not a `Pipeline`.** *Chosen: return a `DraftPipeline` directly.* The whole point of A2 is that templates seed the *editable draft* the wizard already owns — not the old instantiate-a-validated-`Pipeline`-on-create path that sub-project 3 dropped. So the registry yields `DraftPipeline` (inline `prompt_body`), the wizard refines it, and `create_project_from_draft` hard-validates exactly as for the describe→generate path. The `DraftPipeline`↔`Pipeline` distinction stays intact; the dropped auto-instantiate boundary is NOT resurrected.
- **DD2 — Where the seed content lives: built in Rust, not `include_str!` YAML.** *Chosen: construct the `DraftPipeline` in Rust code.* The old `ddd-spec-plan-impl.yaml` was a *validated `Pipeline`* with `prompt: prompts/<id>.md` file paths and `${target_repo}` scope vars. A seed needs inline `prompt_body` instead of prompt-file paths, and being a draft it doesn't need to satisfy hard validation. Building the `DraftPipeline` directly in Rust (recovering team/gate/route content from the git-history YAML) is clearer than parsing a YAML `Pipeline` and back-converting prompt paths into bodies. The content is recovered faithfully from `00ef101^:.../ddd-spec-plan-impl.yaml`.
- **DD3 — `seed_template(id)` returns the draft server-side (not bundled client-side).** *Chosen: a backend command.* Keeps the seed content as Pipeline Authoring's owned language (single source of truth), mirrors `kickoff_generate_cmd`'s shape (description→`DraftPipeline`), and means the frontend never hardcodes a pipeline. `list_seed_templates` returns light summaries for the picker; `seed_template(id)` returns the full draft on selection.
- **DD4 — Prompt bodies in the DDD seed.** *Chosen: short, faithful one-paragraph responsibility prompts per team.* The git-history template referenced `prompts/<id>.md` files that were never committed (sub-project 3's bug was empty prompts). The seed ships real, non-empty prompt bodies so a seeded draft passes `best_effort_validate` with no "team has no prompt yet" issues — the seed is immediately a complete, creatable draft.
- **DD5 — DDD seed schema_version + id.** *Chosen: `schema_version = SCHEMA_VERSION` (current), `id = "ddd-spec-plan-impl"`.* The recovered YAML was `schema_version: 1`; a fresh seed should be current (it carries no v1-only constructs). The id seeds the draft's basename; the user can rename in Basics (the wizard already lets `name`/`description` override post-seed, same as kickoff).
- **DD6 — UI placement.** *Chosen: a "Or start from a template" row of buttons under the Generate button in the Basics step.* Both paths (Generate, template pick) require `name` + `root` first (a template still needs a project name/root); description is optional when seeding from a template. Selecting a template seeds the draft and advances to `teams`, exactly like Generate.
- **DD7 — Naming reconciliation.** *Chosen: "Template (seed)" in DOMAIN.md.* DOMAIN.md line ~116 already anticipated "Templates may return as wizard seeds in v1.1." This plan registers **Template (seed)** under Pipeline Authoring as a named concept (a bundled `DraftPipeline` seed for the Design Session), distinct from the dropped instantiate-on-create Template, and updates the Workspace note. (Applied per the DDD vet.)

---

### Task 1: Seed-template registry (DDD seed as a DraftPipeline)

**Files:**
- Create: `src-tauri/pipeline/src/seed_template.rs`
- Modify: `src-tauri/pipeline/src/lib.rs`

- [ ] **Step 1: Add the module to the crate**

In `src-tauri/pipeline/src/lib.rs`, add alongside the other `pub mod` lines:

```rust
pub mod seed_template;
```

- [ ] **Step 2: Write the failing tests + the registry**

Create `src-tauri/pipeline/src/seed_template.rs`:

```rust
//! Bundled template **seeds** for the wizard's Design Session (DOMAIN.md →
//! Pipeline Authoring, "Template (seed)"). A seed is a `DraftPipeline` — the
//! editable, not-yet-valid draft the wizard owns — NOT the validated `Pipeline`
//! aggregate, and NOT the dropped instantiate-on-create path (sub-project 3).
//! `list_seed_templates()` returns light summaries for the kickoff picker;
//! `seed_template(id)` returns a populated draft that the wizard then refines and
//! creates through the normal hard-validate-at-create path. Prompt text is held
//! inline as `prompt_body` (like every draft), so a seeded draft is immediately a
//! complete, creatable draft.
//!
//! REGISTRY INVARIANT (vet F2): every bundled seed is a complete, creatable
//! `DraftPipeline` — non-empty prompt bodies on every team, all routes resolve to
//! known nodes, and it passes hard validation after `to_pipeline()`. The tests
//! enforce this named contract for the DDD seed.

use crate::draft::{DraftPipeline, DraftTeam};
use crate::model::{Escalation, Gate, Routes, SCHEMA_VERSION};
use serde::{Deserialize, Serialize};

/// A light summary of a bundled seed template, for the kickoff picker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeedTemplate {
    pub id: String,
    pub name: String,
    pub description: String,
}

/// The catalog of bundled seed templates (id/name/description only).
pub fn seed_templates() -> Vec<SeedTemplate> {
    vec![SeedTemplate {
        id: "ddd-spec-plan-impl".to_string(),
        name: "DDD Spec → Plan → Implement".to_string(),
        description: "Domain-driven design pipeline: research → spec → plan → \
            implement, with two human-review gates."
            .to_string(),
    }]
}

/// Return a populated `DraftPipeline` seed for the given template id, or `None`
/// for an unknown id (the OHS command maps `None` to an error).
pub fn seed_template(id: &str) -> Option<DraftPipeline> {
    match id {
        "ddd-spec-plan-impl" => Some(ddd_seed()),
        _ => None,
    }
}

/// Build a `DraftTeam` with an inline prompt body + a single on_approve route.
/// Helper so the seed reads as a flow. Other routes default to None.
fn team(id: &str, name: &str, prompt: &str, on_approve: &str) -> DraftTeam {
    let mut t = DraftTeam::new(id, name);
    t.prompt_body = prompt.to_string();
    t.outputs = Routes {
        on_approve: Some(on_approve.to_string()),
        on_revise: None,
        on_reject: None,
    };
    t
}

/// The DDD spec→plan→implement seed, recovered from the historical bundled
/// template (00ef101^:.../ddd-spec-plan-impl.yaml) and reshaped as a draft:
/// inline prompt bodies (DD4), two human gates, one escalation. schema_version
/// is current (DD5).
fn ddd_seed() -> DraftPipeline {
    let mut d = DraftPipeline::empty();
    d.id = "ddd-spec-plan-impl".to_string();
    d.name = "DDD Spec → Plan → Implement".to_string();
    d.description = "Domain-driven design pipeline with two human gates.".to_string();
    d.schema_version = SCHEMA_VERSION;

    d.teams = vec![
        team(
            "research",
            "Research",
            "You investigate the target repository and existing artifacts, then \
             write a findings/critique analysis the spec writers will build on.",
            "spec-writers",
        ),
        team(
            "spec-writers",
            "Spec Writers",
            "You turn the research findings into a clear specification document \
             with explicit requirements and boundaries.",
            "spec-reviewers",
        ),
        team(
            "spec-reviewers",
            "Spec Reviewers",
            "You review the specification for completeness, soundness, and clarity, \
             then approve, request revision, or reject.",
            "gate-1-spec",
        ),
        team(
            "plan-writers",
            "Plan Writers",
            "You turn the approved specification into a concrete, step-by-step \
             implementation plan.",
            "plan-reviewers",
        ),
        team(
            "plan-reviewers",
            "Plan Reviewers",
            "You review the implementation plan against the spec, then approve, \
             request revision, or reject.",
            "gate-2-plan",
        ),
        team(
            "implementers",
            "Implementers",
            "You implement the approved plan in a worktree, committing the changes.",
            "done",
        ),
        {
            let mut done = DraftTeam::new("done", "Done");
            done.prompt_body =
                "You summarize the completed work. Terminal node — no further routing."
                    .to_string();
            done
        },
    ];

    // Revise/reject edges (recovered from the historical template).
    for t in d.teams.iter_mut() {
        match t.id.as_str() {
            "spec-writers" => {
                t.outputs.on_revise = Some("research".into());
                t.outputs.on_reject = Some("needs-human".into());
            }
            "spec-reviewers" => {
                t.outputs.on_revise = Some("spec-writers".into());
                t.outputs.on_reject = Some("needs-human".into());
            }
            "plan-writers" => {
                t.outputs.on_revise = Some("spec-writers".into());
                t.outputs.on_reject = Some("needs-human".into());
            }
            "plan-reviewers" => {
                t.outputs.on_revise = Some("plan-writers".into());
                t.outputs.on_reject = Some("needs-human".into());
            }
            "implementers" => {
                t.outputs.on_revise = Some("plan-writers".into());
                t.outputs.on_reject = Some("needs-human".into());
            }
            _ => {}
        }
    }

    d.gates = vec![
        Gate {
            id: "gate-1-spec".into(),
            label: "Gate 1 — Spec Approval".into(),
            downstream: "plan-writers".into(),
        },
        Gate {
            id: "gate-2-plan".into(),
            label: "Gate 2 — Plan Approval".into(),
            downstream: "implementers".into(),
        },
    ];

    d.escalations = vec![Escalation {
        id: "needs-human".into(),
        triggers: vec!["attempts >= 3".into(), "verdict == reject".into()],
    }];

    d
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draft::best_effort_validate;

    #[test]
    fn catalog_contains_the_ddd_seed() {
        let t = seed_templates();
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].id, "ddd-spec-plan-impl");
        assert!(!t[0].name.is_empty());
        assert!(!t[0].description.is_empty());
    }

    #[test]
    fn unknown_id_returns_none() {
        assert!(seed_template("nope").is_none());
    }

    #[test]
    fn ddd_seed_is_a_draft_with_seven_teams_and_two_gates() {
        let d = seed_template("ddd-spec-plan-impl").unwrap();
        assert_eq!(d.teams.len(), 7);
        assert_eq!(d.gates.len(), 2);
        assert_eq!(d.escalations.len(), 1);
        assert_eq!(d.id, "ddd-spec-plan-impl");
        assert_eq!(d.schema_version, SCHEMA_VERSION);
    }

    #[test]
    fn ddd_seed_teams_all_have_inline_prompt_bodies() {
        // DD4: the seed is immediately complete — no empty prompts.
        let d = seed_template("ddd-spec-plan-impl").unwrap();
        for t in &d.teams {
            assert!(!t.prompt_body.trim().is_empty(), "team {} has no prompt body", t.id);
        }
    }

    #[test]
    fn ddd_seed_is_a_clean_best_effort_draft() {
        // Every route points at a known node (team/gate/escalation) and every
        // team has a prompt — best-effort validation is silent.
        let d = seed_template("ddd-spec-plan-impl").unwrap();
        assert_eq!(best_effort_validate(&d), Vec::<String>::new());
    }

    #[test]
    fn ddd_seed_hard_validates_after_to_pipeline() {
        // The seed is a complete, creatable draft: to_pipeline() + hard validate
        // passes, so the normal create path accepts it unmodified.
        let d = seed_template("ddd-spec-plan-impl").unwrap();
        let p = d.to_pipeline();
        let resolved = crate::resolve::resolve_defaults(&p);
        assert_eq!(crate::validate::validate(&resolved), Ok(()));
    }
}
```

- [ ] **Step 3: Run the tests, expect FAIL then PASS**

Run: `cd /Users/tim/projects/agent-bus-app/src-tauri && cargo test -p pipeline seed_template`
Expected: compiles, all `seed_template::tests` pass. If `ddd_seed_hard_validates_after_to_pipeline` fails, the route graph has an unreachable node — fix the seed's routes/gates so hard validate passes (the recovered graph is known-valid).

- [ ] **Step 4: Commit**

```bash
cd /Users/tim/projects/agent-bus-app
git add src-tauri/pipeline/src/seed_template.rs src-tauri/pipeline/src/lib.rs
git commit -m "feat(pipeline): bundled seed-template registry yielding a DraftPipeline (A2)"
```

---

### Task 2: OHS commands — list_seed_templates / seed_template

**Files:**
- Modify: `src-tauri/app/src/lib.rs`

- [ ] **Step 1: Add the two commands**

In `src-tauri/app/src/lib.rs`, near `kickoff_generate_cmd` (~line 541), add:

```rust
/// OHS: list the bundled seed templates (id/name/description) for the kickoff
/// picker. Pure pass-through to Pipeline Authoring's registry — no state, no chat.
#[tauri::command(rename_all = "snake_case")]
fn list_seed_templates_cmd() -> Vec<pipeline::seed_template::SeedTemplate> {
    pipeline::seed_template::seed_templates()
}

/// OHS: return a populated `DraftPipeline` seed for a template id. Unknown id is
/// an error. Mirrors `kickoff_generate_cmd`'s shape (→ DraftPipeline); the wizard
/// refines it and creates through the normal hard-validate path.
#[tauri::command(rename_all = "snake_case")]
fn seed_template_cmd(id: String) -> Result<DraftPipeline, String> {
    pipeline::seed_template::seed_template(&id).ok_or_else(|| format!("unknown seed template: {id}"))
}
```

- [ ] **Step 2: Register both in the invoke handler**

In the `tauri::generate_handler![ ... ]` list (~line 960), add after `kickoff_generate_cmd,`:

```rust
            list_seed_templates_cmd,
            seed_template_cmd,
```

- [ ] **Step 3: Add a backend test**

In the `#[cfg(test)] mod tests` block of `app/src/lib.rs` (near the `root_kickoff_produces_a_draft_from_a_canned_reply` test ~line 1374), add:

```rust
    #[test]
    fn seed_template_cmd_returns_a_draft_for_a_known_id() {
        let d = pipeline::seed_template::seed_template("ddd-spec-plan-impl").unwrap();
        assert_eq!(d.teams.len(), 7);
        assert!(pipeline::seed_template::seed_template("nope").is_none());
        assert!(!pipeline::seed_template::seed_templates().is_empty());
    }
```

- [ ] **Step 4: Run it**

Run: `cd /Users/tim/projects/agent-bus-app/src-tauri && cargo test -p app seed_template_cmd && cargo check -p app`
Expected: PASS + clean check (the commands compile into the handler).

- [ ] **Step 5: Commit**

```bash
cd /Users/tim/projects/agent-bus-app
git add src-tauri/app/src/lib.rs
git commit -m "feat(app): list_seed_templates / seed_template OHS commands (A2)"
```

---

### Task 3: Frontend IPC wrappers

**Files:**
- Modify: `src/ipc/pipeline.ts`
- Test: `src/ipc/pipeline.test.ts`

- [ ] **Step 1: Write the failing test**

In `src/ipc/pipeline.test.ts`, add (match the existing mocking style in that file — it mocks `@tauri-apps/api/core`'s `invoke`):

```ts
import { listSeedTemplates, seedTemplate } from "./pipeline";

it("listSeedTemplates invokes list_seed_templates_cmd", async () => {
  const invoke = vi.mocked((await import("@tauri-apps/api/core")).invoke);
  invoke.mockResolvedValueOnce([{ id: "ddd-spec-plan-impl", name: "DDD", description: "d" }]);
  const out = await listSeedTemplates();
  expect(invoke).toHaveBeenCalledWith("list_seed_templates_cmd");
  expect(out[0].id).toBe("ddd-spec-plan-impl");
});

it("seedTemplate invokes seed_template_cmd with the id", async () => {
  const invoke = vi.mocked((await import("@tauri-apps/api/core")).invoke);
  invoke.mockResolvedValueOnce({ id: "ddd-spec-plan-impl", name: "DDD", description: "", schema_version: 2, teams: [], forks: [], joins: [], gates: [], escalations: [] });
  const out = await seedTemplate("ddd-spec-plan-impl");
  expect(invoke).toHaveBeenCalledWith("seed_template_cmd", { id: "ddd-spec-plan-impl" });
  expect(out.id).toBe("ddd-spec-plan-impl");
});
```

(If `pipeline.test.ts` sets up the `invoke` mock differently, follow that file's existing pattern — the assertions on call name + args are what matters.)

- [ ] **Step 2: Run it, expect FAIL**

Run: `cd /Users/tim/projects/agent-bus-app && /opt/homebrew/bin/bun vitest run src/ipc/pipeline.test.ts`
Expected: FAIL — `listSeedTemplates`/`seedTemplate` not exported.

- [ ] **Step 3: Add the types + wrappers**

In `src/ipc/pipeline.ts`, after the `kickoffGenerate` export, add:

```ts
/** A bundled seed template summary for the kickoff picker (A2). */
export interface SeedTemplateSummary {
  id: string;
  name: string;
  description: string;
}

export async function listSeedTemplates(): Promise<SeedTemplateSummary[]> {
  return await invoke<SeedTemplateSummary[]>("list_seed_templates_cmd");
}

/** Return a populated DraftPipeline seed for a template id; the wizard refines it. */
export async function seedTemplate(id: string): Promise<DraftPipeline> {
  return await invoke<DraftPipeline>("seed_template_cmd", { id });
}
```

- [ ] **Step 4: Run it, expect PASS**

Run: `cd /Users/tim/projects/agent-bus-app && /opt/homebrew/bin/bun vitest run src/ipc/pipeline.test.ts`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cd /Users/tim/projects/agent-bus-app
git add src/ipc/pipeline.ts src/ipc/pipeline.test.ts
git commit -m "feat(web): listSeedTemplates / seedTemplate IPC wrappers (A2)"
```

---

### Task 4: Wizard Basics-step template picker

**Files:**
- Modify: `src/wizard/NewProjectWizard.tsx`
- Test: `src/wizard/NewProjectWizard.test.tsx`

- [ ] **Step 1: Write the failing test**

In `src/wizard/NewProjectWizard.test.tsx`, add a test that mocks the IPC and asserts a template pick seeds the draft + advances. Mock `../ipc/pipeline`'s `listSeedTemplates` + `seedTemplate` alongside the existing `kickoffGenerate` mock (follow the file's existing mock setup):

```tsx
it("starting from a template seeds the draft and advances to teams", async () => {
  const seeded = {
    id: "ddd-spec-plan-impl", name: "", description: "",
    schema_version: 2,
    teams: [{ id: "research", name: "Research", prompt_body: "x", runner: { kind: "claude-cli", model: "claude-opus-4-8", effort: { mode: "standard" }, api_key_env: null }, scope: { reads: [], writes: [], tools: [] }, outputs: {}, workers: { default: 1, max: 1 } }],
    forks: [], joins: [], gates: [], escalations: [],
  };
  const ipc = await import("../ipc/pipeline");
  vi.mocked(ipc.listSeedTemplates).mockResolvedValue([{ id: "ddd-spec-plan-impl", name: "DDD Spec → Plan → Implement", description: "d" }]);
  vi.mocked(ipc.seedTemplate).mockResolvedValue(seeded as any);

  render(<NewProjectWizard open onClose={() => {}} onCreated={() => {}} />);
  fireEvent.change(screen.getByLabelText("Project name"), { target: { value: "Demo" } });
  fireEvent.change(screen.getByLabelText("Root path"), { target: { value: "~/p/demo" } });

  // the picker button appears once templates load
  const btn = await screen.findByRole("button", { name: /DDD Spec/ });
  fireEvent.click(btn);

  // advanced to teams: the team from the seed renders
  expect(await screen.findByText("Research")).toBeInTheDocument();
  expect(ipc.seedTemplate).toHaveBeenCalledWith("ddd-spec-plan-impl");
});
```

(Adjust imports — `render`, `screen`, `fireEvent` from `@testing-library/react` — to match the file's existing harness. If the existing tests mock `../ipc/pipeline` with a factory, extend that factory so `listSeedTemplates`/`seedTemplate` are `vi.fn()`.)

- [ ] **Step 2: Run it, expect FAIL**

Run: `cd /Users/tim/projects/agent-bus-app && /opt/homebrew/bin/bun vitest run src/wizard/NewProjectWizard.test.tsx`
Expected: FAIL — no template picker button.

- [ ] **Step 3: Implement the picker**

In `src/wizard/NewProjectWizard.tsx`:

Update the import on line 3:

```tsx
import { kickoffGenerate, listSeedTemplates, seedTemplate, type DraftPipeline, type SeedTemplateSummary, type Step } from "../ipc/pipeline";
```

Add `useEffect` to the React import (line 2):

```tsx
import { useEffect, useMemo, useState } from "react";
```

Add state + a load effect + a seed handler inside the component (after the `sessionId` line ~35):

```tsx
  const [templates, setTemplates] = useState<SeedTemplateSummary[]>([]);

  useEffect(() => {
    if (!open) return;
    let active = true;
    listSeedTemplates().then((t) => { if (active) setTemplates(t); }).catch(() => {});
    return () => { active = false; };
  }, [open]);

  async function startFromTemplate(id: string) {
    setBusy(true);
    try {
      const d = await seedTemplate(id);
      // Carry the user's name (description optional when seeding); keep the
      // template's draft otherwise. Both paths end in the same editable draft.
      setDraft({ ...d, name, description: description.trim() || d.description });
      go("teams");
    } finally {
      setBusy(false);
    }
  }
```

In the `step === "basics"` block, after the Generate button (~line 67), add the picker:

```tsx
            {templates.length > 0 && (
              <div style={{ marginTop: "var(--sp-4)" }}>
                <div style={{ fontSize: 11, color: "var(--text-3)", marginBottom: "var(--sp-2)" }}>Or start from a template</div>
                <div style={{ display: "flex", flexWrap: "wrap", gap: 8 }}>
                  {templates.map((t) => (
                    <button
                      key={t.id}
                      title={t.description}
                      onClick={() => startFromTemplate(t.id)}
                      disabled={busy || !name.trim() || !root.trim()}
                    >
                      {t.name}
                    </button>
                  ))}
                </div>
              </div>
            )}
```

- [ ] **Step 4: Run it, expect PASS**

Run: `cd /Users/tim/projects/agent-bus-app && /opt/homebrew/bin/bun vitest run src/wizard/NewProjectWizard.test.tsx`
Expected: PASS. The existing describe→Generate test must stay green.

- [ ] **Step 5: Commit**

```bash
cd /Users/tim/projects/agent-bus-app
git add src/wizard/NewProjectWizard.tsx src/wizard/NewProjectWizard.test.tsx
git commit -m "feat(wizard): start-from-template kickoff seeds the draft (A2)"
```

---

### Task 5: DOMAIN.md naming reconciliation (DD7 / vet)

**Files:**
- Modify: `DOMAIN.md`

- [ ] **Step 1: Register "Template (seed)" under Pipeline Authoring**

In `DOMAIN.md`, in the Pipeline Authoring section (near the `DraftPipeline` / `Design Session` entries ~lines 73–74), add a bullet:

```markdown
- **Template (seed)** — a bundled `DraftPipeline` seed the Design Session can start from (A2). In the wizard's Basics step the user may "start from a template" (e.g. the DDD spec→plan→implement pipeline) instead of, or alongside, the one-shot describe→generate; the seed populates the `DraftPipeline` and the user refines it through the normal wizard steps and creates as usual. Distinct from the **dropped** instantiate-on-create Template (sub-project 3): a seed seeds the *draft*; the wizard still owns create + hard validation. `list_seed_templates` returns id/name/description summaries; `seed_template(id)` returns the populated draft.
```

- [ ] **Step 2: Update the Workspace note**

In `DOMAIN.md` line ~116 ("Project write surface"), change the trailing parenthetical from:

```
(The bundled-**Template** instantiation path was dropped in sub-project 3; the wizard writes YAML directly. Templates may return as wizard seeds in v1.1.)
```

to:

```
(The bundled-**Template** instantiation path was dropped in sub-project 3; the wizard writes YAML directly. Templates returned in v1.1 as **wizard seeds** — bundled `DraftPipeline` seeds for the Design Session, NOT the dropped instantiate-on-create path; see Pipeline Authoring → "Template (seed)" (A2).)
```

- [ ] **Step 3: Commit**

```bash
cd /Users/tim/projects/agent-bus-app
git add DOMAIN.md
git commit -m "docs(domain): register Template (seed) under Pipeline Authoring (A2, vet DD7)"
```

---

### Task 6: Full verification + merge + tag + backlog

- [ ] **Step 1: Full workspace verification**

```bash
cd /Users/tim/projects/agent-bus-app/src-tauri
cargo test --workspace
cargo check --workspace
cargo clippy --workspace
cd /Users/tim/projects/agent-bus-app
/opt/homebrew/bin/bun vitest run
/opt/homebrew/bin/bun run build
```
Expected: all green; clippy clean (no warnings introduced).

- [ ] **Step 2: Merge --no-ff into main + tag**

```bash
cd /Users/tim/projects/agent-bus-app
git checkout main
git merge --no-ff plan-a2-template-seeds -m "Merge plan-a2-template-seeds: templates as wizard seeds (A2)"
git tag plan-a2-template-seeds
```

- [ ] **Step 3: Mark A2 done in the backlog**

In `docs/v1.1-backlog.md`, change the A2 line under "Pipeline authoring" to `- [x] **A2 · Templates as wizard seeds** — `done` (tag `plan-a2-template-seeds`) — ...` with a one-line summary. Then:

```bash
cd /Users/tim/projects/agent-bus-app
git add docs/v1.1-backlog.md
git commit -m "docs(backlog): mark A2 done (tag plan-a2-template-seeds)"
```

(Do NOT push. Leave the repo on `main`.)

---

## Self-Review

- **Spec coverage:** registry (T1) + OHS commands (T2) + IPC (T3) + UI picker (T4) + DOMAIN naming (T5) + verify/merge/backlog (T6) cover the A2 requirement (templates seed the DraftPipeline; both kickoff paths end in the same editable draft; DraftPipeline≠Pipeline; hard-validate at create unchanged).
- **Placeholder scan:** none — all code is concrete; route content recovered from git history.
- **Type consistency:** `SeedTemplate` (Rust) ↔ `SeedTemplateSummary` (TS, same fields); `seed_template(id) -> Option<DraftPipeline>` ↔ `seedTemplate(id): Promise<DraftPipeline>`; command names `list_seed_templates_cmd` / `seed_template_cmd` match across Rust + IPC + tests.
