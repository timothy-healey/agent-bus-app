# Runtime Lifecycle: Clean Exit + Clean Resume — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Kill in-flight `claude` subprocess groups on app exit and on Stop (brake), give workers a correct working directory + an app-owned absolute artifact base, and reconcile leaked store occupancy on boot so a resumed run is not falsely backpressured.

**Architecture:** A composition-root `ProcessRegistry` tracks live child process-group ids. The existing `with_spawner` seam on both `ClaudeCliRunner` and `ClaudeChatRunner` is given a production spawner that puts the child in its own process group, sets `current_dir`, registers/deregisters the pgid, and captures output exactly as today via `interpret_runner_output`. The Tauri `RunEvent::ExitRequested` handler and every brake-on site call `registry.kill_all()` (SIGTERM → grace → SIGKILL on the group). The engine resolves artifacts under an absolute `<app_data>/projects/<id>/artifacts/...` base threaded through `EngineContext`, and `StoreRepo::reconcile_occupancy` rebuilds occupancy from resident `tasks` rows at boot, wired right after `release_orphaned_running`.

**Tech Stack:** Rust (Cargo workspace under `src-tauri/`), Tauri 2, sqlx + SQLite, `libc` (already in the lockfile, v0.2.186) for unix signals, `std::os::unix::process::CommandExt` for `process_group`. macOS/Unix only; non-unix is a logged no-op.

---

## Background facts confirmed by reading the code

These are load-bearing; do not re-derive them, but do read the cited code before editing.

- **`SpawnFn` (worker)** — `runners/src/claude_cli.rs:15`:
  `pub type SpawnFn = Box<dyn Fn(&[String]) -> Result<String, RunnerError> + Send + Sync>;`
  Called at `claude_cli.rs:96` as `(self.spawn)(&argv)?` inside `run_once`, where `argv[0]` is the program and `argv[1..]` its args.
- **`SpawnFn` (chat)** — `llm_chat/src/claude_cli.rs:17`:
  `pub type SpawnFn = Box<dyn Fn(&[String]) -> Result<String, ChatError> + Send + Sync>;`
  Called at `claude_cli.rs:90` as `(self.spawn)(&args)?` inside `run_once`. Note the chat `.output()` spawner passes the WHOLE `args` slice to `Command::new(CLAUDE_BIN)` (no split-first), unlike the worker runner which splits `argv[0]` as the program.
- **`interpret_runner_output`** — `runners/src/claude_cli.rs:25`:
  `pub fn interpret_runner_output(stdout: Vec<u8>, stderr: Vec<u8>, success: bool) -> Result<String, RunnerError>`
- **`interpret_chat_output`** — `llm_chat/src/claude_cli.rs:28`:
  `pub fn interpret_chat_output(stdout: Vec<u8>, stderr: Vec<u8>, success: bool) -> Result<String, ChatError>`
- **`InvocationRequest`** — `runners/src/output.rs:114` — fields: `task_id, team_id, model, thinking_budget, system_prompt, user_message, settings_path, add_dirs: Vec<String>, sandbox_profile: Option<String>`. The SpawnFn does NOT receive this struct today — only `argv`. We add a working-dir channel (Task 3).
- **`EngineContext`** — `runtime/src/engine.rs:130`. Fields include `project_root: PathBuf`, `target_repo: Option<PathBuf>`. We ADD `artifact_base: PathBuf` (Task 8). Built in `pipeline_activator.rs` `ctx_builder` (`pipeline_activator.rs:325`, struct literal at `:348`).
- **`artifact_path` / `artifact_dir`** — `runtime/src/engine.rs:1382` / `:1389` — today PURE, returning `${project}/artifacts/<stage>/<key>-v<attempt>.md` (resolved later by the scope layer against `project_root`). `effective_target_repo` is at `engine.rs:46`.
- **`PathVars` / `resolve`** — `workspace/src/paths.rs:15` / `:57`. `resolve` recognises `${project}`, `${agent_bus}`, `${target_repo}`, `${task_id}`. `engine::invoke` builds `PathVars::new(&ctx.project_root)` at `engine.rs:1204` and pushes `artifact_dir(&team.id)` into `scope.writes` at `:1215` before `prepare(...)` at `:1216`.
- **`output_contract(role, artifact_dir, already_found)`** — `runners/src/stream_json.rs:242` — embeds `artifact_dir` literally into the agent's contract text. The generator composes it at `engine.rs:891`; the transformer at `:322`.
- **Residency / occupancy lifecycle** — `runtime/src/store.rs`: `reserve` (`:58`) is the only place occupancy goes up (block-before-claim/commit); `release` (`:73`) the only place it goes down. A committed/claimed item lives at `current_stage = <stage>` in states `queued | running | gated | revising | joining` while it occupies the slot; it releases on `done` (transform success at `engine.rs:401`, gate/fork/join settle) or on the operational-failure release (`engine.rs:335`). `needs_human` items are routed to the escalation stage and their slot is released. So **resident states = `queued, running, gated, revising, joining`**.
- **`release_orphaned_running`** — `task_store.rs:230`, called at `app/src/lib.rs:1288`. Flips all `running` → `queued`. Reconciliation must run AFTER this so the just-requeued items are counted as resident.
- **Boot wiring** — `app/src/lib.rs`: stores built at `:1293`; `tasks.release_orphaned_running` at `:1288`; brake `set_on` (frontend command) in `runtime/src/api.rs:685`; RootDispatcher `brake_on` at `app/src/lib.rs:1067`; auto-meter `BrakeDecision::SetOn` at `app/src/lib.rs:1473`; Tauri `.run(...)` at `app/src/lib.rs:1542`; `data_dir = handle.path().app_data_dir()` at `:1225`.
- **`libc`** is already resolved transitively (`Cargo.lock:2193`, v0.2.186). We add it as an explicit dep to the `app` crate only.

---

## File Structure

- **New:** `src-tauri/app/src/process_registry.rs` — the `ProcessRegistry` (composition-root concern; lives in the `app` crate so no ACL crate gains process types).
- **Modify:** `src-tauri/app/Cargo.toml` — add `libc` dep.
- **Modify:** `src-tauri/app/src/lib.rs` — `mod process_registry;`, build the registry, build killable spawners, exit handler, brake-on → `kill_all`, call `reconcile_occupancy` at boot.
- **Modify:** `src-tauri/runners/src/claude_cli.rs` — widen `SpawnFn` to carry an optional working dir; thread it from `run_once`.
- **Modify:** `src-tauri/runners/src/output.rs` — add `working_dir: Option<String>` to `InvocationRequest`.
- **Modify:** `src-tauri/llm_chat/src/claude_cli.rs` — widen chat `SpawnFn` to carry an optional working dir.
- **Modify:** `src-tauri/llm_chat/src/chat.rs` — add `working_dir: Option<String>` to `ChatRequest`.
- **Modify:** `src-tauri/runtime/src/engine.rs` — add `artifact_base` to `EngineContext`; absolute `artifact_path`/`artifact_dir` composition against it; thread the work-item working dir into `InvocationRequest.working_dir`.
- **Modify:** `src-tauri/runtime/src/store.rs` — `reconcile_occupancy`.
- **Modify:** `src-tauri/workspace/src/api.rs` — `read_artifact` reads from the absolute app-data base.
- **Modify:** `src-tauri/app/src/pipeline_activator.rs` — pass `artifact_base` into `EngineContext`; pass the registry-backed spawner into the runners.
- **Modify:** `src-tauri/app/src/lib.rs` (delete cascade) and/or `workspace/src/api.rs` — remove `<app_data>/projects/<id>/artifacts/` on project delete.

> **Sequencing note:** Tasks 1–2 (registry) and Tasks 6–7 (`reconcile_occupancy`) are independent and can be done in any order. Tasks 3–5 (spawner widening) must precede Task 9 (engine threads working dir). Tasks 8 (artifact base) precedes Task 11 (read_artifact) and Task 12 (delete cascade). Task 10 (wire spawners + exit + brake) depends on Tasks 1–5. Do them in the listed order for the cleanest diff.

---

## Task 1: `ProcessRegistry` — register / deregister / list

**Files:**
- Create: `src-tauri/app/src/process_registry.rs`
- Modify: `src-tauri/app/src/lib.rs` (add `mod process_registry;` near the other `mod` declarations)
- Modify: `src-tauri/app/Cargo.toml` (add `libc`)

- [ ] **Step 1: Add the `libc` dependency**

In `src-tauri/app/Cargo.toml`, under `[dependencies]` (after the `skills` line), add:

```toml
libc = "0.2"
```

- [ ] **Step 2: Write the failing test for register/deregister bookkeeping**

