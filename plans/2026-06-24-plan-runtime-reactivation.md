# Runtime Re-Activation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make runtime activation (the active pipeline + project bits, and the per-team worker loops) a runtime operation that can happen on project create/switch, not only at app boot.

**Architecture:** Split `RuntimeState` so the *activatable* bits live behind an `ArcSwap<ActivePipeline>` (lock-free atomic read for the hot `inject`/gate paths; rare swap on activate). A composition-root `PipelineActivator` holds the worker-loop collaborators built once at boot and exposes `activate(project_id)`, which loads the project + its active pipeline, swaps the runtime's active state, bumps a shared `Arc<AtomicU64>` generation, and spawns one worker loop per team of the new pipeline at the new generation. Old-generation loops self-retire by checking `my_gen != current_gen` each iteration. Boot uses the SAME `activate` path.

**Tech Stack:** Rust (Cargo workspace under `src-tauri/`), Tauri, sqlx/SQLite, tokio, arc-swap; TypeScript/React frontend (vitest, bun).

---

## Decisions

- **ArcSwap vs RwLock — chose `arc-swap::ArcSwap<Arc<ActivePipeline>>`.** Reads (`inject_topic_inner`, gate verdicts, `scale_team`) are hot and frequent; activation is rare. `ArcSwap` gives lock-free, wait-free reads (`load_full()` returns an `Arc<ActivePipeline>` snapshot) with no reader/writer contention and no risk of a reader holding a lock across an `.await`. A `RwLock` would force every command to decide guard lifetime around awaits and risks writer starvation under steady inject load. The atomic-pointer swap is exactly "which pipeline is active is now mutable" — minimal and on-language.
- **Add `arc-swap` dependency** to the `runtime` crate (workspace dep). Single, widely-used, no-std-friendly crate; standard for read-mostly atomic config/pointer swaps.
- **Two aggregates preserved.** `ActivePipeline` is a value-snapshot of *resolved* activation inputs (pipeline `Arc<Pipeline>`, project id/root/target_repo strings). It is NOT a new aggregate root and holds no stores. `RuntimeState` still owns `tasks` (Task aggregate) + `brake` + the swap. The WorkerPool aggregate is unchanged — `PoolContext` is still built from resolved values. No god-aggregate, no cross-root transaction.
- **Generation guard design.** `Arc<AtomicU64>` shared between the `PipelineActivator` and every spawned loop. `activate()` does `let my_gen = gen.fetch_add(1, SeqCst) + 1;` then spawns N loops capturing `my_gen`. Each loop, at the TOP of every iteration, reads `gen.load(SeqCst)`; if `!= my_gen` it `break`s (self-retires). This is the recommended approach over forcibly aborting tokio tasks — loops exit cleanly on their next poll (≤ the 500ms cadence). Race-safety: the swap of `ActivePipeline` happens-before the generation bump within `activate`; a loop that observes the new generation observes (via the captured snapshot) the new pipeline. Loops capture their pipeline by value (clone) at spawn, so a retiring old loop never reads the new pipeline. Idempotent under rapid re-activation: each `activate` bumps the generation, so only the latest generation's loops survive; earlier ones retire on next poll. The brief overlap (old loop finishing an in-flight claim) is safe — claims are atomic per the existing `claim_next_for_stage`.
- **PipelineActivator lives in `app`** (the composition root) — it is the only place that imports AppHandle, the runner factory, keychain, usage_sink, the log-sink factory, the revision reader, etc. It does NOT introduce a new cross-context edge: it loads the project (Workspace store) + pipeline (PipelineStore) and hands `runtime` only resolved paths/strings, exactly as boot does today. `runtime` gains the activatable-state shape (`ActivePipeline` + accessors) but does not learn about Workspace/Project types.

---

## File Structure

- `src-tauri/runtime/Cargo.toml` — add `arc-swap` dep.
- `src-tauri/Cargo.toml` — add `arc-swap` to `[workspace.dependencies]`.
- `src-tauri/runtime/src/api.rs` — `ActivePipeline` struct + refactor `RuntimeState` to hold `ArcSwap<Arc<ActivePipeline>>`; accessor `RuntimeState::active()`; rewrite `inject_topic_inner`/gate/`scale_team` to read the snapshot; an `activate_into(ActivePipeline)` swap method; update tests.
- `src-tauri/app/src/pipeline_activator.rs` — NEW: `PipelineActivator` holding boot-built collaborators + `Arc<AtomicU64>` generation + `Arc<RuntimeState>` + project/pipeline stores; `activate(project_id)`; the per-team loop spawn with the generation guard. The runner factory (`runner_for`) + loop body move here.
- `src-tauri/app/src/lib.rs` — `.setup()` builds the `PipelineActivator`, manages it in Tauri state, and calls `manager.activate(boot_project_id)` (replacing the inline `spawn_worker_loops`); `create_project_from_draft` calls `manager.activate` after create; NEW `activate_project` command; registered in `invoke_handler`.
- `src/ipc/workspace.ts` — NEW `activateProject(projectId)` IPC wrapper.
- `src/App.tsx` — call `activateProject` in `onCreated`.
- `src/wizard/NewProjectWizard.test.tsx` / new `src/ipc/workspace.test.ts` — frontend test for the activate IPC call.

