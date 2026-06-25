# Spec — Runtime lifecycle: clean exit + clean resume

*Design doc. Brainstormed 2026-06-25. Promotes backlog **LF20** (exit/brake doesn't stop in-flight `claude` subprocesses) and the **resume gap** it exposes (store-occupancy leak on a crash/kill). Two halves of one thing: killing in-flight work on exit/Stop, and reconciling runtime state on reopen so a run picks up cleanly. Also folds in **LF26** (workers inherit the app's cwd, so artifacts scatter into the app source tree) — the fix lands on the same spawner seam this spec already reworks.*

## Why this exists

Today the app has no graceful shutdown: worker loops die with the process (fine — no new claims), but an in-flight `claude` invocation is spawned via **blocking `std::process::Command::output()`** with no kill handle / process group, so it **orphans** and keeps running after quit — potentially still writing the worktree (implementers mutate the target repo). The brake (Stop) only blocks new claims; it doesn't kill a running invocation. And on reopen, `release_orphaned_running` re-queues interrupted task rows, but **nothing rebuilds store occupancy** — a slot reserved before a kill (`occupancy += 1`) is never committed or released, so the store stays **falsely full → backpressure → the run stalls**. Killing on exit/Stop actually *creates* that leak, so the two must ship together.

The same `Command::output()` call (`runners/src/claude_cli.rs:59`) also sets **no `current_dir`**, so the worker inherits whatever directory the app launched from (`/src-tauri/app` in dev). Agents resolve relative artifact paths against that cwd, so artifacts scatter into the app's own source tree instead of the project, and `read_artifact` can't find them (**LF26**, observed live). Since this spec is already rebuilding that exact spawn site into a killable spawner, the cwd + artifact-base fix belongs here.

## Decisions (from the brainstorm)