Create `src-tauri/app/src/process_registry.rs` with ONLY the test module first (the type does not exist yet, so it fails to compile = a failing test):

```rust
//! ProcessRegistry — the composition-root holder of live child process-GROUP
//! ids. The killable spawner registers a pgid on spawn and deregisters it on
//! completion; `kill_all` signals every registered group (SIGTERM, grace,
//! SIGKILL). Deliberately lives in the `app` crate: no Runner/ChatRunner ACL
//! trait ever gains process types — process control is a root concern.

use std::collections::HashSet;
use std::sync::Mutex;

/// Shared registry of live child process-group ids (pgid == leader pid because
/// the spawner sets `.process_group(0)`). Cloned behind an `Arc` at the root.
#[derive(Default)]
pub struct ProcessRegistry {
    /// The set of currently-live process-group ids.
    groups: Mutex<HashSet<i32>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_then_deregister_leaves_the_set_empty() {
        let reg = ProcessRegistry::new();
        reg.register(1234);
        reg.register(5678);
        assert_eq!(reg.len(), 2);
        reg.deregister(1234);
        assert_eq!(reg.len(), 1);
        reg.deregister(5678);
        assert_eq!(reg.len(), 0);
    }

    #[test]
    fn deregister_unknown_pgid_is_a_noop() {
        let reg = ProcessRegistry::new();
        reg.deregister(9999);
        assert_eq!(reg.len(), 0);
    }

    #[test]
    fn register_is_idempotent_on_the_same_pgid() {
        let reg = ProcessRegistry::new();
        reg.register(42);
        reg.register(42);
        assert_eq!(reg.len(), 1);
    }
}
```

Add `mod process_registry;` to `src-tauri/app/src/lib.rs` (alongside the other top-level `mod` lines near the top of the file — search for an existing `mod events;`-style declaration and place it adjacent).

- [ ] **Step 3: Run the test to verify it fails**

Run: `cd src-tauri && cargo test -p agent-bus-app process_registry`
Expected: FAIL to compile — `no function or associated item named 'new'`, `register`, `deregister`, `len` not found.

- [ ] **Step 4: Implement the bookkeeping methods**

Add to `src-tauri/app/src/process_registry.rs`, in an `impl ProcessRegistry` block ABOVE the test module:

```rust
impl ProcessRegistry {
    pub fn new() -> Self {
        Self { groups: Mutex::new(HashSet::new()) }
    }

    /// Record a live child process-group id (the spawner calls this right after
    /// spawn). Idempotent — re-registering the same pgid is a no-op.
    pub fn register(&self, pgid: i32) {
        self.groups.lock().unwrap().insert(pgid);
    }

    /// Drop a process-group id once its child has been waited on. A pgid that is
    /// not present is fine (best-effort).
    pub fn deregister(&self, pgid: i32) {
        self.groups.lock().unwrap().remove(&pgid);
    }

    /// Count of currently-registered groups (test/inspection helper).
    pub fn len(&self) -> usize {
        self.groups.lock().unwrap().len()
    }

    /// Whether the registry currently holds no live groups.
    pub fn is_empty(&self) -> bool {
        self.groups.lock().unwrap().is_empty()
    }

    /// Snapshot of the live pgids (used by `kill_all` so the kill loop does not
    /// hold the lock while sleeping).
    fn snapshot(&self) -> Vec<i32> {
        self.groups.lock().unwrap().iter().copied().collect()
    }
}
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cd src-tauri && cargo test -p agent-bus-app process_registry`
Expected: PASS (3 tests).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/app/src/process_registry.rs src-tauri/app/src/lib.rs src-tauri/app/Cargo.toml
git commit -m "feat(app): ProcessRegistry register/deregister bookkeeping

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: `ProcessRegistry::kill_all` — SIGTERM → grace → SIGKILL on the group

**Files:**
- Modify: `src-tauri/app/src/process_registry.rs`

- [ ] **Step 1: Write the failing test (real process group + a child that ignores SIGTERM)**

Append to the `tests` module in `src-tauri/app/src/process_registry.rs`:

```rust
    #[cfg(unix)]
    mod unix_kill {
        use super::super::*;
        use std::process::Command;
        use std::os::unix::process::CommandExt;
        use std::time::{Duration, Instant};

        /// Is the process group still alive? `kill(-pgid, 0)` probes without
        /// signalling: Ok(0) => alive, Err(ESRCH) => gone.
        fn group_alive(pgid: i32) -> bool {
            unsafe { libc::kill(-pgid, 0) == 0 }
        }

        #[test]
        fn kill_all_terminates_a_well_behaved_group_and_empties_the_registry() {
            // `sh -c 'sleep 30 & wait'` => the shell leads the group, a child
            // sleep is in the same group, so killing the GROUP must take both.
            let child = Command::new("sh")
                .arg("-c")
                .arg("sleep 30 & wait")
                .process_group(0)
                .spawn()
                .expect("spawn");
            let pgid = child.id() as i32;
            let reg = ProcessRegistry::new();
            reg.register(pgid);

            reg.kill_all();

            assert!(reg.is_empty(), "registry drained after kill_all");
            // Give the OS a beat to reap, then assert the group is gone.
            let deadline = Instant::now() + Duration::from_secs(3);
            while group_alive(pgid) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(50));
            }
            assert!(!group_alive(pgid), "group must be dead after kill_all");
        }

        #[test]
        fn kill_all_escalates_to_sigkill_for_a_sigterm_ignoring_group() {
            // `trap '' TERM` makes the shell ignore SIGTERM; only SIGKILL ends it.
            let child = Command::new("sh")
                .arg("-c")
                .arg("trap '' TERM; sleep 30")
                .process_group(0)
                .spawn()
                .expect("spawn");
            let pgid = child.id() as i32;
            let reg = ProcessRegistry::new();
            reg.register(pgid);

            let start = Instant::now();
            reg.kill_all();
            // kill_all blocks for ~the grace window then SIGKILLs.
            assert!(start.elapsed() < Duration::from_secs(6), "grace bounded");

            let deadline = Instant::now() + Duration::from_secs(3);
            while group_alive(pgid) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(50));
            }
            assert!(!group_alive(pgid), "SIGTERM-ignoring group dies via SIGKILL");
        }
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd src-tauri && cargo test -p agent-bus-app process_registry::tests::unix_kill`
Expected: FAIL to compile — `no method named 'kill_all'`.

- [ ] **Step 3: Implement `kill_all`**

Add to the `impl ProcessRegistry` block:

```rust
    /// Best-effort graceful kill of every registered process GROUP: SIGTERM the
    /// group, poll up to a ~2.5s grace window for it to exit, then SIGKILL any
    /// survivor. Drains the registry. Idempotent — a gone group (ESRCH) is fine.
    /// Negative pid signals the whole group (the spawner set `.process_group(0)`
    /// so pgid == the child's pid), so `claude`'s own tool/subagent children die
    /// too. On non-unix this is a logged no-op (the documented Windows gap).
    pub fn kill_all(&self) {
        let pgids = self.snapshot();
        if pgids.is_empty() {
            self.groups.lock().unwrap().clear();
            return;
        }
        #[cfg(unix)]
        {
            use std::time::{Duration, Instant};
            // 1. SIGTERM every group.
            for &pgid in &pgids {
                unsafe { libc::kill(-pgid, libc::SIGTERM); }
            }
            // 2. Grace poll (~2.5s) for natural exit.
            let deadline = Instant::now() + Duration::from_millis(2500);
            loop {
                let alive: Vec<i32> = pgids
                    .iter()
                    .copied()
                    .filter(|&pgid| unsafe { libc::kill(-pgid, 0) } == 0)
                    .collect();
                if alive.is_empty() || Instant::now() >= deadline {
                    // 3. SIGKILL any survivor.
                    for &pgid in &alive {
                        unsafe { libc::kill(-pgid, libc::SIGKILL); }
                    }
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        #[cfg(not(unix))]
        {
            eprintln!(
                "ProcessRegistry::kill_all: non-unix build cannot kill {} process group(s) \
                 (documented Windows gap)",
                pgids.len()
            );
        }
        // Drain regardless of platform/outcome — these handles are spent.
        self.groups.lock().unwrap().clear();
    }
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd src-tauri && cargo test -p agent-bus-app process_registry`
Expected: PASS (5 tests). The escalation test takes ~2.5s (the grace window).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/app/src/process_registry.rs
git commit -m "feat(app): ProcessRegistry::kill_all SIGTERM->grace->SIGKILL on group

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: Widen the worker `SpawnFn` to carry a working dir + add `working_dir` to `InvocationRequest`

The SpawnFn is a type alias (NOT a trait) — widening it does not touch the `Runner` ACL trait. The working dir is threaded as an `Option<&str>` second argument.

