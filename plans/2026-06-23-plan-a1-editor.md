# A1 — Pipeline Editor Write-Mode Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let an operator edit an existing project's active pipeline graph in-app (reusing the wizard's step editors) and save it back over the project's YAML + prompt files, then reload.

**Architecture:** Reuse the wizard's `DraftPipeline` draft-editing machinery end-to-end. Add a Rust `from_pipeline(&Pipeline, prompt_bodies) -> DraftPipeline` converter (the faithful reverse of `to_pipeline`), exposed as a `pipeline_to_draft_cmd` OHS command. Add a `save_pipeline_edits_inner` orchestration at the composition root that mirrors `create_project_from_draft_inner` (hard-validate → `write_project_pipeline` overwrite → no row create). The frontend adds an "Edit pipeline" affordance in the Pipeline view that opens a `PipelineEditor` modal reusing `TeamsStep`/`PromptsStep`/`WiringStep` (NOT a parallel editor), seeded by loading the active pipeline into a draft (reading each team's prompt body via `read_artifact`), and saves via a `save_pipeline_edits` IPC, then reloads.

**Tech Stack:** Rust (cargo workspace under `src-tauri/`, `pipeline`/`workspace`/`app` crates, serde, sqlx), React + TypeScript + Vite, Tauri commands, vitest.

---

## Decisions

All forks auto-decided with the recommended option (operator AFK).

- **D1 — Converter lives in Pipeline Authoring (`pipeline/src/draft.rs`), reverse of `to_pipeline`.** `from_pipeline` takes the validated `Pipeline` plus a `prompt_bodies: &HashMap<String,String>` (team-id → file content the caller read via `read_artifact`). It is the inverse of `to_pipeline`: each `Team` → `DraftTeam` with `prompt_body` from the map (empty string if absent — tolerant), `runner` from `team.effective_runner()` (the loaded pipeline is already resolved by the store), and scope/outputs/workers/forks/joins/gates/escalations carried straight through. Keeps `DraftPipeline` ≠ `Pipeline` distinction intact. *(Recommended.)*

- **D2 — Round-trip faithfulness is the test contract.** `from_pipeline(p, bodies).to_pipeline()` must equal `p` for any pipeline `to_pipeline` can produce, given `bodies` derived from `prompt_files`. `to_pipeline` always wraps the runner as a complete `Some(TeamRunnerConfig::from_full(..))` and always sets `defaults: None`; therefore the round-trip target is the *resolved* pipeline shape (every team `Some(full)`, `defaults: None`). The from→to test asserts equality against that shape. *(Recommended — strongest invariant.)*