1. **Killable spawns** via the existing `with_spawner` ACL seam: `.spawn()` the child **in its own process group**, track it in a shared registry, and kill the *group* (so `claude`'s own child tools/subagents die too). No process types cross the `Runner`/`ChatRunner` traits.
2. **Two kill triggers:** **app exit** (Tauri `RunEvent::ExitRequested`) AND **Stop** (the brake — `brake_on` + the auto-meter sweep that trips it). Stop now actually halts running work.
3. **Politeness:** **SIGTERM → ~2–3s grace → SIGKILL** the group.
4. **Clean resume:** on boot/activation, after `release_orphaned_running` (exists), **rebuild each store's occupancy from the resident work-items** (clears leaked reservations) so the resumed run isn't falsely backpressured.
5. **Platform:** Unix (macOS + Linux) via process groups + signals; Windows is a documented gap (job objects / `taskkill /T`) — dev + target is macOS.
6. **Worker cwd + artifact base (LF26):** the killable spawner sets the child's `current_dir` to the work-item's resolved working dir — the git worktree for implementers, the target repo for read-only/artifact stages — so the worker never inherits the app cwd. Inter-stage artifacts live in an **app-owned data dir** (`<app_data>/projects/<project_id>/artifacts/<stage>/…`), resolved to an **absolute** path before it reaches the agent (both the generator and transformer passes), added to the scope writes + `--add-dir`, and read back from the same base. Keep the project/target directory settable — the fix is to *use* it, not lock it down. The project-delete cascade removes the app-data artifacts dir (never touches `target_repo`).

## Architecture

### `ProcessRegistry` (composition-root)
A shared `Arc<ProcessRegistry>` holding the live child **process-group ids** (the spawner registers a pgid on spawn, removes it on completion). `#[cfg(unix)]` methods:
- `register(pgid)` / `deregister(pgid)`.
- `kill_all()` — `kill(-pgid, SIGTERM)` for every registered group; wait a short grace (~2–3s, polling for exit); `kill(-pgid, SIGKILL)` for any survivor. Idempotent + best-effort (a gone pid is fine). Uses `nix` (or `libc`) `kill`.
- On non-unix: a no-op with a logged warning (the Windows gap).

### Killable spawner (the seam)
The composition root builds the worker `ClaudeCliRunner::with_spawner(...)` AND the chat `ClaudeChatRunner::with_spawner(...)` with a spawner closure capturing `registry.clone()` that:
1. Builds the `Command`, sets `.process_group(0)` (Unix `CommandExt`) so the child leads a fresh group (pgid == child pid), **sets `.current_dir(<work-item working dir>)`** (LF26 — worktree for implementers, target repo otherwise; resolved from the work-item's `${target_repo}`/worktree, never inherited from the app), pipes stdout/stderr.
2. `.spawn()` → `Child`; `registry.register(child.id())`.
3. Reads stdout/stderr to completion and `.wait()`s (replicating `.output()` capture; the streaming runner keeps forwarding deltas as today).
4. `registry.deregister(pgid)`; returns the same `interpret_runner_output(stdout, stderr, success)` as the current closure.
`ClaudeCliRunner::new()`'s plain blocking `.output()` stays for tests/non-app use. The ACL is unchanged — only the root's spawner closure differs. The working dir is passed in alongside the existing invocation inputs (it is already resolvable per work-item — `engine.rs:42` `${target_repo}`), so the runner stays free of project/worktree knowledge.

### Artifact base (LF26)
Artifacts resolve to an **absolute, app-owned** path — `<app_data>/projects/<project_id>/artifacts/<stage>/<key>-v<attempt>.md` — not the relative `${project}/artifacts/…` shape that today gets resolved against the (wrong) cwd. `engine::artifact_path` (and the generator pass at `engine.rs:887`) compose against this absolute base; the value is substituted into the **output contract** the agent receives so the agent writes to the right place regardless of its cwd, and `read_artifact` reads from the same base. The base dir is added to the work-item's scope writes (`engine.rs:1215`) + the invocation's `--add-dir` so `claude` is permitted to write there even though it sits outside the worker's cwd. The **generator** pass must run through the same PathVars/scope substitution the transformer does (`engine.rs:1204`/`:1216`) — today it appears to emit an unresolved/relative path, which is why the live research candidates landed under the app cwd.

### Kill triggers
- **Exit:** `tauri::Builder…build()?.run(move |_app, event| { if matches!(event, RunEvent::ExitRequested { .. }) { registry.kill_all(); } })`.
- **Stop:** every brake-on site at the composition root (`brake_on` command + the auto-meter sweep that trips the brake) calls `registry.kill_all()` after setting the brake. (Route `brake_on` through a root wrapper, or give the brake an on-set hook — the plan picks the cleanest; the runtime `Brake` aggregate itself stays registry-unaware.)

### Boot reconciliation (resume)
At boot/activation, in order:
1. `TaskStore::release_orphaned_running` (exists) — `running` → `queued`.
2. **`StoreRepo::reconcile_occupancy(run_id)`** (new) — for each store of the run, set `occupancy` = the count of work-items resident at that stage (truth from `tasks`, using the engine's residency states — items parked awaiting a worker). This clears reservations leaked by a killed/crashed worker. Then the worker loops + the resumed run drive normally (the active run, generator-dry flag, and found-key ledger all persist).

## Components / files
- New: `ProcessRegistry` (app crate — composition root; or a small `runtime`/util module it can live in, but it's wired at the root). `StoreRepo::reconcile_occupancy`.
- Modified: `app/src/lib.rs` (build the killable spawners; the `RunEvent::ExitRequested` handler; brake-on → `kill_all`; call `reconcile_occupancy` in the boot/activation recovery path next to `release_orphaned_running`). `pipeline_activator.rs` if activation is where recovery runs.
- Modified (LF26): `runners/src/claude_cli.rs` spawner sets `current_dir`; `runtime/src/engine.rs` `artifact_path`/generator pass resolve the absolute app-data artifact base + scope-write/`--add-dir` it; the work-item working dir threads through to the spawner; the project-delete cascade removes `<app_data>/projects/<id>/artifacts/`. The app-data base path comes from the Tauri app-data dir (resolved at the composition root, passed into the engine context alongside `project_root`).
- Unchanged ACL: `runners`/`llm_chat` traits; `ClaudeCliRunner`/`ClaudeChatRunner` keep `with_spawner`.

## Testing (no live `claude`)
- **ProcessRegistry (unix):** spawn a real `sleep 30` in its own group (with a child process so the *group* matters), `register`, `kill_all`; assert the process **and its child** are dead; a SIGTERM-ignoring child still dies after the grace via SIGKILL.
- **Killable spawner:** a real `echo hello` (or `sh -c`) through the spawner returns the captured output and registers-then-deregisters (registry empty afterward).
- **`reconcile_occupancy`:** seed a store row with inflated `occupancy` + N resident task rows at that stage; reconcile; assert `occupancy == N`. Plus: a leaked reservation (occupancy=2, zero resident) reconciles to 0.
- **Wiring** (exit handler / brake → kill_all): composition-root, structural (the registry methods carry the tested behavior).
- **Worker cwd (LF26):** a spawner test asserts the child runs with the expected `current_dir` (e.g. a `sh -c pwd` through the spawner returns the passed working dir, not the test process's cwd).
- **Artifact base (LF26):** `artifact_path` (and the generator pass) compose an absolute path under the app-data base; a round-trip test writes via the resolved path and `read_artifact` reads it back; assert the generator pass produces the same absolute shape the transformer does (regression for the relative-path scatter).

## Out of scope / non-goals
- Windows kill (job objects / `taskkill /T`) — documented gap; Unix only now.
- Per-task selective kill (Stop kills ALL running, per the decision) — no per-card kill button in v1.
- Resuming a *partially-produced* artifact mid-invocation — a killed invocation's item is simply re-run from scratch (idempotent re-claim); no checkpoint/restore of in-flight model output.

## Relationship to other items
- Closes **LF20** + the resume-occupancy gap it exposed, and **LF26** (worker cwd / artifact scatter) — folded in because it lands on the same spawner seam.
- Builds on **R** (the engine's reserve/commit/take store semantics) and the existing `release_orphaned_running` (crash recovery, F4).