**Files:**
- Modify: `src-tauri/runners/src/output.rs:114` (`InvocationRequest`)
- Modify: `src-tauri/runners/src/claude_cli.rs`

- [ ] **Step 1: Write the failing test (spawner receives the working dir)**

In `src-tauri/runners/src/claude_cli.rs`, append to the `tests` module:

```rust
    #[tokio::test]
    async fn spawner_receives_the_request_working_dir() {
        use std::sync::{Arc, Mutex};
        let seen: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let s = seen.clone();
        let canned = r#"{"type":"result","is_error":false,"result":"VERDICT: approve","usage":{"input_tokens":1,"output_tokens":1}}"#;
        let runner = ClaudeCliRunner::with_spawner(Box::new(move |_args, cwd| {
            *s.lock().unwrap() = cwd.map(|c| c.to_string());
            Ok(canned.to_string())
        }));
        let mut r = req();
        r.working_dir = Some("/tmp/work-here".into());
        runner.invoke(&r).await.unwrap();
        assert_eq!(seen.lock().unwrap().as_deref(), Some("/tmp/work-here"));
    }
```

Also update the existing `req()` builder in the test module to set the new field (so it still compiles):

```rust
            sandbox_profile: None,
            working_dir: None,
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd src-tauri && cargo test -p runners claude_cli`
Expected: FAIL to compile — `InvocationRequest` has no field `working_dir`; closure arity mismatch.

- [ ] **Step 3: Add the field to `InvocationRequest`**

In `src-tauri/runners/src/output.rs`, inside `pub struct InvocationRequest { ... }` (after `sandbox_profile`):

```rust
    /// The directory the child `claude` process runs in (LF26). The composition
    /// root resolves this to the work-item's working dir (worktree for
    /// implementers, target repo otherwise). `None` = inherit the parent's cwd
    /// (the pre-LF26 behaviour, kept for the plain `.output()` spawner + tests).
    pub working_dir: Option<String>,
```

- [ ] **Step 4: Widen the `SpawnFn` alias and the production `.output()` spawner**

In `src-tauri/runners/src/claude_cli.rs`, change the alias at line 15-16:

```rust
pub type SpawnFn =
    Box<dyn Fn(&[String], Option<&str>) -> Result<String, RunnerError> + Send + Sync>;
```

Change the production spawner in `new()` (lines 52-68) to accept and apply the cwd:

```rust
            spawn: Box::new(|args: &[String], cwd: Option<&str>| {
                let (program, rest) = args
                    .split_first()
                    .ok_or_else(|| RunnerError::Spawn("empty argv".into()))?;
                let mut cmd = std::process::Command::new(program);
                cmd.args(rest);
                if let Some(dir) = cwd {
                    cmd.current_dir(dir);
                }
                let output = cmd
                    .output()
                    .map_err(|e| RunnerError::Spawn(e.to_string()))?;
                interpret_runner_output(
                    output.stdout,
                    output.stderr,
                    output.status.success(),
                )
            }),
```

Change the call site in `run_once` (line 96) to pass the request's working dir:

```rust
        let stdout = (self.spawn)(&argv, req.working_dir.as_deref())?;
```

- [ ] **Step 5: Fix every existing test closure in this file to the new arity**

Each existing `with_spawner(Box::new(move |args| ...))` and `move |_args| ...` in the `tests` module must become `|args, _cwd|` / `|_args, _cwd|`. Update all of them (the tests at lines ~145, ~159, ~172, ~187, ~196, ~214, ~228).

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test -p runners`
Expected: PASS (all existing runner tests + the new one).

- [ ] **Step 7: Commit**

```bash
git add src-tauri/runners/src/output.rs src-tauri/runners/src/claude_cli.rs
git commit -m "feat(runners): SpawnFn carries working_dir; InvocationRequest.working_dir

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: Widen the chat `SpawnFn` to carry a working dir + add `working_dir` to `ChatRequest`

**Files:**
- Modify: `src-tauri/llm_chat/src/chat.rs:25` (`ChatRequest`)
- Modify: `src-tauri/llm_chat/src/claude_cli.rs`

- [ ] **Step 1: Write the failing test (chat spawner receives the working dir)**

In `src-tauri/llm_chat/src/claude_cli.rs`, append to the `tests` module:

```rust
    #[tokio::test]
    async fn chat_spawner_receives_the_request_working_dir() {
        let seen: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let s = seen.clone();
        let runner = ClaudeChatRunner::with_spawner(Box::new(move |_args, cwd| {
            *s.lock().unwrap() = cwd.map(|c| c.to_string());
            Ok(FIRST.to_string())
        }));
        let mut r = req("hi");
        r.working_dir = Some("/tmp/chat-here".into());
        runner.chat(&r).await.unwrap();
        assert_eq!(seen.lock().unwrap().as_deref(), Some("/tmp/chat-here"));
    }
```

Update the test `req(...)` builder (lines 154-162) to set the new field:

```rust
            thinking_budget: 8192,
            working_dir: None,
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd src-tauri && cargo test -p llm_chat claude_cli`
Expected: FAIL to compile — `ChatRequest` has no field `working_dir`; closure arity mismatch.

- [ ] **Step 3: Add the field to `ChatRequest`**

In `src-tauri/llm_chat/src/chat.rs`, inside `pub struct ChatRequest { ... }` (after `thinking_budget`):

```rust
    /// The directory the child `claude` process runs in (LF26 parity with the
    /// worker runner). `None` = inherit the parent's cwd (the default, used by
    /// the terminal chat which has no work-item working dir).
    pub working_dir: Option<String>,
```

- [ ] **Step 4: Widen the chat `SpawnFn` alias and the production `.output()` spawner**

In `src-tauri/llm_chat/src/claude_cli.rs`, change the alias at line 17:

```rust
pub type SpawnFn = Box<dyn Fn(&[String], Option<&str>) -> Result<String, ChatError> + Send + Sync>;
```

Change the production spawner in `new()` (lines 58-68) — note the chat spawner passes the WHOLE `args` to `CLAUDE_BIN` (no split-first):

```rust
            spawn: Box::new(|args: &[String], cwd: Option<&str>| {
                let mut cmd = std::process::Command::new(CLAUDE_BIN);
                cmd.args(args);
                if let Some(dir) = cwd {
                    cmd.current_dir(dir);
                }
                let output = cmd
                    .output()
                    .map_err(|e| ChatError::Spawn(e.to_string()))?;
                interpret_chat_output(
                    output.stdout,
                    output.stderr,
                    output.status.success(),
                )
            }),
```

Change the call site in `run_once` (line 90):

```rust
        let stdout = (self.spawn)(&args, req.working_dir.as_deref())?;
```

- [ ] **Step 5: Fix every existing chat test closure to the new arity**

Update all `with_spawner(Box::new(move |args| ...))` / `move |_args| ...` closures in the `tests` module (lines ~169, ~189, ~211, ~233, ~248, ~265, ~274) to `|args, _cwd|` / `|_args, _cwd|`.

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test -p llm_chat`
Expected: PASS (all existing chat tests + the new one).

- [ ] **Step 7: Commit**

```bash
git add src-tauri/llm_chat/src/chat.rs src-tauri/llm_chat/src/claude_cli.rs
git commit -m "feat(llm_chat): chat SpawnFn carries working_dir; ChatRequest.working_dir

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 5: Fix all OTHER callers of the widened SpawnFn / InvocationRequest / ChatRequest

Widening the alias + adding fields breaks any other construction site. Find and fix them all before moving on.

**Files:** discovered by grep below.

- [ ] **Step 1: Find every affected construction site**

Run:

```bash
cd src-tauri && grep -rn "InvocationRequest {" --include=*.rs | grep -v "pub struct"
grep -rn "ChatRequest {" --include=*.rs | grep -v "pub struct"
grep -rn "with_spawner(" --include=*.rs
```

- [ ] **Step 2: Add `working_dir: None` to every `InvocationRequest { ... }` literal that lacks it**