---

## Task 1: Add arc-swap dependency

**Files:**
- Modify: `src-tauri/Cargo.toml` (workspace deps)
- Modify: `src-tauri/runtime/Cargo.toml`

- [ ] **Step 1: Add to workspace deps**

In `src-tauri/Cargo.toml` under `[workspace.dependencies]` add:
```toml
arc-swap = "1"
```

- [ ] **Step 2: Add to runtime crate**

In `src-tauri/runtime/Cargo.toml` under `[dependencies]` add:
```toml
arc-swap = { workspace = true }
```

- [ ] **Step 3: Verify it resolves**

Run: `cd src-tauri && cargo fetch && cargo check -p runtime`
Expected: compiles (arc-swap downloaded, no code uses it yet).

- [ ] **Step 4: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/runtime/Cargo.toml src-tauri/Cargo.lock
git commit -m "build(runtime): add arc-swap dependency for interior-mutable active state"
```

---

## Task 2: Introduce `ActivePipeline` + interior-mutable `RuntimeState`

**Files:**
- Modify: `src-tauri/runtime/src/api.rs`

This is the core refactor. `RuntimeState` keeps `tasks` + `brake` (unchanged), and replaces the four flat activatable fields with one `ArcSwap<Arc<ActivePipeline>>`.

- [ ] **Step 1: Write failing tests for the new shape + activate swap**

Replace the existing `state_with_project_target_repo` helper and add a swap test. In `src-tauri/runtime/src/api.rs` tests module, change the helper to build via the new API and add:

```rust
#[tokio::test]
async fn activate_swaps_active_pipeline_and_inject_reads_new() {
    let state = state_with_project_target_repo(Some("/proj-repo")).await;
    // build a second pipeline with a DIFFERENT entry team + project id
    let p2 = Pipeline {
        id: "p2".into(), name: "P2".into(), description: String::new(), schema_version: 1,
        defaults: None,
        teams: vec![pipeline::model::Team {
            id: "entry2".into(), name: "E2".into(), prompt: "e2.md".into(),
            scope: Default::default(), runner: None,
            outputs: Default::default(), workers: Default::default(),
        }],
        gates: vec![], escalations: vec![], forks: vec![], joins: vec![],
    };
    state.activate_into(ActivePipeline {
        pipeline: Arc::new(p2),
        project_id: "proj".into(),
        project_root: "/p2".into(),
        project_target_repo: Some("/p2-repo".into()),
    });
    let task = inject_topic_inner(&state, "topic".into(), None).await.unwrap();
    // entry stage now the NEW pipeline's first team + the NEW target_repo default
    assert_eq!(task.current_stage, "entry2");
    assert_eq!(task.pipeline, "p2");
    assert_eq!(task.target_repo, Some("/p2-repo".to_string()));
}
```

Update `state_with_project_target_repo` to construct through the new constructor:

```rust
async fn state_with_project_target_repo(project_default: Option<&str>) -> RuntimeState {
    use sqlx::sqlite::SqlitePoolOptions;
    let pool = SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
    sqlx::query(include_str!("../../app/migrations/001_initial.sql")).execute(&pool).await.unwrap();
    sqlx::query(include_str!("../../app/migrations/003_runtime.sql")).execute(&pool).await.unwrap();
    sqlx::query(include_str!("../../app/migrations/006_fanout.sql")).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO projects (id,name,root_path,created_at,updated_at) VALUES ('proj','n','/p',0,0)")
        .execute(&pool).await.unwrap();
    let pipeline = Pipeline {
        id: "p".into(), name: "P".into(), description: String::new(), schema_version: 1,
        defaults: None,
        teams: vec![pipeline::model::Team {
            id: "t1".into(), name: "T1".into(), prompt: "t1.md".into(),
            scope: Default::default(), runner: None,
            outputs: Default::default(), workers: Default::default(),
        }],
        gates: vec![], escalations: vec![], forks: vec![], joins: vec![],
    };
    RuntimeState::new(
        Arc::new(TaskStore::new(pool)),
        Arc::new(Brake::new()),
        ActivePipeline {
            pipeline: Arc::new(pipeline),
            project_id: "proj".into(),
            project_root: "/p".into(),
            project_target_repo: project_default.map(String::from),
        },
    )
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test -p runtime api::tests 2>&1 | tail`
Expected: compile error — `RuntimeState::new`, `ActivePipeline`, `activate_into` not defined.

- [ ] **Step 3: Implement `ActivePipeline` + refactor `RuntimeState`**

Replace the `RuntimeState` struct (lines ~16-31) and add `ActivePipeline`. New imports at top: `use arc_swap::ArcSwap;`.

```rust
/// The activatable slice of runtime state — a value snapshot of the resolved
/// activation inputs. Swapped atomically on project create/switch (and at boot).
/// NOT an aggregate root: holds no stores, only resolved values handed in at the
/// composition root (Runtime never learns the Project/Pipeline-on-disk types).
#[derive(Clone)]
pub struct ActivePipeline {
    /// The active pipeline, kept in memory for routing.
    pub pipeline: Arc<Pipeline>,
    /// The active project id (one project active at a time in v1).
    pub project_id: String,
    /// The active project root path (for scope/worktree resolution).
    pub project_root: String,
    /// Project-level `${target_repo}` default (A5). Resolved string handed in at
    /// the composition root.
    pub project_target_repo: Option<String>,
}

/// Shared Runtime state held by Tauri's state manager. `tasks` + `brake` are the
/// stable Task-aggregate collaborators. WHICH pipeline/project is active is
/// interior-mutable (vet: this makes activation a runtime op, not boot-only — it
/// does NOT merge the Task and WorkerPool aggregates). Reads are lock-free
/// (`ArcSwap::load_full`); activation swaps a fresh `Arc<ActivePipeline>`.
pub struct RuntimeState {
    pub tasks: Arc<TaskStore>,
    pub brake: Arc<Brake>,
    active: ArcSwap<ActivePipeline>,
}

impl RuntimeState {
    pub fn new(tasks: Arc<TaskStore>, brake: Arc<Brake>, active: ActivePipeline) -> Self {
        Self { tasks, brake, active: ArcSwap::from_pointee(active) }
    }

    /// Lock-free snapshot of the current active pipeline/project. Each read gets a
    /// consistent `Arc<ActivePipeline>` — a concurrent activate never tears it.
    pub fn active(&self) -> Arc<ActivePipeline> {
        self.active.load_full()
    }

    /// Atomically swap the active pipeline/project (project create/switch + boot).
    pub fn activate_into(&self, next: ActivePipeline) {
        self.active.store(Arc::new(next));
    }
}
```

- [ ] **Step 4: Rewrite the read sites to use the snapshot**

`inject_topic_inner` (read a snapshot once at the top):
```rust
pub async fn inject_topic_inner(
    state: &RuntimeState,
    topic: String,
    target_repo: Option<String>,
) -> Result<Task, String> {
    let active = state.active();
    let stage = entry_stage(&active.pipeline)?;
    let effective = crate::pool::effective_target_repo(
        target_repo.as_deref(),
        active.project_target_repo.as_deref().map(std::path::Path::new),
    )
    .map(|p| p.to_string_lossy().into_owned());
    let task = Task::injected(
        active.project_id.clone(),
        active.pipeline.id.clone(),
        stage,
        topic,
        effective,
        now_unix(),
    );
    state.tasks.insert(&task).await.map_err(|e| e.to_string())?;
    Ok(task)
}
```

`apply_gate_verdict_inner` — read `let active = state.active();` at the top and replace every `state.pipeline` with `active.pipeline`.

`scale_team_inner`:
```rust
pub fn scale_team_inner(state: &RuntimeState, team_id: String) -> Result<u32, String> {
    state
        .active()
        .pipeline
        .teams
        .iter()
        .find(|t| t.id == team_id)
        .map(|t| t.workers.max)
        .ok_or_else(|| format!("unknown team: {team_id}"))
}
```

- [ ] **Step 5: Run runtime tests**

Run: `cd src-tauri && cargo test -p runtime 2>&1 | tail -20`
Expected: PASS (all prior runtime tests + the new `activate_swaps_active_pipeline_and_inject_reads_new`).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/runtime/src/api.rs
git commit -m "feat(runtime): interior-mutable active pipeline via ArcSwap<ActivePipeline>"
```

---

## Task 3: PipelineActivator with generation-guarded loops

**Files:**
- Create: `src-tauri/app/src/pipeline_activator.rs`
- Modify: `src-tauri/app/src/lib.rs` (add `mod pipeline_activator;`, move `runner_for`)

The manager holds every loop collaborator built once at boot. `activate(project_id)` loads the project + active pipeline, swaps runtime state, bumps the generation, and spawns guarded loops.

- [ ] **Step 1: Write the generation-guard unit test**

Create `src-tauri/app/src/pipeline_activator.rs` with the guard logic and a test that does not need a real AppHandle. The loop body is extracted into a free async fn `run_team_loop` that takes a generation + a `should_continue` closure so it is testable. Test:

```rust
#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    // The pure generation predicate the loop checks each iteration.
    fn still_current(my_gen: u64, shared: &AtomicU64) -> bool {
        shared.load(Ordering::SeqCst) == my_gen
    }

    #[test]
    fn loop_retires_when_generation_bumps() {
        let gen = Arc::new(AtomicU64::new(1));
        assert!(still_current(1, &gen));      // current loop keeps going
        gen.fetch_add(1, Ordering::SeqCst);   // activate() bumped to 2
        assert!(!still_current(1, &gen));     // old gen-1 loop retires
        assert!(still_current(2, &gen));      // new gen-2 loop runs
    }

    #[test]
    fn activate_bumps_generation_each_call() {
        let gen = Arc::new(AtomicU64::new(0));
        let g1 = gen.fetch_add(1, Ordering::SeqCst) + 1;
        let g2 = gen.fetch_add(1, Ordering::SeqCst) + 1;
        assert_eq!((g1, g2), (1, 2));
        assert_eq!(gen.load(Ordering::SeqCst), 2);
    }
}
```

- [ ] **Step 2: Run to verify it fails (module not wired)**

Run: `cd src-tauri && cargo test -p app pipeline_activator 2>&1 | tail`
Expected: FAIL — `pipeline_activator` module not found (until added to lib.rs).

- [ ] **Step 3: Add the module declaration + move `runner_for`**

In `src-tauri/app/src/lib.rs` near the top (after imports) add:
```rust
mod pipeline_activator;
```
Move the `runner_for` fn and its `runner_factory_tests` module from `lib.rs` into `pipeline_activator.rs` (make `runner_for` `pub(crate)`), keeping its body byte-for-byte.

- [ ] **Step 4: Implement `PipelineActivator` + `run_team_loop`**

In `pipeline_activator.rs`, write the struct and methods. It holds the boot-built collaborators and a `Arc<RuntimeState>`, `Arc<AtomicU64>` generation, the `ProjectStore`, and a flag for whether log/audit/usage are wired (production passes Some). `activate` loads project + pipeline, computes `ActivePipeline`, swaps it into runtime, bumps the generation, and spawns one loop per team. Full code:

```rust
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tauri::{AppHandle, Emitter};

use pipeline::model::{Pipeline, Team};
use runners::claude_cli::ClaudeCliRunner;
use runners::output::Runner;
use runtime::api::{ActivePipeline, RuntimeState};
use runtime::brake::Brake;
use runtime::pool::{process_one_claim, ClaimOutcome, LogSinkFactory, PoolContext};
use runtime::task_store::TaskStore;
use workspace::store::ProjectStore;

/// Collaborators a worker loop needs that are built ONCE at boot and reused
/// across activations. Cheap to clone (Arcs / Options of Arcs).
#[derive(Clone)]
pub struct WorkerDeps {
    pub usage_sink: Option<Arc<dyn agent_bus_core::UsageSink>>,
    pub revision_reader: Option<Arc<dyn runtime::revision::RevisionBundleReader>>,
    pub pool: sqlx::SqlitePool,
    pub log_sink: Option<Arc<LogSinkFactory>>,
    pub audit: Option<Arc<runtime::invocation_audit::InvocationAuditStore>>,
    pub keychain: Option<Arc<dyn secrets::KeychainStore>>,
}

/// Owns the runtime-activation lifecycle: swap the active pipeline + (re)spawn
/// the per-team worker loops at a fresh generation. Built once at boot, held in
/// Tauri state, and invoked at boot AND on project create/switch (ONE path).
///
/// vet: this is a composition-root concern (it alone imports AppHandle + the
/// runner factory + Workspace/Pipeline stores). It hands Runtime only resolved
/// values via `ActivePipeline` — no new cross-context edge, two aggregates intact.
pub struct PipelineActivator {
    handle: AppHandle,
    runtime: Arc<RuntimeState>,
    project_store: Arc<ProjectStore>,
    tasks: Arc<TaskStore>,
    brake: Arc<Brake>,
    deps: WorkerDeps,
    /// Shared generation. `activate` bumps it; loops retire when it changes.
    generation: Arc<AtomicU64>,
}

impl PipelineActivator {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        handle: AppHandle,
        runtime: Arc<RuntimeState>,
        project_store: Arc<ProjectStore>,
        tasks: Arc<TaskStore>,
        brake: Arc<Brake>,
        deps: WorkerDeps,
    ) -> Self {
        Self { handle, runtime, project_store, tasks, brake, deps, generation: Arc::new(AtomicU64::new(0)) }
    }

    /// Load `project_id` + its active pipeline (resolved at the root), swap the
    /// runtime's active state to it, then bump the generation and spawn one guarded
    /// worker loop per team of the NEW pipeline. Old-generation loops retire on
    /// their next poll. Idempotent under rapid re-activation (only the latest
    /// generation survives). An empty/absent project is a no-op activation that
    /// still swaps an empty ActivePipeline + retires old loops (so a stale project's
    /// loops stop even when switching to "nothing").
    pub async fn activate(&self, project_id: &str) -> Result<(), String> {
        let (active, teams) = self.resolve(project_id).await?;
        // Swap FIRST so any reader (inject) that observes the new generation also
        // observes the new active state.
        self.runtime.activate_into(active.clone());
        let my_gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        for team in teams {
            self.spawn_team_loop(active.clone(), team, my_gen);
        }
        Ok(())
    }

    /// Resolve a project id into a swappable ActivePipeline + the team list to
    /// spawn loops for. Reuses the boot resolution rules (newest pipeline file).
    async fn resolve(&self, project_id: &str) -> Result<(ActivePipeline, Vec<Team>), String> {
        let empty = Pipeline {
            id: String::new(), name: String::new(), description: String::new(),
            schema_version: pipeline::model::SCHEMA_VERSION,
            defaults: None, teams: vec![], gates: vec![], escalations: vec![],
            forks: vec![], joins: vec![],
        };
        if project_id.is_empty() {
            return Ok((ActivePipeline {
                pipeline: Arc::new(empty), project_id: String::new(),
                project_root: String::new(), project_target_repo: None,
            }, vec![]));
        }
        let project = self
            .project_store
            .get(&agent_bus_core::ProjectId(project_id.to_string()))
            .await
            .map_err(|e| e.to_string())?;
        let root = project.root_path.to_string_lossy().into_owned();
        let target_repo = project.target_repo.clone();
        let store = pipeline::store::PipelineStore::new(&root);
        let pipe = store
            .list_ids()
            .ok()
            .and_then(|ids| ids.into_iter().next())
            .and_then(|id| store.load(&id).ok())
            .unwrap_or(empty);
        let teams = pipe.teams.clone();
        Ok((ActivePipeline {
            pipeline: Arc::new(pipe),
            project_id: project.id.0,
            project_root: root,
            project_target_repo: target_repo,
        }, teams))
    }

    fn spawn_team_loop(&self, active: ActivePipeline, team: Team, my_gen: u64) {
        let pipeline = active.pipeline.clone();
        let project_root = active.project_root.clone();
        let project_target_repo: Option<std::path::PathBuf> =
            active.project_target_repo.as_deref().map(std::path::PathBuf::from);
        let fanout = Arc::new(runtime::fanout_store::FanOutStore::new(self.deps.pool.clone()));

        // Per-team runner selection (keychain-first, then api_key_env). On failure
        // keep the loop alive on claude-cli rather than panicking.
        let effective = team.effective_runner();
        let keychain = self.deps.keychain.clone();
        let resolve_key = |c: &pipeline::model::RunnerConfig| -> Option<String> {
            let account = c.api_key_env.as_deref().unwrap_or("anthropic-api");
            if let Some(kc) = keychain.as_ref() {
                if let Ok(k) = kc.get(secrets::api::SERVICE, account) {
                    if !k.is_empty() { return Some(k); }
                }
            }
            c.api_key_env.as_ref().and_then(|n| std::env::var(n).ok())
        };
        let runner: Arc<dyn Runner> = match crate::pipeline_activator::runner_for(&effective, &resolve_key) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("app: team `{}` runner selection failed ({e}); falling back to claude-cli", team.id);
                Arc::new(ClaudeCliRunner::new())
            }
        };

        let ctx = PoolContext {
            pipeline,
            runner,
            tasks: self.tasks.clone(),
            fanout,
            brake: self.brake.clone(),
            project_root: std::path::PathBuf::from(&project_root),
            project_target_repo,
            read_prompt: Arc::new({
                let root = project_root.clone();
                move |t: &Team| {
                    std::fs::read_to_string(std::path::Path::new(&root).join(&t.prompt)).unwrap_or_default()
                }
            }),
            usage_sink: self.deps.usage_sink.clone(),
            revision_reader: self.deps.revision_reader.clone(),
            log_sink: self.deps.log_sink.clone(),
            audit: self.deps.audit.clone(),
            sandbox: false,
        };
        let handle = self.handle.clone();
        let generation = self.generation.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                // GENERATION GUARD: retire when a newer activation has happened.
                if generation.load(Ordering::SeqCst) != my_gen { break; }
                match process_one_claim(&ctx, &team).await {
                    Ok(ClaimOutcome::Settled { task_id, .. }) => {
                        let _ = handle.emit("task.changed", task_id);
                        let _ = handle.emit("usage.changed", ());
                    }
                    Ok(ClaimOutcome::RateLimited { .. }) => {
                        ctx.brake.set_on("rate-limit");
                        let _ = handle.emit("task.changed", "rate-limited");
                        let _ = handle.emit("usage.changed", ());
                    }
                    _ => {}
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        });
    }
}

/// Composition-root factory: map a team's resolved RunnerConfig to a concrete
/// Runner. (Moved here from lib.rs.)
pub(crate) fn runner_for(
    config: &pipeline::model::RunnerConfig,
    resolve_key: &dyn Fn(&pipeline::model::RunnerConfig) -> Option<String>,
) -> Result<Arc<dyn Runner>, runners::output::RunnerError> {
    use agent_bus_core::RunnerKind;
    use runners::anthropic_api::AnthropicApiRunner;
    match config.kind {
        RunnerKind::ClaudeCli => Ok(Arc::new(ClaudeCliRunner::new())),
        RunnerKind::AnthropicApi => {
            let key = resolve_key(config).ok_or_else(|| {
                runners::output::RunnerError::Other(
                    "anthropic-api runner: no API key found in the keychain or `api_key_env`".into(),
                )
            })?;
            Ok(Arc::new(AnthropicApiRunner::new(key)))
        }
    }
}
```

Then append the `#[cfg(test)] mod tests` from Step 1 AND the moved `runner_factory_tests` module.

- [ ] **Step 5: Run pipeline_activator tests**

Run: `cd src-tauri && cargo test -p app pipeline_activator 2>&1 | tail -20`
Expected: PASS (`loop_retires_when_generation_bumps`, `activate_bumps_generation_each_call`, the moved runner_factory tests).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/app/src/pipeline_activator.rs src-tauri/app/src/lib.rs
git commit -m "feat(app): PipelineActivator with generation-guarded worker loops"
```

---

## Task 4: Boot uses the shared activate path

**Files:**
- Modify: `src-tauri/app/src/lib.rs` (`.setup()` + remove old `spawn_worker_loops`)

- [ ] **Step 1: Build RuntimeState via the new constructor + build the manager**

In `.setup()`, replace the RuntimeState construction (lines ~935-957). Build `ActivePipeline` from `load_active`, construct ONE `Arc<RuntimeState>` via `RuntimeState::new`, manage a thin handle the commands read. Since Tauri's `manage` needs the value (not Arc) but the dispatcher needs an Arc, manage the Arc and have commands read it — OR keep managing `RuntimeState` for the `tauri::State` commands and share an `Arc` for the manager/dispatcher. To avoid two diverging copies (the current bug-adjacent smell of building it twice), **manage `Arc<RuntimeState>`** and update the command signatures.

Replace construction with:
```rust
// Runtime state (Plan 3). ONE instance, shared by the Tauri commands, the
// terminal dispatcher, and the PipelineActivator (no divergent copies).
let (project_id, project_root, project_target_repo, pipe) = load_active(&project_store).await;
let tasks = Arc::new(TaskStore::new(pool.clone()));
let invocation_audit = Arc::new(runtime::invocation_audit::InvocationAuditStore::new(pool.clone()));
let brake = Arc::new(Brake::new());
let _ = tasks.release_orphaned_running(now_unix()).await;

let runtime_state_arc = Arc::new(RuntimeState::new(
    tasks.clone(),
    brake.clone(),
    runtime::api::ActivePipeline {
        pipeline: Arc::new(pipe),
        project_id: project_id.clone(),
        project_root: project_root.clone(),
        project_target_repo: project_target_repo.clone(),
    },
));
handle.manage(runtime_state_arc.clone());
```

- [ ] **Step 2: Update Tauri command signatures to `State<Arc<RuntimeState>>`**

In `src-tauri/runtime/src/api.rs`, change every `state: tauri::State<'_, RuntimeState>` to `state: tauri::State<'_, Arc<RuntimeState>>` and deref (`&state` already yields `&Arc<RuntimeState>`; calls like `inject_topic_inner(&state, ...)` need `&**state` or pass `state.as_ref()`). Update the bodies: e.g. `inject_topic_inner(state.as_ref(), topic, target_repo).await`. For the synchronous brake commands, `state.brake` works through the Arc Deref. Verify each command compiles.

- [ ] **Step 3: Build + manage the PipelineActivator and activate the boot project**

After the usage_sink + revision_reader are available (they are built ~line 970+), assemble `WorkerDeps` and the manager, manage it, then activate. Replace the old `if !pipe.teams.is_empty() { spawn_worker_loops(...) }` block (lines ~1055-1061) with:
```rust
let revision_reader: Option<Arc<dyn runtime::revision::RevisionBundleReader>> =
    Some(Arc::new(SqliteRevisionReader { pool: pool.clone() }));
let manager = Arc::new(pipeline_activator::PipelineActivator::new(
    handle.clone(),
    runtime_state_arc.clone(),
    project_store.clone(),
    tasks.clone(),
    brake.clone(),
    pipeline_activator::WorkerDeps {
        usage_sink: Some(usage_sink.clone()),
        revision_reader,
        pool: pool.clone(),
        log_sink: Some(make_task_log_sink(handle.clone())),
        audit: Some(invocation_audit.clone()),
        keychain: Some(keychain.clone()),
    },
));
handle.manage(manager.clone());
// Boot activation goes through the SAME path as runtime activation.
if let Err(e) = manager.activate(&project_id).await {
    eprintln!("app: boot activation failed: {e}");
}
```

- [ ] **Step 4: Delete the old `spawn_worker_loops` fn**

Remove the entire `spawn_worker_loops` fn (lines ~1144-1275, the body now lives in `PipelineActivator::spawn_team_loop`). Remove the now-unused `use runtime::pool::{process_one_claim, PoolContext};` from lib.rs if nothing else uses them (the AnthropicApiRunner import moved too — verify `cargo check` flags unused imports).

- [ ] **Step 5: Update the RootDispatcher field type**

`RootDispatcher.runtime` is already `Arc<RuntimeState>` (line 72) — it now receives `runtime_state_arc.clone()` (already does). Confirm no `.pipeline`/`.project_id` field access remains on `RuntimeState` anywhere in lib.rs (the agentic engine uses `project_id` the local var, not the state field — fine).

- [ ] **Step 6: Build the app crate**

Run: `cd src-tauri && cargo check -p app 2>&1 | tail -30`
Expected: compiles. Fix any borrow/Arc-deref issues surfaced.

- [ ] **Step 7: Run app tests**

Run: `cd src-tauri && cargo test -p app 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add src-tauri/app/src/lib.rs src-tauri/runtime/src/api.rs
git commit -m "refactor(app): boot activates via the shared PipelineActivator.activate path"
```

---

## Task 5: `create_project_from_draft` triggers activation + `activate_project` command

**Files:**
- Modify: `src-tauri/app/src/lib.rs`

- [ ] **Step 1: Add the `activate_project` command**

After `create_project_from_draft` add:
```rust
/// OHS command: activate a project's runtime (swap the active pipeline + respawn
/// worker loops at a fresh generation). Called by the frontend when a project is
/// created or selected. Idempotent: re-activating the same project just bumps the
/// generation (old loops retire, new ones take over).
#[tauri::command(rename_all = "snake_case")]
async fn activate_project(
    manager: tauri::State<'_, Arc<pipeline_activator::PipelineActivator>>,
    project_id: String,
) -> Result<(), String> {
    manager.activate(&project_id).await
}
```

- [ ] **Step 2: Activate inside `create_project_from_draft` (the command, not the inner)**

The `_inner` stays pure (Workspace-only, unit-tested without a manager). The *command* wrapper gains the manager and activates after create. Change the command:
```rust
#[tauri::command(rename_all = "snake_case")]
async fn create_project_from_draft(
    state: tauri::State<'_, WorkspaceState>,
    manager: tauri::State<'_, Arc<pipeline_activator::PipelineActivator>>,
    name: String,
    root: String,
    draft: DraftPipeline,
    target_repo: Option<String>,
) -> Result<Project, String> {
    let project = create_project_from_draft_inner(&state, name, root, draft, target_repo).await?;
    // Trigger runtime activation for the just-created project (the bug fix):
    // swap RuntimeState + spawn the new pipeline's worker loops.
    manager.activate(&project.id.0).await?;
    Ok(project)
}
```

- [ ] **Step 3: Register both in `invoke_handler`**

Add to the `tauri::generate_handler![...]` list:
```rust
            activate_project,
```
(`create_project_from_draft` is already listed.)

- [ ] **Step 4: Build**

Run: `cd src-tauri && cargo check -p app 2>&1 | tail -20`
Expected: compiles.

- [ ] **Step 5: Run the full workspace tests**

Run: `cd src-tauri && cargo test --workspace 2>&1 | grep -E "test result|error" | tail -30`
Expected: all green (≥ baseline 537 + new tests). The `create_project_from_draft_inner` tests are unchanged (they call the inner directly).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/app/src/lib.rs
git commit -m "feat(app): activate_project command + create-from-draft triggers activation"
```

---

## Task 6: Frontend triggers activation on create

**Files:**
- Modify: `src/ipc/workspace.ts`
- Create: `src/ipc/workspace.test.ts`
- Modify: `src/App.tsx`

- [ ] **Step 1: Write the failing IPC test**

Create `src/ipc/workspace.test.ts`:
```ts
import { describe, it, expect, vi, beforeEach } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invoke(...a) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));

beforeEach(() => invoke.mockReset());

describe("activateProject", () => {
  it("invokes activate_project with the project id (snake_case)", async () => {
    const { activateProject } = await import("./workspace");
    invoke.mockResolvedValue(undefined);
    await activateProject("proj-1");
    expect(invoke).toHaveBeenCalledWith("activate_project", { project_id: "proj-1" });
  });
});
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd /Users/tim/projects/agent-bus-app && /opt/homebrew/bin/bun vitest run src/ipc/workspace.test.ts 2>&1 | tail`
Expected: FAIL — `activateProject` is not exported.

- [ ] **Step 3: Add the IPC wrapper**

In `src/ipc/workspace.ts` add:
```ts
/** Activate a project's runtime: swap the active pipeline + (re)spawn its worker
 *  loops. Call after creating or selecting a project so /inject targets it. */
export async function activateProject(projectId: string): Promise<void> {
  await invoke<void>("activate_project", { project_id: projectId });
}
```

- [ ] **Step 4: Run the IPC test**

Run: `cd /Users/tim/projects/agent-bus-app && /opt/homebrew/bin/bun vitest run src/ipc/workspace.test.ts 2>&1 | tail`
Expected: PASS.

- [ ] **Step 5: Call activateProject in `onCreated`**

In `src/App.tsx`, import `activateProject` from `./ipc/workspace` (add to the existing import) and update `onCreated`:
```tsx
  const onCreated = useCallback(
    (p: Project) => {
      setWizardOpen(false);
      // Trigger runtime activation for the new project so /inject targets it
      // (the runtime re-activation fix). Reload regardless of activation result.
      void activateProject(p.id).finally(() => reload());
    },
    [reload],
  );
```

- [ ] **Step 6: Run the full vitest suite + typecheck**

Run: `cd /Users/tim/projects/agent-bus-app && /opt/homebrew/bin/bun vitest run 2>&1 | tail -15`
Expected: all green (existing + the new IPC test). `NewProjectWizard.test.tsx` still passes (its `onCreated` is a mock).

- [ ] **Step 7: Commit**

```bash
git add src/ipc/workspace.ts src/ipc/workspace.test.ts src/App.tsx
git commit -m "feat(frontend): activate project runtime after create (activateProject IPC)"
```

---

## Task 7: Full verification gates

- [ ] **Step 1: cargo test**

Run: `cd src-tauri && cargo test --workspace 2>&1 | grep -E "test result|error\[" | tail -30`
Expected: all `ok`, 0 failed.

- [ ] **Step 2: cargo check**

Run: `cd src-tauri && cargo check --workspace 2>&1 | tail -5`
Expected: clean.

- [ ] **Step 3: cargo clippy**

Run: `cd src-tauri && cargo clippy --workspace 2>&1 | tail -15`
Expected: no warnings (fix any clippy lint, e.g. needless clones).

- [ ] **Step 4: vitest**

Run: `cd /Users/tim/projects/agent-bus-app && /opt/homebrew/bin/bun vitest run 2>&1 | tail -10`
Expected: all green.

- [ ] **Step 5: bun build**

Run: `cd /Users/tim/projects/agent-bus-app && /opt/homebrew/bin/bun run build 2>&1 | tail -10`
Expected: build succeeds (tsc + vite).

- [ ] **Step 6: Merge**

```bash
git checkout main
git merge --no-ff plan-runtime-reactivation -m "merge: runtime re-activation fix"
git tag plan-runtime-reactivation
```
Do NOT push. Leave on `main`.