- **D3 — Prompt bodies read at the composition root, not inside Pipeline Authoring.** Pipeline Authoring never touches the filesystem (DDD: Workspace owns IO). The converter receives already-read bodies. The root command `pipeline_to_draft_cmd` reads each team's prompt file via Workspace's `read_artifact` path-resolution, builds the map, then calls `from_pipeline`. *(Recommended — no PA filesystem coupling, mirrors create flow's split.)*

- **D4 — Save reuses `write_project_pipeline` (overwrite); no new write path, no row create.** `save_pipeline_edits_inner` mirrors `create_project_from_draft_inner` minus the project-row insert and minus tilde-expansion (the project already exists). It hard-validates `draft.to_pipeline()`, serializes YAML, computes `prompt_files`, writes to `pipelines/<id>.yaml` (same convention as create) via `write_project_pipeline_inner`, then re-activates the pipeline id (idempotent — already active, but keeps the active pointer correct if the id is unchanged). Overwriting the same YAML + prompt paths is safe and idempotent (plain `std::fs::write`). *(Recommended.)*

- **D5 — The edited pipeline keeps its id.** The draft seeded from the active pipeline carries the original `id`; saving overwrites `pipelines/<id>.yaml`. We do not rename or create a second file. If a team is removed during editing, its now-orphaned `prompts/<old-id>.md` is left on disk (harmless — not referenced by the YAML). A garbage-collect of orphan prompt files is out of scope (note as `TODO(a1)` only if it surfaces; it does not block). *(Recommended — minimal, safe.)*

- **D6 — Frontend reuses the three wizard step components directly.** `PipelineEditor.tsx` is a thin modal shell that renders `TeamsStep` / `PromptsStep` / `WiringStep` against editor-local draft state with simple Back/Next/Save chrome and a live best-effort issues banner (reusing `bestEffortValidate`). It does NOT embed `ChatDraftPanel` (no Design Session chat in edit-mode — editing is manual). It does NOT fork a second set of step editors. *(Recommended — refactor-before-add; avoids a parallel editor.)* **Vet F1:** edit-mode is deliberately **not a Design Session** — it reuses only the draft-editing half of the wizard (`DraftPipeline` + step editors + best-effort validation), never the LLM Chat ACL / `dialogue_id`. The ubiquitous-language term is **"Pipeline edit-mode"** (registered in DOMAIN.md), keeping the LLM Chat ACL the only chat seam.

- **D9 — Create + save share one validate→serialize→prompt-files core (vet F2).** Extract a pure `prepare_pipeline_write(&DraftPipeline) -> Result<(yaml_rel, yaml, prompts), String>` in `pipeline::draft` (hard-validate the `to_pipeline()`, serialize YAML, compute `prompt_files`, build `pipelines/<id>.yaml`). Both `create_project_from_draft_inner` and `save_pipeline_edits_inner` call it, so the "nothing is written when invalid" gate lives in one place and can't drift. Pipeline Authoring owns it (it serializes; Workspace still writes). *(Recommended — Refactor before you add.)*

- **D7 — "Edit pipeline" affordance sits in `PipelineView` via an optional `onEdit` prop.** `PipelineView` stays a pure presentational viewer; `App.tsx` passes `onEdit` (only when a pipeline + active project exist) which opens the `PipelineEditor`. The stale "editing arrives in v1.1" caption is replaced. *(Recommended — keeps the viewer dumb.)*

- **D8 — Save errors surface in the editor; nothing is written on hard-validation failure.** Same contract as create: `save_pipeline_edits_inner` validates before any write; an invalid draft returns an `Err` string shown in the editor, no partial write. *(Recommended.)*

---

## File Structure

**Rust**
- `src-tauri/pipeline/src/draft.rs` — add `from_pipeline(&Pipeline, &HashMap<String,String>) -> DraftPipeline` + tests (the converter + round-trip).
- `src-tauri/app/src/lib.rs` — add `pipeline_to_draft_cmd` (read prompt bodies via Workspace + convert) and `save_pipeline_edits_inner` / `save_pipeline_edits` command; register both in the `invoke_handler`.

**TypeScript**
- `src/ipc/pipeline.ts` — add `pipelineToDraft(projectId, projectRoot, id)` and `savePipelineEdits(projectId, draft)` IPC wrappers.
- `src/components/PipelineView.tsx` — add optional `onEdit?: () => void`; render an "Edit pipeline" button; drop the stale caption.
- `src/wizard/PipelineEditor.tsx` — NEW: modal reusing `TeamsStep`/`PromptsStep`/`WiringStep` over editor-local draft, Save → `savePipelineEdits`.
- `src/App.tsx` — wire `onEdit` on the pipeline view to open the editor (seed via `pipelineToDraft`), and reload the pipeline on save.

**Tests**
- `src-tauri/pipeline/src/draft.rs` (`#[cfg(test)]`) — converter + round-trip.
- `src-tauri/app/src/lib.rs` (`#[cfg(test)]`) — `save_pipeline_edits_inner` overwrite + reject-invalid; `pipeline_to_draft_cmd` body wiring (via inner helper).
- `src/wizard/PipelineEditor.test.tsx` — NEW vitest: renders seeded draft, edits a team name, Save calls the IPC with the edited draft.
- `src/components/PipelineView.test.tsx` — add: renders an Edit button when `onEdit` given, calls it on click. (Create if absent.)

---

## Task 1: `from_pipeline` converter (Pipeline Authoring)

**Files:**
- Modify: `src-tauri/pipeline/src/draft.rs`
- Test: `src-tauri/pipeline/src/draft.rs` (`#[cfg(test)]`)

- [ ] **Step 1: Write the failing round-trip test**

Add to the `tests` module in `draft.rs`:

```rust
    #[test]
    fn from_pipeline_is_the_inverse_of_to_pipeline() {
        use std::collections::HashMap;
        // Start from a complete draft, project it to a (resolved) Pipeline, then
        // back. The result must equal the resolved Pipeline to_pipeline emits.
        let draft = complete_draft();
        let pipeline = draft.to_pipeline();
        // bodies the root would read back from prompts/<id>.md
        let bodies: HashMap<String, String> = prompt_files(&draft)
            .into_iter()
            .map(|(path, body)| {
                let id = path.trim_start_matches("prompts/").trim_end_matches(".md").to_string();
                (id, body)
            })
            .collect();

        let back_draft = DraftPipeline::from_pipeline(&pipeline, &bodies);
        // to_pipeline of the reconstructed draft must equal the original pipeline
        assert_eq!(back_draft.to_pipeline(), pipeline);
        // prompt bodies were restored
        let research = back_draft.teams.iter().find(|t| t.id == "research").unwrap();
        assert_eq!(research.prompt_body, "You investigate the repo.");
    }

    #[test]
    fn from_pipeline_carries_forks_joins_gates_and_runner() {
        use std::collections::HashMap;
        use crate::model::{Fork, Gate, Join};
        let mut d = DraftPipeline::empty();
        d.id = "demo".into();
        d.name = "Demo".into();
        let mut a = DraftTeam::new("entry", "Entry");
        a.prompt_body = "x".into();
        a.outputs.on_approve = Some("fork-1".into());
        let mut la = DraftTeam::new("lane-a", "Lane A");
        la.prompt_body = "x".into();
        la.outputs.on_approve = Some("join-1".into());
        let mut lb = DraftTeam::new("lane-b", "Lane B");
        lb.prompt_body = "x".into();
        lb.outputs.on_approve = Some("join-1".into());
        let mut after = DraftTeam::new("after", "After");
        after.prompt_body = "x".into();
        d.teams.push(a); d.teams.push(la); d.teams.push(lb); d.teams.push(after);
        d.forks.push(Fork { id: "fork-1".into(), lanes: vec!["lane-a".into(), "lane-b".into()] });
        d.joins.push(Join { id: "join-1".into(), waits_for: vec!["lane-a".into(), "lane-b".into()], downstream: "after".into(), cancel_on_reject: false, quorum: Some(2) });
        d.gates.push(Gate { id: "g".into(), label: "G".into(), downstream: "after".into() });

        let pipeline = d.to_pipeline();
        let bodies: HashMap<String, String> = prompt_files(&d).into_iter()
            .map(|(p, b)| (p.trim_start_matches("prompts/").trim_end_matches(".md").to_string(), b))
            .collect();
        let back = DraftPipeline::from_pipeline(&pipeline, &bodies);
        assert_eq!(back.forks, d.forks);
        assert_eq!(back.joins, d.joins);
        assert_eq!(back.joins[0].quorum, Some(2));
        assert_eq!(back.gates, d.gates);
        // runner survives the round-trip through effective_runner
        assert_eq!(back.teams[0].runner.kind, agent_bus_core::RunnerKind::ClaudeCli);
    }

    #[test]
    fn from_pipeline_tolerates_a_missing_prompt_body() {
        use std::collections::HashMap;
        let pipeline = complete_draft().to_pipeline();
        let back = DraftPipeline::from_pipeline(&pipeline, &HashMap::new());
        // no bodies supplied -> empty prompt_body, not a panic
        assert!(back.teams.iter().all(|t| t.prompt_body.is_empty()));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src-tauri && cargo test -p pipeline from_pipeline 2>&1 | tail -20`
Expected: FAIL — `no function or associated item named 'from_pipeline'`.

- [ ] **Step 3: Implement `from_pipeline`**

Add `use std::collections::HashMap;` near the top of `draft.rs` (after the existing `use` lines). Add this associated function inside the existing `impl DraftPipeline { ... }` block that holds `to_pipeline` (place it just above `to_pipeline`):

```rust
    /// Reconstruct a draft from a validated, already-resolved `Pipeline` (A1).
    /// The faithful inverse of `to_pipeline`: each team's `prompt_body` comes from
    /// `prompt_bodies` (team-id → file content the caller read via Workspace's
    /// read_artifact; missing => empty, tolerant), the runner from the resolved
    /// `effective_runner()`, and forks/joins/gates/escalations/scope/outputs carry
    /// straight through. Pipeline Authoring never reads files itself (Workspace owns
    /// IO) — the bodies are injected. `DraftPipeline` stays distinct from `Pipeline`.
    pub fn from_pipeline(pipeline: &Pipeline, prompt_bodies: &HashMap<String, String>) -> Self {
        Self {
            id: pipeline.id.clone(),
            name: pipeline.name.clone(),
            description: pipeline.description.clone(),
            schema_version: pipeline.schema_version,
            teams: pipeline
                .teams
                .iter()
                .map(|t| DraftTeam {
                    id: t.id.clone(),
                    name: t.name.clone(),
                    prompt_body: prompt_bodies.get(&t.id).cloned().unwrap_or_default(),
                    runner: t.effective_runner(),
                    scope: t.scope.clone(),
                    outputs: t.outputs.clone(),
                    workers: t.workers.clone(),
                })
                .collect(),
            forks: pipeline.forks.clone(),
            joins: pipeline.joins.clone(),
            gates: pipeline.gates.clone(),
            escalations: pipeline.escalations.clone(),
        }
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test -p pipeline from_pipeline 2>&1 | tail -20`
Expected: PASS (3 tests).

- [ ] **Step 5: Add the shared `prepare_pipeline_write` helper (vet F2)**

Add a test to `draft.rs`'s `tests` module:

```rust
    #[test]
    fn prepare_pipeline_write_returns_yaml_path_yaml_and_prompts() {
        let (yaml_rel, yaml, prompts) = prepare_pipeline_write(&complete_draft()).unwrap();
        assert_eq!(yaml_rel, "pipelines/demo.yaml");
        assert!(yaml.contains("id: demo"));
        assert!(prompts.iter().any(|(p, _)| p == "prompts/research.md"));
    }

    #[test]
    fn prepare_pipeline_write_errors_on_an_invalid_draft_and_writes_nothing() {
        let mut d = complete_draft();
        d.teams[0].outputs.on_approve = Some("ghost".into()); // dangling route
        assert!(prepare_pipeline_write(&d).is_err());
    }
```

Run: `cd src-tauri && cargo test -p pipeline prepare_pipeline_write 2>&1 | tail -10`
Expected: FAIL — `cannot find function prepare_pipeline_write`.

Then add this free function to `draft.rs` (next to `to_yaml`/`prompt_files`):

```rust
/// The shared validate-then-serialize core for both the create-from-draft and the
/// edit-mode save flows (A1; vet F2). HARD-validates the draft's Pipeline, then
/// returns the bytes to write: the project-root-relative YAML path
/// (`pipelines/<id>.yaml`), the serialized YAML, and the per-team prompt files.
/// An invalid draft is an `Err` and nothing is returned — the single home of the
/// "nothing is written when invalid" gate. Pipeline Authoring serializes;
/// Workspace writes (the caller passes these to `write_project_pipeline`).
pub fn prepare_pipeline_write(
    draft: &DraftPipeline,
) -> Result<(String, String, Vec<(String, String)>), String> {
    let pipeline = draft.to_pipeline();
    crate::validate::validate(&pipeline).map_err(|e| e.to_string())?;
    let yaml = to_yaml(&pipeline).map_err(|e| e.to_string())?;
    let prompts = prompt_files(draft);
    let yaml_rel = format!("pipelines/{}.yaml", pipeline.id);
    Ok((yaml_rel, yaml, prompts))
}
```

Run: `cd src-tauri && cargo test -p pipeline prepare_pipeline_write 2>&1 | tail -10`
Expected: PASS (2 tests).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/pipeline/src/draft.rs
git commit -m "feat(pipeline): from_pipeline converter + prepare_pipeline_write core for A1 editor"
```

---

## Task 2: Backend commands — `pipeline_to_draft_cmd` + `save_pipeline_edits`

**Files:**
- Modify: `src-tauri/app/src/lib.rs` (add commands near `create_project_from_draft`, register in `invoke_handler`)
- Test: `src-tauri/app/src/lib.rs` (`#[cfg(test)]`)

Context: `create_project_from_draft_inner` (lib.rs:590) is the template. The active project is resolved from the Workspace store; `write_project_pipeline_inner` (workspace/src/api.rs:138) overwrites. `read_artifact` resolution logic is `resolve_under_root` + `std::fs::read_to_string`. We expose an inner helper that reads bodies from the project root so it is testable without Tauri State.

- [ ] **Step 1: Write the failing tests**

Find the existing `#[cfg(test)]` block in `app/src/lib.rs` (it already has `complete_draft(...)` and `create_project_from_draft_inner` tests around line 1390–1480). Add these tests to that module:

```rust
    #[tokio::test]
    async fn save_pipeline_edits_overwrites_yaml_and_prompts() {
        let root = std::env::temp_dir().join(format!("abp-save-{}", uuid::Uuid::new_v4()));
        let ws = test_workspace().await;
        // create first via the create flow so the project + files exist
        let project = create_project_from_draft_inner(&ws, "Demo".into(), root.to_string_lossy().into(), complete_draft("demo"))
            .await.unwrap();

        // edit: change a team prompt body, then save
        let mut edited = complete_draft("demo");
        edited.teams[0].prompt_body = "REWRITTEN body".into();
        save_pipeline_edits_inner(&ws, project.id.0.clone(), edited).await.unwrap();

        // the prompt file on disk was overwritten
        let body = std::fs::read_to_string(root.join("prompts/research.md")).unwrap();
        assert_eq!(body, "REWRITTEN body");
        // the pipeline still loads + validates (active pointer intact)
        let reloaded = pipeline::store::PipelineStore::new(root.to_string_lossy().into_owned()).load("demo").unwrap();
        assert_eq!(reloaded.id, "demo");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn save_pipeline_edits_rejects_an_invalid_draft_without_writing() {
        let root = std::env::temp_dir().join(format!("abp-save-bad-{}", uuid::Uuid::new_v4()));
        let ws = test_workspace().await;
        let project = create_project_from_draft_inner(&ws, "Demo".into(), root.to_string_lossy().into(), complete_draft("demo"))
            .await.unwrap();
        let before = std::fs::read_to_string(root.join("prompts/research.md")).unwrap();

        // an invalid edit: route to a non-existent node -> hard validate fails
        let mut bad = complete_draft("demo");
        bad.teams[0].prompt_body = "should NOT be written".into();
        bad.teams[0].outputs.on_approve = Some("ghost-node".into());
        let err = save_pipeline_edits_inner(&ws, project.id.0.clone(), bad).await.unwrap_err();
        assert!(!err.is_empty());
        // nothing was written
        assert_eq!(std::fs::read_to_string(root.join("prompts/research.md")).unwrap(), before);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn pipeline_to_draft_inner_reads_prompt_bodies_back() {
        let root = std::env::temp_dir().join(format!("abp-todraft-{}", uuid::Uuid::new_v4()));
        let ws = test_workspace().await;
        let project = create_project_from_draft_inner(&ws, "Demo".into(), root.to_string_lossy().into(), complete_draft("demo"))
            .await.unwrap();
        let loaded = pipeline::store::PipelineStore::new(root.to_string_lossy().into_owned()).load("demo").unwrap();

        let draft = pipeline_to_draft_inner(&ws, project.id.0.clone(), &loaded).await.unwrap();
        let research = draft.teams.iter().find(|t| t.id == "research").unwrap();
        assert_eq!(research.prompt_body, "You investigate the repo.");
        let _ = std::fs::remove_dir_all(&root);
    }
```

If the test module lacks a `test_workspace()` helper, check how existing `create_project_from_draft_inner` tests build their `WorkspaceState` (look near line 1450). Reuse that exact pattern; if it is inline, extract it into a local `async fn test_workspace() -> workspace::api::WorkspaceState` helper at the top of the test module (in-memory sqlite + `001_initial.sql` migration + `ProjectStore`, identical to `workspace/src/api.rs` tests' `state_with_project` minus the project insert). Also confirm `complete_draft(id: &str)` exists in this module (it is referenced by existing tests at lib.rs:1458/1472); reuse it.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src-tauri && cargo test -p app save_pipeline_edits 2>&1 | tail -25`
Expected: FAIL — `cannot find function save_pipeline_edits_inner` / `pipeline_to_draft_inner`.

- [ ] **Step 3: Implement the inner helpers + commands**

Add, immediately after `create_project_from_draft` (lib.rs:633), the following. Note `use std::collections::HashMap;` may already be imported at the top — if not, add it there rather than inline.

```rust
/// Read every team's prompt file body via Workspace's escape-guarded path
/// resolution, then convert the resolved Pipeline into an editable DraftPipeline
/// (A1; vet: Workspace owns the file read, Pipeline Authoring owns the convert).
/// Inner fn so it is testable without a Tauri State wrapper.
pub async fn pipeline_to_draft_inner(
    ws: &workspace::api::WorkspaceState,
    project_id: String,
    pipeline: &pipeline::model::Pipeline,
) -> Result<DraftPipeline, String> {
    let project = ws
        .store
        .get(&agent_bus_core::ProjectId(project_id))
        .await
        .map_err(|e| e.to_string())?;
    let root = project.root_path.to_string_lossy().into_owned();
    let mut bodies: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for team in &pipeline.teams {
        let full = workspace::api::resolve_under_root(&root, &team.prompt)?;
        // A missing prompt file is tolerated (empty body) — the editor seeds a
        // blank prompt the operator can fill, rather than failing to open.
        let body = std::fs::read_to_string(&full).unwrap_or_default();
        bodies.insert(team.id.clone(), body);
    }
    Ok(DraftPipeline::from_pipeline(pipeline, &bodies))
}

/// OHS: load the project's pipeline (resolved) into an editable DraftPipeline,
/// reading each team's prompt body back from disk. Seeds the in-app editor (A1).
#[tauri::command(rename_all = "snake_case")]
async fn pipeline_to_draft_cmd(
    state: tauri::State<'_, WorkspaceState>,
    project_id: String,
    project_root: String,
    id: String,
) -> Result<DraftPipeline, String> {
    let pipeline = pipeline::store::PipelineStore::new(project_root)
        .load(&id)
        .map_err(|e| e.to_string())?;
    pipeline_to_draft_inner(&state, project_id, &pipeline).await
}

/// Orchestrate save-pipeline-edits (A1; mirrors create_project_from_draft_inner
/// minus the row insert). HARD validate the draft's Pipeline; only on Ok overwrite
/// the project's existing YAML + prompt files (Workspace owns the write) and keep
/// the pipeline active. Nothing is written when invalid. Inner fn so it is
/// unit-testable without a Tauri State wrapper.
pub async fn save_pipeline_edits_inner(
    ws: &workspace::api::WorkspaceState,
    project_id: String,
    draft: DraftPipeline,
) -> Result<(), String> {
    // 1. HARD validate + serialize (the shared gate; nothing is written when
    //    invalid). vet F2: same core the create flow uses.
    let (yaml_rel, yaml, prompts) = pipeline::draft::prepare_pipeline_write(&draft)?;
    let pipeline_id = draft.to_pipeline().id;

    // 2. Overwrite the YAML + prompt files (Workspace owns bytes-to-disk; the
    //    project row already exists — do NOT insert, do NOT re-expand the root).
    workspace::api::write_project_pipeline_inner(
        ws, project_id.clone(), yaml_rel, yaml, prompts,
    )
    .await?;

    // 3. Keep the pipeline active (idempotent — typically already active; correct
    //    if the active pointer was cleared).
    ws.store
        .set_active_pipeline(
            &agent_bus_core::ProjectId(project_id),
            Some(&agent_bus_core::PipelineId(pipeline_id)),
            now_unix(),
        )
        .await
        .map_err(|e| e.to_string())
}

/// OHS command: hard-validate → overwrite the active pipeline's YAML + prompts.
#[tauri::command(rename_all = "snake_case")]
async fn save_pipeline_edits(
    state: tauri::State<'_, WorkspaceState>,
    project_id: String,
    draft: DraftPipeline,
) -> Result<(), String> {
    save_pipeline_edits_inner(&state, project_id, draft).await
}
```

Then register both commands in the `tauri::generate_handler!` list near line 980 (next to `create_project_from_draft`):

```rust
            create_project_from_draft,
            pipeline_to_draft_cmd,
            save_pipeline_edits,
```

- [ ] **Step 4: Refactor `create_project_from_draft_inner` onto the shared helper (vet F2)**

In `create_project_from_draft_inner` (lib.rs:596–612), replace the inline
validate+serialize block (step "1." through the `yaml_rel` line) so create and save
share one core. Replace:

```rust
    // 1. Serialize + HARD validate (gate before any write).
    let pipeline = draft.to_pipeline();
    pipeline::validate::validate(&pipeline).map_err(|e| e.to_string())?;
    let yaml = pipeline::draft::to_yaml(&pipeline).map_err(|e| e.to_string())?;
    let prompts = pipeline::draft::prompt_files(&draft);
    let yaml_rel = format!("pipelines/{}.yaml", pipeline.id);
```

with:

```rust
    // 1. HARD validate + serialize (shared gate; nothing is written when invalid).
    let (yaml_rel, yaml, prompts) = pipeline::draft::prepare_pipeline_write(&draft)?;
    let pipeline = draft.to_pipeline();
```

(`pipeline` is still needed below for `pipeline.id` at activation; keep that line.)

Run: `cd src-tauri && cargo test -p app create_project_from_draft 2>&1 | tail -15`
Expected: the existing create tests still PASS.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test -p app save_pipeline_edits 2>&1 | tail -25 && cargo test -p app pipeline_to_draft 2>&1 | tail -15`
Expected: PASS (3 new tests).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/app/src/lib.rs
git commit -m "feat(app): pipeline_to_draft_cmd + save_pipeline_edits; share validate-write core (A1)"
```

---

## Task 3: IPC wrappers (`pipelineToDraft`, `savePipelineEdits`)

**Files:**
- Modify: `src/ipc/pipeline.ts`
- Test: covered by the component tests (Task 5); no standalone test needed for thin wrappers.

- [ ] **Step 1: Add the wrappers**

Append to `src/ipc/pipeline.ts` (after `createProjectFromDraft`):

```typescript
/** Load a project's pipeline (resolved) into an editable DraftPipeline, reading
 *  each team's prompt body back from disk (A1 editor seed). */
export async function pipelineToDraft(
  projectId: string,
  projectRoot: string,
  id: string,
): Promise<DraftPipeline> {
  return await invoke<DraftPipeline>("pipeline_to_draft_cmd", {
    project_id: projectId,
    project_root: projectRoot,
    id,
  });
}

/** Hard-validate then overwrite the project's active pipeline YAML + prompt files
 *  (A1). Throws (no write) on a hard-validation failure. */
export async function savePipelineEdits(projectId: string, draft: DraftPipeline): Promise<void> {
  await invoke<void>("save_pipeline_edits", { project_id: projectId, draft });
}
```

- [ ] **Step 2: Typecheck**

Run: `bun run build 2>&1 | tail -15` (will also catch later tasks; it is fine if it fails until Task 6 — at minimum confirm no error originates in `ipc/pipeline.ts`). Alternatively `bunx tsc --noEmit 2>&1 | grep pipeline.ts || echo "ipc/pipeline.ts clean"`.
Expected: no type error in `ipc/pipeline.ts`.

- [ ] **Step 3: Commit**

```bash
git add src/ipc/pipeline.ts
git commit -m "feat(ipc): pipelineToDraft + savePipelineEdits wrappers for A1 editor"
```

---

## Task 4: `PipelineEditor` modal (reuse the wizard step components)

**Files:**
- Create: `src/wizard/PipelineEditor.tsx`
- Test: `src/wizard/PipelineEditor.test.tsx`

- [ ] **Step 1: Write the failing test**

Create `src/wizard/PipelineEditor.test.tsx`:

```tsx
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { PipelineEditor } from "./PipelineEditor";
import type { DraftPipeline } from "../ipc/pipeline";

// Mock the IPC layer (no Tauri runtime in vitest).
vi.mock("../ipc/pipeline", async () => {
  const actual = await vi.importActual<typeof import("../ipc/pipeline")>("../ipc/pipeline");
  return {
    ...actual,
    bestEffortValidate: vi.fn().mockResolvedValue([]),
    savePipelineEdits: vi.fn().mockResolvedValue(undefined),
  };
});

import { savePipelineEdits } from "../ipc/pipeline";

function seedDraft(): DraftPipeline {
  return {
    id: "demo",
    name: "Demo",
    description: "",
    schema_version: 2,
    teams: [
      {
        id: "research",
        name: "Research",
        prompt_body: "investigate",
        runner: { kind: "claude-cli", model: "claude-opus-4-8", effort: { mode: "standard" }, api_key_env: null },
        scope: { reads: [], writes: [], tools: [] },
        outputs: {},
        workers: { default: 1, max: 1 },
      },
    ],
    forks: [],
    joins: [],
    gates: [],
    escalations: [],
  };
}

describe("PipelineEditor", () => {
  beforeEach(() => vi.clearAllMocks());

  it("renders the seeded draft on the teams step", () => {
    render(<PipelineEditor projectId="p1" seed={seedDraft()} onClose={() => {}} onSaved={() => {}} />);
    expect(screen.getByLabelText("name for research")).toHaveValue("Research");
  });

  it("saves the edited draft via savePipelineEdits and calls onSaved", async () => {
    const onSaved = vi.fn();
    render(<PipelineEditor projectId="p1" seed={seedDraft()} onClose={() => {}} onSaved={onSaved} />);
    fireEvent.change(screen.getByLabelText("name for research"), { target: { value: "Researchers" } });
    fireEvent.click(screen.getByRole("button", { name: /save pipeline/i }));
    await waitFor(() => expect(savePipelineEdits).toHaveBeenCalledTimes(1));
    const [pid, draft] = (savePipelineEdits as unknown as ReturnType<typeof vi.fn>).mock.calls[0];
    expect(pid).toBe("p1");
    expect(draft.teams[0].name).toBe("Researchers");
    await waitFor(() => expect(onSaved).toHaveBeenCalled());
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `bun vitest run src/wizard/PipelineEditor.test.tsx 2>&1 | tail -20`
Expected: FAIL — cannot resolve `./PipelineEditor`.

- [ ] **Step 3: Implement `PipelineEditor`**

Create `src/wizard/PipelineEditor.tsx`:

```tsx
import type React from "react";
import { useState } from "react";
import { bestEffortValidate, savePipelineEdits, type DraftPipeline } from "../ipc/pipeline";
import { TeamsStep } from "./TeamsStep";
import { PromptsStep } from "./PromptsStep";
import { WiringStep } from "./WiringStep";

const EDIT_STEPS = ["teams", "prompts", "wiring"] as const;
type EditStep = (typeof EDIT_STEPS)[number];

interface PipelineEditorProps {
  projectId: string;
  seed: DraftPipeline;
  onClose: () => void;
  onSaved: () => void;
}

/// In-app pipeline editor (A1). REUSES the wizard's step editors over a local
/// draft seeded from the active pipeline (no Design Session chat — editing is
/// manual). Save hard-validates backend-side and overwrites the YAML + prompts.
export function PipelineEditor({ projectId, seed, onClose, onSaved }: PipelineEditorProps) {
  const [draft, setDraft] = useState<DraftPipeline>(seed);
  const [step, setStep] = useState<EditStep>("teams");
  const [issues, setIssues] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const idx = EDIT_STEPS.indexOf(step);

  // Manual edits re-fetch the backend's best-effort issues (the backend stays the
  // validation authority — same contract as ChatDraftPanel's manual-edit path).
  function onChange(d: DraftPipeline) {
    setDraft(d);
    bestEffortValidate(d).then(setIssues).catch(() => {});
  }

  async function save() {
    setBusy(true);
    setError(null);
    try {
      await savePipelineEdits(projectId, draft);
      onSaved();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div role="dialog" aria-modal="true" aria-label="Edit pipeline" style={overlay} onClick={onClose}>
      <div onClick={(e) => e.stopPropagation()} style={panel}>
        <div style={{ display: "flex", gap: 8, marginBottom: "var(--sp-4)" }}>
          {EDIT_STEPS.map((s) => (
            <button
              key={s}
              onClick={() => setStep(s)}
              style={{
                background: s === step ? "var(--surface-2)" : "transparent",
                border: "1px solid",
                borderColor: s === step ? "var(--border-2)" : "transparent",
                color: s === step ? "var(--text)" : "var(--text-3)",
                padding: "3px 12px",
                borderRadius: "var(--r-sm)",
                fontSize: 12,
                textTransform: "capitalize",
                cursor: "pointer",
              }}
            >
              {s}
            </button>
          ))}
        </div>

        {issues.length > 0 && (
          <div
            role="status"
            aria-label="validation issues"
            style={{ marginBottom: "var(--sp-3)", padding: "var(--sp-2)", border: "1px solid var(--accent-bd)", background: "var(--accent-2)", borderRadius: "var(--r-sm)", color: "var(--text-2)", fontSize: 11 }}
          >
            {issues.map((iss, i) => (
              <div key={i}>• {iss}</div>
            ))}
          </div>
        )}

        <div style={{ minHeight: 360, maxHeight: 460, overflowY: "auto" }}>
          {step === "teams" && <TeamsStep draft={draft} onChange={onChange} />}
          {step === "prompts" && <PromptsStep draft={draft} onChange={onChange} />}
          {step === "wiring" && <WiringStep draft={draft} onChange={onChange} />}
        </div>

        {error && <div style={{ color: "var(--danger)", fontSize: 12, marginTop: "var(--sp-2)" }}>{error}</div>}

        <div style={{ display: "flex", justifyContent: "space-between", marginTop: "var(--sp-5)" }}>
          <button onClick={onClose}>Cancel</button>
          <div style={{ display: "flex", gap: 8 }}>
            {idx > 0 && <button onClick={() => setStep(EDIT_STEPS[idx - 1])}>Back</button>}
            {idx < EDIT_STEPS.length - 1 && <button onClick={() => setStep(EDIT_STEPS[idx + 1])}>Next</button>}
            <button onClick={save} disabled={busy} aria-label="save pipeline">Save pipeline</button>
          </div>
        </div>
      </div>
    </div>
  );
}

const overlay: React.CSSProperties = { position: "fixed", inset: 0, background: "rgba(0,0,0,0.5)", display: "flex", alignItems: "center", justifyContent: "center", zIndex: 100 };
const panel: React.CSSProperties = { background: "var(--surface)", border: "1px solid var(--border)", borderRadius: "var(--r-md)", padding: "var(--sp-7)", minWidth: 720, maxWidth: 900 };
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `bun vitest run src/wizard/PipelineEditor.test.tsx 2>&1 | tail -20`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add src/wizard/PipelineEditor.tsx src/wizard/PipelineEditor.test.tsx
git commit -m "feat(wizard): PipelineEditor modal reusing the wizard step editors (A1)"
```

---

## Task 5: `PipelineView` "Edit pipeline" affordance

**Files:**
- Modify: `src/components/PipelineView.tsx`
- Test: `src/components/PipelineView.test.tsx` (create if absent)

- [ ] **Step 1: Write the failing test**

Create (or extend) `src/components/PipelineView.test.tsx`:

```tsx
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { PipelineView } from "./PipelineView";
import type { Pipeline } from "../ipc/pipeline";

function p(): Pipeline {
  return {
    id: "demo", name: "Demo", description: "", schema_version: 2, defaults: null,
    teams: [], gates: [], escalations: [], forks: [], joins: [],
  };
}

describe("PipelineView edit affordance", () => {
  it("renders an Edit pipeline button when onEdit is provided and calls it", () => {
    const onEdit = vi.fn();
    render(<PipelineView pipeline={p()} onEdit={onEdit} />);
    const btn = screen.getByRole("button", { name: /edit pipeline/i });
    fireEvent.click(btn);
    expect(onEdit).toHaveBeenCalledTimes(1);
  });

  it("shows no edit button without onEdit (pure viewer)", () => {
    render(<PipelineView pipeline={p()} />);
    expect(screen.queryByRole("button", { name: /edit pipeline/i })).toBeNull();
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `bun vitest run src/components/PipelineView.test.tsx 2>&1 | tail -20`
Expected: FAIL — no "edit pipeline" button (and `onEdit` not a prop).

- [ ] **Step 3: Implement the affordance**

In `src/components/PipelineView.tsx`, change the props interface and the header. Replace:

```tsx
export interface PipelineViewProps {
  pipeline: Pipeline | null;
}

export function PipelineView({ pipeline }: PipelineViewProps) {
```

with:

```tsx
export interface PipelineViewProps {
  pipeline: Pipeline | null;
  /// When provided, an "Edit pipeline" affordance opens the in-app editor (A1).
  /// Omitted = pure read-only viewer (e.g. the wizard review step).
  onEdit?: () => void;
}

export function PipelineView({ pipeline, onEdit }: PipelineViewProps) {
```

Then replace the header block (the `<h1>` through the schema caption `<div style={meta}>…read-only viewer (editing arrives in v1.1)…</div>`) with:

```tsx
      <div style={{ display: "flex", alignItems: "flex-start", justifyContent: "space-between", gap: "var(--sp-4)" }}>
        <div>
          <h1 style={{ fontSize: 16, color: "var(--text)", margin: 0 }}>{pipeline.name}</h1>
          {pipeline.description && (
            <p style={{ ...meta, marginTop: "var(--sp-1)" }}>{pipeline.description}</p>
          )}
          <div style={meta}>schema v{pipeline.schema_version}</div>
        </div>
        {onEdit && (
          <button
            onClick={onEdit}
            aria-label="edit pipeline"
            style={{ flexShrink: 0, background: "var(--surface-2)", border: "1px solid var(--border-2)", color: "var(--text)", padding: "4px 14px", borderRadius: "var(--r-sm)", fontSize: 12, cursor: "pointer" }}
          >
            Edit pipeline
          </button>
        )}
      </div>
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `bun vitest run src/components/PipelineView.test.tsx 2>&1 | tail -20`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add src/components/PipelineView.tsx src/components/PipelineView.test.tsx
git commit -m "feat(view): Edit pipeline affordance in PipelineView (A1)"
```

---

## Task 6: Wire the editor into `App.tsx`

**Files:**
- Modify: `src/App.tsx`

- [ ] **Step 1: Add editor state + seed loader + render**

In `App.tsx`:

1. Add the import (next to the other wizard import):

```tsx
import { PipelineEditor } from "./wizard/PipelineEditor";
```

2. Add to the `pipelineToDraft` import from ipc/pipeline (extend the existing `import { listPipelines, loadPipeline, type Pipeline } from "./ipc/pipeline";`):

```tsx
import { listPipelines, loadPipeline, pipelineToDraft, type DraftPipeline, type Pipeline } from "./ipc/pipeline";
```

3. Add editor state below the `const [pipeline, setPipeline] = useState<Pipeline | null>(null);` line:

```tsx
  // A1: the seeded draft for the in-app pipeline editor (null = closed).
  const [editorSeed, setEditorSeed] = useState<DraftPipeline | null>(null);
```

4. Extract the pipeline-load effect's body into a reusable callback so save can reload. Replace the existing `useEffect(() => { ... }, [activeProject]);` block (the one that calls `listPipelines`/`loadPipeline`) with:

```tsx
  const reloadPipeline = useCallback(async () => {
    if (!activeProject) {
      setPipeline(null);
      return;
    }
    const root = activeProject.root_path;
    try {
      const ids = await listPipelines(root);
      const target = ids[0] ?? null;
      const loaded = target ? await loadPipeline(root, target) : null;
      setPipeline(loaded);
    } catch {
      setPipeline(null);
    }
  }, [activeProject]);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      if (!cancelled) await reloadPipeline();
    })();
    return () => {
      cancelled = true;
    };
  }, [reloadPipeline]);
```

5. Add an open-editor handler (near the other handlers, e.g. after `onCreated`):

```tsx
  async function openEditor() {
    if (!activeProject || !pipeline) return;
    try {
      const seed = await pipelineToDraft(activeProject.id, activeProject.root_path, pipeline.id);
      setEditorSeed(seed);
    } catch {
      // opening the editor failed (e.g. pipeline missing on disk); stay on the viewer
    }
  }
```

6. Pass `onEdit` to the pipeline view. Change:

```tsx
        {view === "pipeline" ? (
          <PipelineView pipeline={pipeline} />
```

to:

```tsx
        {view === "pipeline" ? (
          <PipelineView pipeline={pipeline} onEdit={activeProject && pipeline ? openEditor : undefined} />
```

7. Render the editor modal (add just before `<NewProjectWizard ... />` near the end of the JSX):

```tsx
      {editorSeed && activeProject && (
        <PipelineEditor
          projectId={activeProject.id}
          seed={editorSeed}
          onClose={() => setEditorSeed(null)}
          onSaved={() => {
            setEditorSeed(null);
            reloadPipeline();
          }}
        />
      )}
```

- [ ] **Step 2: Typecheck + build**

Run: `bun run build 2>&1 | tail -25`
Expected: build succeeds (no TS errors).

- [ ] **Step 3: Run the full vitest suite**

Run: `bun vitest run 2>&1 | tail -25`
Expected: all green (existing wizard/view tests still pass; new editor/view tests pass).

- [ ] **Step 4: Commit**

```bash
git add src/App.tsx
git commit -m "feat(app): wire the in-app pipeline editor + reload-on-save (A1)"
```

---

## Task 7: Full verification

- [ ] **Step 1: Rust**

Run: `cd src-tauri && cargo test --workspace 2>&1 | tail -30`
Expected: all pass.

- [ ] **Step 2: Check + clippy**

Run: `cd src-tauri && cargo check --workspace 2>&1 | tail -10 && cargo clippy --workspace 2>&1 | tail -20`
Expected: clean (no warnings from clippy on the new code).

- [ ] **Step 3: Frontend**

Run: `bun vitest run 2>&1 | tail -15 && bun run build 2>&1 | tail -10`
Expected: vitest all green; build succeeds.

- [ ] **Step 4: No commit (verification only).**

---

## Self-Review notes

- **Spec coverage:** from_pipeline converter (Task 1), reads prompt bodies back via read_artifact at the root (Task 2 `pipeline_to_draft_inner`), Edit affordance in the viewer (Task 5), reuse the wizard step editors not a parallel editor (Task 4), save_pipeline_edits hard-validates + overwrites via write_project_pipeline + reactivate/reload (Tasks 2 & 6), DraftPipeline≠Pipeline preserved (converter returns a DraftPipeline; create flow untouched), P2/P3 join options round-trip (Task 1 `from_pipeline_carries_…` asserts `quorum`/`cancel_on_reject`).
- **Type consistency:** `from_pipeline(&Pipeline, &HashMap<String,String>)`, `pipeline_to_draft_inner`/`pipeline_to_draft_cmd`, `save_pipeline_edits_inner`/`save_pipeline_edits`, IPC `pipelineToDraft`/`savePipelineEdits`, `PipelineEditor` props `{projectId, seed, onClose, onSaved}`, `PipelineView` prop `onEdit?` — all consistent across tasks.
- **Idempotency / safety:** save overwrites the same `pipelines/<id>.yaml` + `prompts/<id>.md` paths with `std::fs::write` (truncating, deterministic); re-activation is idempotent. Orphan prompt files from removed teams are left on disk (harmless; out of scope — note only).