Including `runtime/src/engine.rs:1240` (the engine's `invoke` — this one is rewritten in Task 9, but add the field now so the workspace compiles), and any builder in `runners`/`conversational_control`/`runtime` tests. The `engine.rs` literal becomes (working_dir is set properly in Task 9):

```rust
        sandbox_profile: None,
        working_dir: None,
```

- [ ] **Step 3: Add `working_dir: None` to every `ChatRequest { ... }` literal that lacks it**

Search `conversational_control` (the agentic chat engine builds `ChatRequest`) and `llm_chat`'s `anthropic_api` runner/tests. Add `working_dir: None` to each.

- [ ] **Step 4: Fix any non-test `with_spawner` closures to the new arity**

(The fake/test runners; the `anthropic_api` runners do not use `SpawnFn`.) Change `|args|` → `|args, _cwd|`.

- [ ] **Step 5: Verify the whole workspace compiles + tests pass**

Run: `cd src-tauri && cargo test`
Expected: PASS across all crates. If a crate fails to compile, it has an un-updated literal/closure — fix it.

- [ ] **Step 6: Commit**

```bash
git add -A src-tauri
git commit -m "fix: thread working_dir through all SpawnFn/Invocation/Chat call sites

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 6: `StoreRepo::reconcile_occupancy` — rebuild occupancy from resident tasks

**Files:**
- Modify: `src-tauri/runtime/src/store.rs`

- [ ] **Step 1: Write the failing test**

In `src-tauri/runtime/src/store.rs`, append to the `tests` module. Insert task rows directly via SQL (the store tests use FK-less pools), using the resident-state set:

```rust
    async fn insert_task(pool: &SqlitePool, id: &str, run_id: &str, stage: &str, state: &str) {
        sqlx::query(
            "INSERT INTO tasks
               (id, project_id, pipeline, topic, current_stage, state, attempts,
                created_at, updated_at, run_id)
             VALUES (?,?,?,?,?,?,0,100,100,?)",
        )
        .bind(id)
        .bind("p")
        .bind("pipe")
        .bind("topic")
        .bind(stage)
        .bind(state)
        .bind(run_id)
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn reconcile_sets_occupancy_to_resident_count() {
        let pool = fresh_pool().await;
        let repo = StoreRepo::new(pool.clone());
        repo.ensure("R1", "spec", 5).await.unwrap();
        // Inflate occupancy as a kill/crash would leave it.
        repo.reserve("R1", "spec").await.unwrap();
        repo.reserve("R1", "spec").await.unwrap();
        repo.reserve("R1", "spec").await.unwrap();
        assert_eq!(repo.occupancy("R1", "spec").await.unwrap(), Some(3));

        // Reality: 2 resident items at `spec` (queued + running), 1 done (left),
        // 1 needs_human (left), and one item at a different stage.
        insert_task(&pool, "t-queued", "R1", "spec", "queued").await;
        insert_task(&pool, "t-running", "R1", "spec", "running").await;
        insert_task(&pool, "t-done", "R1", "spec", "done").await;
        insert_task(&pool, "t-nh", "R1", "spec", "needs_human").await;
        insert_task(&pool, "t-other", "R1", "plan", "queued").await;

        repo.reconcile_occupancy("R1").await.unwrap();
        assert_eq!(repo.occupancy("R1", "spec").await.unwrap(), Some(2));
    }

    #[tokio::test]
    async fn reconcile_clears_a_fully_leaked_reservation_to_zero() {
        let pool = fresh_pool().await;
        let repo = StoreRepo::new(pool.clone());
        repo.ensure("R1", "spec", 3).await.unwrap();
        repo.reserve("R1", "spec").await.unwrap();
        repo.reserve("R1", "spec").await.unwrap();
        assert_eq!(repo.occupancy("R1", "spec").await.unwrap(), Some(2));
        // No resident tasks at all -> occupancy must drop to 0.
        repo.reconcile_occupancy("R1").await.unwrap();
        assert_eq!(repo.occupancy("R1", "spec").await.unwrap(), Some(0));
    }

    #[tokio::test]
    async fn reconcile_counts_gated_revising_joining_as_resident() {
        let pool = fresh_pool().await;
        let repo = StoreRepo::new(pool.clone());
        repo.ensure("R1", "gate-1", 9).await.unwrap();
        insert_task(&pool, "g1", "R1", "gate-1", "gated").await;
        insert_task(&pool, "g2", "R1", "gate-1", "revising").await;
        insert_task(&pool, "g3", "R1", "gate-1", "joining").await;
        repo.reconcile_occupancy("R1").await.unwrap();
        assert_eq!(repo.occupancy("R1", "gate-1").await.unwrap(), Some(3));
    }

    #[tokio::test]
    async fn reconcile_only_touches_the_given_run() {
        let pool = fresh_pool().await;
        let repo = StoreRepo::new(pool.clone());
        repo.ensure("R1", "spec", 5).await.unwrap();
        repo.ensure("R2", "spec", 5).await.unwrap();
        repo.reserve("R2", "spec").await.unwrap(); // R2 leaked
        insert_task(&pool, "a", "R1", "spec", "queued").await;
        repo.reconcile_occupancy("R1").await.unwrap();
        assert_eq!(repo.occupancy("R1", "spec").await.unwrap(), Some(1));
        // R2 untouched by an R1 reconcile.
        assert_eq!(repo.occupancy("R2", "spec").await.unwrap(), Some(2));
    }
```

> Note: `fresh_pool()` in `store.rs` tests already applies `001_initial.sql`, `003_runtime.sql`, `012_runtime_stores.sql` (line 121-123) — that gives both `tasks` (with `run_id`) and `stores`. The `tasks` insert columns above match `task_store.rs` insert ordering minus the optional fanout columns (which default NULL).

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src-tauri && cargo test -p runtime reconcile`
Expected: FAIL to compile — `no method named 'reconcile_occupancy'`.

- [ ] **Step 3: Implement `reconcile_occupancy`**

Add to `impl StoreRepo` in `src-tauri/runtime/src/store.rs` (after `is_full`):

```rust
    /// Boot/resume reconciliation: for every store of `run_id`, set `occupancy`
    /// to the count of work-items actually resident at that stage — the truth
    /// from `tasks`. Residency = a work-item parked at `current_stage = <stage>`
    /// in a state that occupies a slot: `queued`, `running`, `gated`, `revising`,
    /// `joining`. (`done` left the store; `needs_human` was routed to the
    /// escalation stage and its slot released.) This clears reservations leaked by
    /// a killed/crashed worker (occupancy incremented at `reserve`, never released)
    /// so a resumed run is not falsely backpressured. Run AFTER
    /// `TaskStore::release_orphaned_running` so just-requeued items are counted.
    pub async fn reconcile_occupancy(&self, run_id: &str) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE stores
                SET occupancy = (
                    SELECT COUNT(*) FROM tasks
                     WHERE tasks.run_id = stores.run_id
                       AND tasks.current_stage = stores.stage
                       AND tasks.state IN
                           ('queued','running','gated','revising','joining')
                )
              WHERE run_id = ?",
        )
        .bind(run_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test -p runtime reconcile`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/runtime/src/store.rs
git commit -m "feat(runtime): StoreRepo::reconcile_occupancy rebuilds occupancy from tasks

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 7: Wire `reconcile_occupancy` at boot, after `release_orphaned_running`

**Files:**
- Modify: `src-tauri/app/src/lib.rs` (around `:1288`–`:1296`)

- [ ] **Step 1: Add the boot reconciliation call**

In `src-tauri/app/src/lib.rs`, the `stores` Arc is created at line 1293 — AFTER the `release_orphaned_running` call at line 1288. Move the reconciliation to run after `stores` exists. Immediately after line 1295 (`let ledger = ...`) and before the `RuntimeState::new` build, insert:

```rust
                // Resume reconciliation (LF20 occupancy leak): after orphaned
                // `running` rows were requeued above, rebuild each store's
                // occupancy from the resident work-items so a slot reserved before
                // a kill/crash (and never released) does not keep the resumed run
                // falsely backpressured. Per active run. Best-effort: a reconcile
                // failure must not block boot.
                if let Ok(Some(active)) = runs.latest_active_for_project(&project_id).await {
                    if let Err(e) = stores.reconcile_occupancy(&active.id).await {
                        eprintln!("app: boot reconcile_occupancy failed: {e}");
                    }
                }
```

> Confirm `runs.latest_active_for_project(project_id) -> Result<Option<Run>>` is the same call `pipeline_activator::active_run` uses (`pipeline_activator.rs:371`). `Run` exposes `.id` (used at `pipeline_activator.rs` run loops). If `project_id` is empty (no project), `latest_active_for_project` returns `Ok(None)` and the block is a no-op.

- [ ] **Step 2: Verify it compiles**

Run: `cd src-tauri && cargo build -p agent-bus-app`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/app/src/lib.rs
git commit -m "feat(app): reconcile store occupancy at boot after orphan release

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 8: Absolute, app-owned artifact base in the engine (LF26)

Today `artifact_path`/`artifact_dir` emit a relative `${project}/artifacts/...` shape resolved by the scope layer against `project_root`. We change them to compose against an absolute base carried on `EngineContext`, substitute the resolved absolute dir into the agent's `output_contract`, scope-write + `--add-dir` it, and write the artifact-base parent into the scope so `claude` may write outside its cwd.

**Files:**
- Modify: `src-tauri/runtime/src/engine.rs` (`EngineContext` `:130`; `artifact_path` `:1382`; `artifact_dir` `:1389`; the generator `generate_once` `:887`–`:891`; the transformer `transform_once` `:320`–`:322`; `invoke` `:1204`–`:1216`)

- [ ] **Step 1: Write the failing test (absolute base composition + generator/transformer parity)**

The `artifact_path`/`artifact_dir` fns become methods on `EngineContext` (they need the base). Add to the engine's test module (the `test_support` builder at `engine.rs:1416` constructs an `EngineContext` — extend it to set `artifact_base`). Add a unit test that does not need the full context for the pure shape:

```rust
    #[test]
    fn artifact_path_is_absolute_under_the_app_data_base() {
        let base = std::path::PathBuf::from("/data/projects/p1/artifacts");
        assert_eq!(
            super::compose_artifact_path(&base, "spec", "my key/ish", 2),
            "/data/projects/p1/artifacts/spec/my-key-ish-v2.md"
        );
        assert_eq!(
            super::compose_artifact_dir(&base, "spec"),
            "/data/projects/p1/artifacts/spec"
        );
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd src-tauri && cargo test -p runtime artifact_path_is_absolute`
Expected: FAIL to compile — `compose_artifact_path` / `compose_artifact_dir` not found.

- [ ] **Step 3: Add `artifact_base` to `EngineContext` and pure compose helpers**

In `src-tauri/runtime/src/engine.rs`, add to the `EngineContext` struct (after `target_repo` at `:150`):

```rust
    /// Absolute, app-owned artifact base for this run's project:
    /// `<app_data>/projects/<project_id>/artifacts` (LF26). Artifacts compose
    /// under `<base>/<stage>/<key>-v<attempt>.md`. Resolved at the composition
    /// root from the Tauri app-data dir and handed in alongside `project_root`,
    /// so workers write app-owned files regardless of their cwd.
    pub artifact_base: PathBuf,
```

Replace `artifact_path` (`:1382`) and `artifact_dir` (`:1389`) with base-aware pure helpers + thin `EngineContext` methods:

```rust
/// Compose the absolute artifact path for a work-item's output under `base`:
/// `<base>/<stage>/<key>-v<attempt>.md`. `key` is sanitised to one safe path
/// segment. PURE.
pub fn compose_artifact_path(base: &std::path::Path, stage: &str, key: &str, attempt: u32) -> String {
    let safe_key = sanitize_key(key);
    base.join(stage)
        .join(format!("{safe_key}-v{attempt}.md"))
        .to_string_lossy()
        .into_owned()
}

/// Compose the absolute artifact directory for `stage` under `base`:
/// `<base>/<stage>` (granted write access + handed to `output_contract`). PURE.
pub fn compose_artifact_dir(base: &std::path::Path, stage: &str) -> String {
    base.join(stage).to_string_lossy().into_owned()
}

impl EngineContext {
    /// This run's absolute artifact path for `stage`/`key`/`attempt`.
    pub fn artifact_path(&self, stage: &str, key: &str, attempt: u32) -> String {
        compose_artifact_path(&self.artifact_base, stage, key, attempt)
    }
    /// This run's absolute artifact directory for `stage`.
    pub fn artifact_dir(&self, stage: &str) -> String {
        compose_artifact_dir(&self.artifact_base, stage)
    }
}
```

> Keep `sanitize_key` (`:1395`) as-is. Delete the old free `artifact_path`/`artifact_dir` fns (they are replaced) — then fix every caller in the next step.

- [ ] **Step 4: Update the three callers to the context methods**

- Transformer `transform_once` (`engine.rs:320`): replace `let dir = artifact_dir(&team.id);` with `let dir = ctx.artifact_dir(&team.id);`.
- The transformer's produced-artifact fallback (`engine.rs:~377`, `artifact_path(&team.id, &produced_key, task.attempts)`): replace with `ctx.artifact_path(&team.id, &produced_key, task.attempts)`.
- Generator `generate_once` (`engine.rs:887`): replace `let dir = artifact_dir(&source_team.id);` with `let dir = ctx.artifact_dir(&source_team.id);`. (Now the generator composes the SAME absolute shape the transformer does — closing the relative-path scatter.)

- [ ] **Step 5: Update `invoke` to scope-write the absolute dir + ensure it exists**

In `invoke` (`engine.rs:1212`–`:1216`), the line `scope.writes.push(artifact_dir(&team.id));` becomes:

```rust
    // Grant write access to this stage's ABSOLUTE artifact dir (the L1 + LF26
    // fix): the dir is outside the worker's cwd, so it must be in scope.writes
    // (settings allow) AND surfaced as an --add-dir (the build_settings pass
    // turns scope.writes into both). Create it so the agent can write there.
    let artifact_dir = ctx.artifact_dir(&team.id);
    let _ = std::fs::create_dir_all(&artifact_dir);
    let mut scope = team.scope.clone();
    scope.writes.push(artifact_dir);
```

> The `--add-dir` requirement (spec item 3) is satisfied automatically: `build_settings` (`runners/src/scope.rs:50`) turns scope writes into `add_dirs`. Confirm by reading `scope.rs:44`–`:60` — the `add_dirs` are derived from the scope's add patterns and writes. If, on reading, writes do NOT feed `add_dirs`, additionally push the dir onto the request's `add_dirs` in Task 9.

- [ ] **Step 6: Extend the test_support `EngineContext` builder to set `artifact_base`**

In the `test_support` module (`engine.rs:1416`), the `EngineContext { ... }` literal must set `artifact_base`. Add a sensible default, e.g. a temp dir or `PathBuf::from("/tmp/test-artifacts")`:

```rust
            artifact_base: std::path::PathBuf::from("/tmp/agent-bus-test-artifacts"),
```

(Use the existing `project_root` test value's parent if the builder already has a tempdir; the unit tests above do not assert disk writes.)

- [ ] **Step 7: Run the engine tests**

Run: `cd src-tauri && cargo test -p runtime`
Expected: PASS. Watch for any remaining caller of the deleted free fns — fix to the `ctx.` method.

- [ ] **Step 8: Commit**

```bash
git add src-tauri/runtime/src/engine.rs
git commit -m "feat(runtime): absolute app-owned artifact base on EngineContext (LF26)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 9: Thread the work-item working dir into `InvocationRequest.working_dir`

**Files:**
- Modify: `src-tauri/runtime/src/engine.rs` (`invoke`, `:1204`–`:1250`)

- [ ] **Step 1: Write the failing test**

The cleanest seam to assert: `invoke` sets `req.working_dir` to the resolved `effective_target_repo`. Use a `Runner` test double that captures the request. The runners crate has a fake (`runners/src/fake.rs`) but it may not capture the request; if not, add a tiny capturing runner in the engine test module:

```rust
    #[tokio::test]
    async fn invoke_sets_working_dir_to_effective_target_repo() {
        use std::sync::{Arc, Mutex};
        use runners::output::{Runner, RunnerError, RunnerOutput, InvocationRequest, LogSink};

        struct CapturingRunner(Arc<Mutex<Option<String>>>);
        #[async_trait::async_trait]
        impl Runner for CapturingRunner {
            async fn invoke(&self, req: &InvocationRequest) -> Result<RunnerOutput, RunnerError> {
                *self.0.lock().unwrap() = req.working_dir.clone();
                Ok(RunnerOutput::approve_with("KEY: k\nARTIFACT: a.md"))
            }
            async fn invoke_stream(&self, req: &InvocationRequest, _s: &LogSink) -> Result<RunnerOutput, RunnerError> {
                self.invoke(req).await
            }
        }

        let seen = Arc::new(Mutex::new(None));
        let mut ctx = test_support::ctx_with_runner(Arc::new(CapturingRunner(seen.clone())));
        ctx.target_repo = Some(std::path::PathBuf::from("/repo/here"));
        // drive one generator/transformer invoke via test_support helper…
        // (use the existing engine test harness that runs a single invoke)
        let team = test_support::team("research");
        let _ = super::generate_once(&ctx, &team).await;
        assert_eq!(seen.lock().unwrap().as_deref(), Some("/repo/here"));
    }
```

> Adapt to the exact `test_support` API present in `engine.rs:1416` (helper names like `ctx()`, `team(...)`). If `RunnerOutput::approve_with` does not exist, build a `RunnerOutput` with `final_text` set to a parseable item line — read `runners/src/output.rs` for the constructor / fields. The load-bearing assertion is `req.working_dir == Some(effective_target_repo)`.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd src-tauri && cargo test -p runtime invoke_sets_working_dir`
Expected: FAIL — `working_dir` is `None` (not yet set by `invoke`).

- [ ] **Step 3: Set `working_dir` in `invoke`**

In `src-tauri/runtime/src/engine.rs`, `invoke` already computes the effective target repo into `vars` (`:1209`). Capture it as the working dir and set it on the request. After the `vars` block (`:1211`), add:

```rust
    // LF26: the child `claude` runs in the work-item's resolved working dir —
    // the worktree for implementers, the target repo otherwise. v1 has no
    // per-item worktree wiring, so this is the effective `${target_repo}` (task
    // override → project default). `None` keeps the pre-LF26 inherit-cwd
    // behaviour for a topic-less run with no target repo configured.
    let working_dir = effective_target_repo(task.target_repo.as_deref(), ctx.target_repo.as_deref())
        .map(|p| p.to_string_lossy().into_owned());
```

In the `InvocationRequest { ... }` literal (`:1240`), set:

```rust
        sandbox_profile: None,
        working_dir,
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd src-tauri && cargo test -p runtime invoke_sets_working_dir`
Expected: PASS.

- [ ] **Step 5: Run the full runtime suite**

Run: `cd src-tauri && cargo test -p runtime`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/runtime/src/engine.rs
git commit -m "feat(runtime): set InvocationRequest.working_dir to effective target repo (LF26)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 10: Wire the registry-backed killable spawners + exit handler + brake-on kills at the root

**Files:**
- Modify: `src-tauri/app/src/lib.rs` (registry build; exit handler `:1542`; RootDispatcher `brake_on` `:1067`; auto-meter `SetOn` `:1473`)
- Modify: `src-tauri/app/src/pipeline_activator.rs` (build the worker `ClaudeCliRunner::with_spawner` with the registry; carry the registry on `WorkerDeps` / the activator)

This task is composition-root wiring — its behavior is carried by the already-tested `ProcessRegistry` (Tasks 1-2) and spawner widening (Tasks 3-4). Verify structurally + with a focused spawner test.

- [ ] **Step 1: Add a focused test: a registry-backed spawner registers-then-deregisters around a real `echo`**

This proves the production spawner closure shape. Add to `src-tauri/app/src/process_registry.rs` tests (it can construct a `ClaudeCliRunner::with_spawner` since `app` depends on `runners`):

```rust
    #[cfg(unix)]
    #[tokio::test]
    async fn killable_spawner_runs_echo_captures_output_and_drains_registry() {
        use std::sync::Arc;
        use runners::output::Runner;
        let reg = Arc::new(ProcessRegistry::new());
        let runner = crate::build_killable_worker_runner(reg.clone());
        // Build a minimal InvocationRequest that resolves to `echo` via a stub.
        // Simplest: assert the registry is empty before and after a run.
        // (A full invoke needs a parseable claude stream; instead assert the
        // spawner closure registers/deregisters using a direct call.)
        let out = (crate::killable_spawn(&reg))(
            &["sh".into(), "-c".into(), "printf hello".into()],
            None,
        );
        assert_eq!(out.unwrap(), "hello");
        assert!(reg.is_empty(), "registry drained after the child is waited on");
    }
```

> If exposing `killable_spawn`/`build_killable_worker_runner` as `pub(crate)` testable units is cleaner than a full `Runner::invoke`, do that — the spawner closure is the unit under test. Adjust names to whatever you implement in Step 3.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd src-tauri && cargo test -p agent-bus-app killable_spawner`
Expected: FAIL to compile — `killable_spawn` / `build_killable_worker_runner` not found.

- [ ] **Step 3: Implement the killable spawner factory in `app`**

Add to `src-tauri/app/src/lib.rs` (or a small helper module) a function that builds the production spawner closure capturing the registry. It mirrors the runner's `.output()` spawner but uses `.process_group(0)`, registers the pgid, captures stdout/stderr to completion + `.wait()`, deregisters, and returns `interpret_runner_output`:

```rust
use std::sync::Arc;
use crate::process_registry::ProcessRegistry;

/// Build the production worker spawner closure: spawns `claude` in its own
/// process group, registers the pgid, captures output to completion, waits,
/// deregisters, and maps the result via `interpret_runner_output`. The group +
/// registry are what make `kill_all` reach `claude`'s own children (LF20).
pub(crate) fn killable_spawn(
    registry: &Arc<ProcessRegistry>,
) -> runners::claude_cli::SpawnFn {
    let registry = registry.clone();
    Box::new(move |args: &[String], cwd: Option<&str>| {
        use runners::output::RunnerError;
        use std::process::Stdio;
        let (program, rest) = args
            .split_first()
            .ok_or_else(|| RunnerError::Spawn("empty argv".into()))?;
        let mut cmd = std::process::Command::new(program);
        cmd.args(rest).stdout(Stdio::piped()).stderr(Stdio::piped());
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0); // child leads a fresh group; pgid == child pid
        }
        let mut child = cmd.spawn().map_err(|e| RunnerError::Spawn(e.to_string()))?;
        let pgid = child.id() as i32;
        registry.register(pgid);
        // Capture to completion, then wait. (Reads the piped handles; for the
        // streaming path the engine still forwards deltas via the stream parser
        // over the returned stdout — same as the .output() path.)
        use std::io::Read;
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        if let Some(mut o) = child.stdout.take() { let _ = o.read_to_end(&mut stdout); }
        if let Some(mut e) = child.stderr.take() { let _ = e.read_to_end(&mut stderr); }
        let status = child.wait().map_err(|e| RunnerError::Spawn(e.to_string()));
        registry.deregister(pgid);
        let status = status?;
        runners::claude_cli::interpret_runner_output(stdout, stderr, status.success())
    })
}

/// Build a worker `ClaudeCliRunner` wired to the killable spawner.
pub(crate) fn build_killable_worker_runner(
    registry: Arc<ProcessRegistry>,
) -> std::sync::Arc<dyn runners::output::Runner> {
    std::sync::Arc::new(runners::claude_cli::ClaudeCliRunner::with_spawner(
        killable_spawn(&registry),
    ))
}

/// Same for the chat runner (capture-to-completion via the chat SpawnFn).
pub(crate) fn killable_chat_spawn(
    registry: &Arc<ProcessRegistry>,
) -> llm_chat::claude_cli::SpawnFn {
    let registry = registry.clone();
    Box::new(move |args: &[String], cwd: Option<&str>| {
        use llm_chat::chat::ChatError;
        use std::process::Stdio;
        let mut cmd = std::process::Command::new(runners::command::CLAUDE_BIN);
        cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());
        if let Some(dir) = cwd { cmd.current_dir(dir); }
        #[cfg(unix)]
        { use std::os::unix::process::CommandExt; cmd.process_group(0); }
        let mut child = cmd.spawn().map_err(|e| ChatError::Spawn(e.to_string()))?;
        let pgid = child.id() as i32;
        registry.register(pgid);
        use std::io::Read;
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        if let Some(mut o) = child.stdout.take() { let _ = o.read_to_end(&mut stdout); }
        if let Some(mut e) = child.stderr.take() { let _ = e.read_to_end(&mut stderr); }
        let status = child.wait().map_err(|e| ChatError::Spawn(e.to_string()));
        registry.deregister(pgid);
        let status = status?;
        llm_chat::claude_cli::interpret_chat_output(stdout, stderr, status.success())
    })
}
```

> Confirm `runners::command::CLAUDE_BIN` is `pub` (it is `pub` per `command.rs` use in the runner). If not exported at crate root, reference it via the existing path used by `runners::claude_cli`. The chat runner factory mirrors `build_killable_worker_runner`.

- [ ] **Step 4: Build + register the `ProcessRegistry` at the root and thread it into the runners**

In `src-tauri/app/src/lib.rs`, near the top of the `setup` async block (after the pool/migrations, before building runtime state), add:

```rust
                // Live child process-group registry (LF20): the killable spawner
                // registers each `claude` group; the exit + brake triggers below
                // kill them all. Shared by the worker + chat runners.
                let process_registry = Arc::new(crate::process_registry::ProcessRegistry::new());
```

- The **worker** runners are built per-team in `pipeline_activator.rs` `runner_for_team`/`runner_for` (it constructs `ClaudeCliRunner::new()` for the `ClaudeCli` kind at `:411` and the fallback at `:207`). Thread the registry into the activator (`WorkerDeps`) and replace those two `ClaudeCliRunner::new()` constructions with `build_killable_worker_runner(self.deps.process_registry.clone())`. Add a `process_registry: Arc<ProcessRegistry>` field to `WorkerDeps` and pass `process_registry.clone()` from `lib.rs:1433`.

  > `runner_for` is a free fn taking a `RunnerConfig`; pass the registry as an extra arg, or move the claude-cli arm to call a closure provided by the activator. The minimal change: have `runner_for_team` build the killable runner directly (it already has `&self`) and keep `runner_for` only for the anthropic-api arm. Read `:186`–`:210` and `:404`–`:420` and pick the smaller diff; the anthropic-api runner is NOT subprocess-based and needs no registry.

- The **chat** runner is built at `lib.rs:1376` via `chat_runner_for`. For the CLI arm (`pipeline_activator.rs:438`, `ClaudeChatRunner::new()`), build it with `ClaudeChatRunner::with_spawner(killable_chat_spawn(&process_registry))` instead. Thread the registry into `chat_runner_for`.

- [ ] **Step 5: Add the exit kill trigger**

In `src-tauri/app/src/lib.rs`, the chain ends at `.run(tauri::generate_context!())` (`:1542`). The registry is created inside the `setup` closure, so it must be made available to the `run` callback. Build the registry BEFORE `.setup(...)` (move its construction up to where the builder is assembled, or clone it into an outer `let`), then change the tail:

```rust
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(move |_app, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                // LF20: kill every in-flight `claude` process group on quit so no
                // orphan keeps mutating a worktree after the app is gone.
                process_registry_for_exit.kill_all();
            }
        });
```

> `process_registry_for_exit` is a clone of the registry captured before `.setup`. Because `setup` runs inside `block_on` and manages state on the `handle`, the cleanest pattern is: construct `let process_registry = Arc::new(ProcessRegistry::new());` ABOVE the `tauri::Builder::default()` line, clone it into the setup closure (move-captured) for the spawners, and clone it again (`let process_registry_for_exit = process_registry.clone();`) for the run callback. Read `:1138`–`:1222` to place the construction before the builder.

- [ ] **Step 6: Add the brake-on kill triggers (3 sites)**

Each brake-on site at the root must call `kill_all()` AFTER setting the brake. The runtime `Brake` stays registry-unaware.

1. **RootDispatcher `brake_on`** (`lib.rs:1067`–`1073`): the `RootDispatcher` struct must hold the registry. Add a `process_registry: Arc<ProcessRegistry>` field to `RootDispatcher` (constructed at `:1361`) and after `s.set_on(...)` (`:1070`) add `self.process_registry.kill_all();`.

2. **Auto-meter `SetOn`** (`lib.rs:1473`): the sweep task already clones what it needs. Clone the registry into the spawned task (`let process_registry = process_registry.clone();` alongside the other clones at `:1456`–`:1460`) and change the `SetOn` arm to:
   ```rust
   BrakeDecision::SetOn(reason) => { brake.set_on(reason); process_registry.kill_all(); let _ = handle.emit(crate::events::USAGE_CHANGED, ()); }
   ```

3. **Frontend `runtime::api::brake_on`** (`runtime/src/api.rs:685`): the runtime crate must NOT know about the registry. Per the spec, route this through a root wrapper. The cleanest: the frontend `brake_on` Tauri command is registered in the `invoke_handler` (`lib.rs:1527`). Replace `runtime::api::brake_on` in that list with a thin `app`-crate command that sets the brake AND calls `kill_all`:
   ```rust
   #[tauri::command(rename_all = "snake_case")]
   fn brake_on(
       runtime: tauri::State<'_, Arc<RuntimeState>>,
       registry: tauri::State<'_, Arc<crate::process_registry::ProcessRegistry>>,
       reason: Option<String>,
   ) -> runtime::brake::BrakeState {
       runtime.brake.set_on(reason.unwrap_or_else(|| "manual".into()));
       registry.kill_all();
       runtime.brake.state()
   }
   ```
   `handle.manage(process_registry.clone());` so the command can resolve the registry State. Update `tauri::generate_handler![ ... ]` to use the local `brake_on` instead of `runtime::api::brake_on`.

- [ ] **Step 7: Verify it builds + the focused test passes**

Run: `cd src-tauri && cargo build -p agent-bus-app && cargo test -p agent-bus-app`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add src-tauri/app/src/lib.rs src-tauri/app/src/pipeline_activator.rs src-tauri/app/src/process_registry.rs
git commit -m "feat(app): wire killable spawners + exit/brake kill_all triggers (LF20)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 11: `read_artifact` reads from the absolute app-owned base

`read_artifact` (`workspace/src/api.rs:221`) currently resolves under `project.root_path` via `resolve_under_root` (`:199`), which REJECTS absolute paths. Now that artifacts live at the absolute app-data base, the agent's `ARTIFACT:` lines (and the engine's stored `parent_artifact`) are absolute paths under `<app_data>/projects/<id>/artifacts`. `read_artifact` must read those.

**Files:**
- Modify: `src-tauri/workspace/src/api.rs` (`read_artifact` `:220`; possibly a new resolver)

- [ ] **Step 1: Write the failing test**

Add to the `workspace/src/api.rs` test module (read the existing test scaffolding first to match the `WorkspaceState`/`ProjectStore` test setup). The behavior under test: given an absolute path under the project's app-data artifact base, `read_artifact` returns its contents; a path OUTSIDE the base is rejected.

```rust
    #[test]
    fn artifact_base_for_project_is_app_data_projects_id_artifacts() {
        let base = super::artifact_base_for(std::path::Path::new("/data"), "proj-7");
        assert_eq!(base, std::path::PathBuf::from("/data/projects/proj-7/artifacts"));
    }

    #[test]
    fn resolve_under_base_accepts_absolute_inside_and_rejects_outside() {
        let base = std::path::PathBuf::from("/data/projects/p/artifacts");
        let ok = super::resolve_under_base(&base, "/data/projects/p/artifacts/spec/k-v1.md").unwrap();
        assert_eq!(ok, std::path::PathBuf::from("/data/projects/p/artifacts/spec/k-v1.md"));
        assert!(super::resolve_under_base(&base, "/etc/passwd").is_err());
        // a relative path is still accepted, resolved under the base
        let rel = super::resolve_under_base(&base, "spec/k-v1.md").unwrap();
        assert_eq!(rel, std::path::PathBuf::from("/data/projects/p/artifacts/spec/k-v1.md"));
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd src-tauri && cargo test -p workspace artifact_base_for resolve_under_base`
Expected: FAIL to compile — fns not found.

- [ ] **Step 3: Implement `artifact_base_for` + `resolve_under_base`**

In `src-tauri/workspace/src/api.rs`, add near `resolve_under_root` (`:196`):

```rust
/// The absolute, app-owned artifact base for `project_id` under the Tauri
/// app-data dir: `<app_data>/projects/<project_id>/artifacts` (LF26). The single
/// place this layout is spelled so the engine, the reader, and the delete cascade
/// agree. PURE.
pub fn artifact_base_for(app_data: &Path, project_id: &str) -> PathBuf {
    app_data.join("projects").join(project_id).join("artifacts")
}

/// Resolve `path` against the artifact `base`, allowing an ABSOLUTE path only
/// when it is inside `base`, or a relative path resolved under `base`. Rejects
/// anything that escapes the base (the new escape guard for the app-owned base,
/// replacing `resolve_under_root`'s reject-all-absolutes for artifacts). PURE.
pub fn resolve_under_base(base: &Path, path: &str) -> Result<PathBuf, String> {
    let p = Path::new(path);
    let candidate = if p.is_absolute() {
        p.to_path_buf()
    } else {
        let mut out = base.to_path_buf();
        for comp in p.components() {
            match comp {
                Component::Normal(c) => out.push(c),
                Component::CurDir => {}
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    return Err("artifact path may not escape the artifact base".into());
                }
            }
        }
        out
    };
    if !candidate.starts_with(base) {
        return Err("artifact path is outside the project artifact base".into());
    }
    Ok(candidate)
}
```

- [ ] **Step 4: Rewrite `read_artifact` to use the base**

`read_artifact` needs the app-data dir. The command has `tauri::State<'_, WorkspaceState>` only. Resolve the app-data dir from the `AppHandle` — add `app: tauri::AppHandle` as a command param (Tauri injects it). Rewrite (`:220`):

```rust
#[tauri::command(rename_all = "snake_case")]
pub async fn read_artifact(
    app: tauri::AppHandle,
    state: tauri::State<'_, WorkspaceState>,
    project_id: String,
    path: String,
) -> Result<String, String> {
    use tauri::Manager;
    // Confirm the project exists (keeps the not-found behaviour).
    let _ = state
        .store
        .get(&ProjectId(project_id.clone()))
        .await
        .map_err(|e| e.to_string())?;
    let app_data = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let base = artifact_base_for(&app_data, &project_id);
    let full = resolve_under_base(&base, &path)?;
    std::fs::read_to_string(&full).map_err(|e| e.to_string())
}
```

> Read the top of `workspace/src/api.rs` for the `Manager`/`tauri::AppHandle` imports already in use; add `use tauri::Manager;` if not present. `Path`/`PathBuf`/`Component` are already imported (used by `resolve_under_root`).

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test -p workspace`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/workspace/src/api.rs
git commit -m "feat(workspace): read_artifact resolves under app-owned artifact base (LF26)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 12: Pass the resolved `artifact_base` into `EngineContext` at the composition root

The engine field added in Task 8 must be populated by the activator's `ctx_builder` (`pipeline_activator.rs:325`–`:366`) from the Tauri app-data dir.

**Files:**
- Modify: `src-tauri/app/src/pipeline_activator.rs` (`ctx_builder`; the activator constructor / `WorkerDeps`)
- Modify: `src-tauri/app/src/lib.rs` (pass the app-data dir into the activator)

- [ ] **Step 1: Thread the app-data dir into the activator**

`load_active` / the activator currently carry `project_root` + `project_target_repo`. The `data_dir` is available at `lib.rs:1225`. Add an `app_data: PathBuf` to `WorkerDeps` (or to the `PipelineActivator`), set from `data_dir.clone()` at the activator construction (`lib.rs:1427`–`:1445`).

- [ ] **Step 2: Set `artifact_base` in the `ctx_builder` struct literal**

In `pipeline_activator.rs`, the `ctx_builder` captures the values it needs (`:330`–`:347`). Add:

```rust
        let app_data = self.deps.app_data.clone();
        let project_id_for_base = active.project_id.clone();
```

and in the `EngineContext { ... }` literal (`:348`), add (using the shared layout fn so it matches `read_artifact`):

```rust
            artifact_base: workspace::api::artifact_base_for(&app_data, &project_id_for_base),
```

> `workspace::api::artifact_base_for` is the `pub` fn from Task 11 — single source of truth for the layout. Confirm `app` depends on `workspace` (it does, per `app/Cargo.toml`).

- [ ] **Step 3: Verify it builds + runtime tests still pass**

Run: `cd src-tauri && cargo build -p agent-bus-app && cargo test -p runtime`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/app/src/pipeline_activator.rs src-tauri/app/src/lib.rs
git commit -m "feat(app): populate EngineContext.artifact_base from app-data dir (LF26)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 13: Project-delete cascade removes the app-owned artifacts dir

The delete cascade (`workspace/src/api.rs:workspace_remove_project`, `:157`) removes project files but never the app-data artifacts dir. It must remove `<app_data>/projects/<id>/artifacts` (actually the whole `<app_data>/projects/<id>`), and NEVER touch `target_repo`.

**Files:**
- Modify: `src-tauri/workspace/src/api.rs` (`workspace_remove_project`)

- [ ] **Step 1: Write the failing test**

Add a test that the cascade removes the app-data project dir. The cleanest pure unit is a small helper `app_data_project_dir(app_data, id)` + a removal that the command calls; assert the helper path and that removing a created dir succeeds:

```rust
    #[test]
    fn app_data_project_dir_is_under_projects_id() {
        let d = super::app_data_project_dir(std::path::Path::new("/data"), "p9");
        assert_eq!(d, std::path::PathBuf::from("/data/projects/p9"));
    }

    #[test]
    fn remove_app_data_project_dir_is_best_effort_and_removes_the_tree() {
        let tmp = std::env::temp_dir().join(format!("abtest-{}", uuid::Uuid::new_v4()));
        let proj = super::app_data_project_dir(&tmp, "p1");
        std::fs::create_dir_all(proj.join("artifacts/spec")).unwrap();
        std::fs::write(proj.join("artifacts/spec/k-v1.md"), b"x").unwrap();
        super::remove_app_data_project_dir(&tmp, "p1");
        assert!(!proj.exists());
        // calling again on a missing dir does not panic
        super::remove_app_data_project_dir(&tmp, "p1");
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd src-tauri && cargo test -p workspace app_data_project_dir remove_app_data_project_dir`
Expected: FAIL to compile — fns not found.

- [ ] **Step 3: Implement the helpers + call them in the cascade**

In `src-tauri/workspace/src/api.rs` add:

```rust
/// The app-owned data dir for a project: `<app_data>/projects/<project_id>`
/// (parent of the artifacts dir). Removed on project delete. PURE.
pub fn app_data_project_dir(app_data: &Path, project_id: &str) -> PathBuf {
    app_data.join("projects").join(project_id)
}

/// Best-effort removal of the app-owned project data dir (artifacts live here).
/// Never touches `target_repo`. A missing dir is fine.
pub fn remove_app_data_project_dir(app_data: &Path, project_id: &str) {
    let dir = app_data_project_dir(app_data, project_id);
    if let Err(e) = std::fs::remove_dir_all(&dir) {
        if e.kind() != std::io::ErrorKind::NotFound {
            eprintln!("workspace_remove_project: app-data cleanup for {project_id} failed: {e}");
        }
    }
}
```

In `workspace_remove_project`, add `app: tauri::AppHandle` as a param, and after the existing on-disk cleanup (`:191`) add:

```rust
    // App-owned artifact data dir cleanup (LF26): remove
    // <app_data>/projects/<id> (artifacts live there). Best-effort; never
    // touches target_repo.
    use tauri::Manager;
    if let Ok(app_data) = app.path().app_data_dir() {
        remove_app_data_project_dir(&app_data, &project_id.0);
    }
```

> `project_id` is the `ProjectId` newtype; `.0` is the inner `String`. Confirm by reading the `ProjectId` def used at `:171`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test -p workspace`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/workspace/src/api.rs
git commit -m "feat(workspace): project-delete removes app-owned artifacts dir (LF26)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 14: Final verification gates

**Files:** none (verification only).

- [ ] **Step 1: Full workspace test suite**

Run: `cd src-tauri && cargo test`
Expected: PASS (all crates). The `process_registry` unix kill tests run real `sh`/`sleep` children; the SIGTERM-ignoring test takes ~2.5s.

- [ ] **Step 2: Clippy with warnings as errors**

Run: `cd src-tauri && cargo clippy --all-targets -- -D warnings`
Expected: clean (no warnings). Watch for `clippy::len_without_is_empty` on `ProcessRegistry` (Task 1 added `is_empty`, so it is satisfied).

- [ ] **Step 3: Release-shaped build**

Run: `cd src-tauri && cargo build`
Expected: PASS.

- [ ] **Step 4: TS gates (only if any TS changed)**

This plan changes NO TypeScript. If a task incidentally touched TS, run:
Run: `npx vitest run && npx tsc --noEmit`
Expected: PASS. Otherwise skip.

- [ ] **Step 5: Commit any final lint fixes**

```bash
git add -A src-tauri
git commit -m "chore: lint + verification fixes for lifecycle exit/resume

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Self-review against the spec

- **Decision 1 (killable spawns via `with_spawner`, kill the group, no process types across traits):** Tasks 1, 2, 3, 4, 10. The registry lives in the `app` crate; the `SpawnFn` type aliases (not traits) carry an extra `cwd` arg only.
- **Decision 2 (two triggers: exit + Stop):** Task 10 (exit handler + 3 brake-on sites).
- **Decision 3 (SIGTERM → grace → SIGKILL):** Task 2.
- **Decision 4 (clean resume — reconcile occupancy after orphan release):** Tasks 6, 7.
- **Decision 5 (platform: unix; non-unix logged no-op):** Task 2 `#[cfg]` arms.
- **Decision 6 / LF26 (worker cwd + absolute app-owned artifact base, scope-write + add-dir, read from same base, delete cascade):** Tasks 8, 9, 11, 12, 13. `current_dir` set in the spawner (Tasks 3, 4) from `InvocationRequest.working_dir` (Task 9).
- **Spec item 5 (Brake stays registry-unaware; root wrapper / on-set hook):** Task 10 routes the frontend `brake_on` through an `app`-crate command wrapper; the RootDispatcher + auto-meter call `kill_all` after `set_on`. Runtime `Brake` is untouched.
- **Tauri event names contain no dots:** No new events are added; existing `crate::events::*` constants are reused.

## Deferred follow-ups (beyond this spec — NO tasks written)

These were explicitly out of scope; noted only so they are not lost:
- kill/spawn race latch (a child spawned during `kill_all` may be missed)
- pgid-reuse safety (a recycled pgid could be signalled)
- async-offload of the blocking `kill_all` grace sleep (currently blocks the caller ~2.5s)
- crash-orphan reaping on boot (kill stale OS processes left by a hard crash)
- chat-exempt-from-kill policy
- brake persistence across reboot
- per-reason auto-meter kill policy (currently every `SetOn` kills)
- killed-task-non-terminal-state handling (a killed invocation re-runs from scratch via re-claim — acceptable per spec non-goals)
