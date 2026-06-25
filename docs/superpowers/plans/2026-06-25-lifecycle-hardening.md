# Lifecycle Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the eight deferred hardening gaps on top of the shipped lifecycle exit/resume feature (tag `plan-lifecycle`): make Stop/exit kills race-free, PID-reuse-safe, non-blocking on the async runtime, scoped (chat survives Stop), crash-orphan-reaping on boot, brake-state persistent + per-reason restored, auto-meter soft-brake (no kill, no re-run loop), and killed-task re-runnable.

**Architecture:** All eight items are **composition-root** concerns wired in `src-tauri/app` — the `runtime` crate's `Brake` / `worker` / `engine` stay registry-and-persistence-unaware. The diff reshapes one type (`ProcessRegistry`) so registration is latch-guarded and entries are scoped (`Worker` vs `Chat`) and hold the owned `Child` handle (kill via handle ⇒ no recycled-pgid signal); adds a persisted `live_processes` table (boot-time orphan reaping) and a persisted `brake_state` row (per-reason restore); and threads brake-reason awareness into the kill triggers and the worker failure path so an auto-meter brake never kills and a manual-Stop kill leaves the task re-runnable. Sequencing matters: LH1+LH2 reshape the registry type first, LH5 adds the scope tag onto that shape, LH4 layers persistence onto the same spawner, LH6 adds the brake row, and LH7+LH8 are a paired reason-aware kill/recovery policy.

**Tech Stack:** Tauri 2 + Rust (Cargo workspace under `src-tauri/`: crates `runners`, `llm_chat`, `runtime`, `usage_telemetry`, `app`). macOS/unix. `libc` for signals. `sqlx`/SQLite for persistence. Tauri event names contain NO dots.

**Conventions reminder:** strict TDD (failing test first, run it red, minimal impl, run green, commit). Real-process tests (`sh -c 'sleep …'` in its own process group) for the latch / reuse / reaping mechanics; structural tests for root wiring and reason policy (no live `claude` — use `FakeRunner` / `FakeSpawner`). The `runtime` crate stays Tauri-free AND registry/persistence-unaware — process control and durable brake/process rows are wired in `app`. Append-only migrations: never edit `001`–`012`; add `013_*`. Register every new migration in BOTH `run_migrations`' `MIGRATIONS` array (`app/src/lib.rs:39`) AND the `tauri_plugin_sql` `migrations` vec (`app/src/lib.rs` ends at `:1255`), and bump the `user_version` idempotency test (`app/src/lib.rs:1873`). Run Rust gates from `src-tauri/`: `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo build`.

---

## File Structure

**Reshaped:**
- `src-tauri/app/src/process_registry.rs` — `ProcessRegistry` gains: a `killing` latch + `Scope` tag + owned-`Child` entries (LH1/LH2/LH5); `register` returns a guard/decision; `kill_workers()` vs `kill_all()`; `kill_all_async` offload helper (LH3); a persisted-record hook on register/deregister (LH4); a boot `reap_orphans` reading the persisted table.

**New:**
- `src-tauri/app/migrations/013_lifecycle_hardening.sql` — `live_processes` table (LH4) + `brake_state` single-row table (LH6).
- `src-tauri/app/src/process_records.rs` — `LiveProcessStore` (persist `(run_id, task_id, pgid, started_ts)`; list/clear) + boot `reap_orphans` (LH4). Small, focused, app-crate (root concern).
- `src-tauri/app/src/brake_persist.rs` — `BrakeStore` (persist `(on, reason, ts)`; load) + `is_manual_reason`/`persists_across_reboot` discriminator (LH6/LH7).

**Modified (wiring):**
- `src-tauri/app/src/pipeline_activator.rs` — worker spawner registers `Scope::Worker` + persists a live-process record; chat registers `Scope::Chat`; the transformer worker loop (`:314`) consults the brake before settling a failed step (LH8).
- `src-tauri/app/src/lib.rs` — migration lists; boot reap-before-recovery ordering; persisted brake load + per-reason restore; reason-aware kill at the three brake-on sites; `kill_workers` on Stop, `kill_all` on exit; async offload at the async Stop sites; idempotency test bump.
- `src-tauri/runtime/src/engine.rs` — `transform_once` failure path becomes brake-aware (retain claim, no attempts-bump, non-terminal) — OR a new `EngineError::Interrupted` short-circuit; decided in LH8.

---

## Sequencing / interdependencies

1. **LH1 + LH2 first** — they reshape the `ProcessRegistry` stored type (latch + owned `Child`). Everything else builds on the new shape.
2. **LH5** adds the `Scope` tag to the LH1/LH2 shape and splits `kill_all`/`kill_workers`.
3. **LH3** offloads the (now-final) kill primitive at the async Stop sites.
4. **LH4** layers the persisted live-process record onto the (now-final) spawner + adds boot reaping ordered before `release_orphaned_running`.
5. **LH6** adds the persisted brake row + per-reason boot restore (independent of the registry; can proceed in parallel after LH1/LH2 but is written here for a coherent diff).
6. **LH7 + LH8 paired last** — reason-aware kill (auto-meter soft-brake) + killed-task-stays-re-runnable; LH8 guards the precondition LH7's manual path relies on.

Each task below is independently committable and green.

---

## Section LH1+LH2 — Reshape `ProcessRegistry`: kill latch + owned `Child` (no recycled-pgid signal)

The current registry stores bare `i32` pgids in a `HashSet` and `kill_all` snapshots-then-signals. Two defects: (a) a worker that passes the brake gate can `.spawn()` + `register()` *after* the snapshot and escape (LH1 spawn-after-snapshot / register-after-spawn TOCTOU); (b) `kill_all` can signal a pgid whose `Child` was already `.wait()`ed and recycled by the OS (LH2 PID-reuse).

The fix is one new stored type: an entry keyed by pgid that holds the **owned `Child`** behind a per-entry `Arc<Mutex<Option<Child>>>`, plus a registry-wide `killing` latch guarded by the **same** `Mutex` as the entry map. `register` takes the lock, and if the latch is set returns a `RegisterDecision::KillImmediately` (the spawner self-kills the child it just spawned); otherwise it inserts and returns `RegisterDecision::Registered`. The reaping `.wait()` path takes the `Child` *out* of the entry under the lock before waiting, so `kill_all` never holds a reaped pgid. `kill_all` signals only entries whose `Child` is still present (not yet reaped) — by construction no recycled pid is reachable.

### Task LH1a: Add the `killing` latch + `RegisterDecision` to `ProcessRegistry`

**Files:**
- Modify: `src-tauri/app/src/process_registry.rs` (struct `ProcessRegistry` at `:13`; `register` at `:27`; `kill_all` at `:62`)
- Test: same file (`#[cfg(test)] mod tests`)

- [ ] **Step 1: Write the failing test.** Add to the `tests` module in `process_registry.rs`:

```rust
    #[test]
    fn register_under_a_set_latch_returns_kill_immediately() {
        let reg = ProcessRegistry::new();
        reg.begin_killing(); // latch on
        // A spawner that registers while the latch is set is told to self-kill.
        assert!(matches!(reg.register_pgid(4321), RegisterDecision::KillImmediately));
        assert_eq!(reg.len(), 0, "a latched register must not retain the pgid");
    }

    #[test]
    fn register_clears_to_registered_after_latch_off() {
        let reg = ProcessRegistry::new();
        reg.begin_killing();
        reg.end_killing(); // latch off (brake-off / resume)
        assert!(matches!(reg.register_pgid(99), RegisterDecision::Registered));
        assert_eq!(reg.len(), 1);
    }
```

- [ ] **Step 2: Run it red.** Run: `cd src-tauri && cargo test -p agent-bus-app register_under_a_set_latch -- --nocapture`
  Expected: FAIL (`cannot find value RegisterDecision` / `no method begin_killing`).

- [ ] **Step 3: Implement the latch.** In `process_registry.rs`, add the decision enum and latch. Add a `killing: bool` field guarded inside the existing lock by widening the locked value to a small struct, OR add a separate `AtomicBool` *plus* the requirement that `register`/`kill_all` both take the entry-map `Mutex` (so the latch read/write is ordered with the map). Use the map-lock approach to keep one critical section:

```rust
/// What the spawner must do after attempting to register a freshly-spawned child.
pub enum RegisterDecision {
    /// The pgid was recorded; proceed normally (deregister on completion).
    Registered,
    /// A kill is in progress (latch set): the spawner must immediately kill the
    /// child it just spawned and NOT track it. Closes the spawn-after-snapshot
    /// and register-after-spawn TOCTOU windows.
    KillImmediately,
}

#[derive(Default)]
struct Inner {
    /// pgid -> owned child handle (LH2: held until reaped, so kill_all never
    /// signals a recycled pgid). `None` once the reaper took it out to `.wait()`.
    groups: std::collections::HashMap<i32, ()>, // replaced by the Child map in LH2b
    /// Latch: while set, no new child may register (it self-kills instead).
    killing: bool,
}
```

Replace the `groups: Mutex<HashSet<i32>>` field with `inner: Mutex<Inner>`. Add:

```rust
    /// Set the killing latch (called at the start of kill_all / kill_workers).
    pub fn begin_killing(&self) {
        self.inner.lock().unwrap().killing = true;
    }

    /// Clear the latch — re-enable normal spawning (wire on brake-off / resume).
    pub fn end_killing(&self) {
        self.inner.lock().unwrap().killing = false;
    }

    /// Register a freshly-spawned pgid under the lock. If the latch is set the
    /// spawner is told to self-kill its child instead of tracking it.
    pub fn register_pgid(&self, pgid: i32) -> RegisterDecision {
        let mut g = self.inner.lock().unwrap();
        if g.killing {
            return RegisterDecision::KillImmediately;
        }
        g.groups.insert(pgid, ());
        RegisterDecision::Registered
    }
```

Update `register`/`deregister`/`len`/`is_empty`/`snapshot`/`kill_all` to use `inner.lock().unwrap().groups`. Keep the existing public `register(&self, pgid)` as a thin wrapper that ignores the decision ONLY where back-compat is needed — but the spawner will move to `register_pgid`. (Resolve the exact field/type when LH2b lands the `Child` map; for LH1a keep `()` values so the test compiles.)

- [ ] **Step 4: Run it green.** Run: `cd src-tauri && cargo test -p agent-bus-app register_under_a_set_latch register_clears_to_registered -- --nocapture`
  Expected: PASS. Then `cargo test -p agent-bus-app process_registry` to confirm the existing register/deregister/idempotency/kill tests still pass after the field rename.

- [ ] **Step 5: Commit.**

```bash
cd src-tauri && git add app/src/process_registry.rs && \
  git commit -m "feat(lifecycle): add killing latch + RegisterDecision to ProcessRegistry (LH1)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

### Task LH1b: `kill_all` sets the latch first, then sweeps; latch held until cleared

**Files:**
- Modify: `src-tauri/app/src/process_registry.rs` (`kill_all` at `:62`)
- Test: same file

- [ ] **Step 1: Write the failing test.** Add:

```rust
    #[test]
    fn kill_all_leaves_the_latch_set_so_stragglers_self_kill() {
        let reg = ProcessRegistry::new();
        reg.kill_all(); // empty sweep, but the latch must now be set
        assert!(
            matches!(reg.register_pgid(7), RegisterDecision::KillImmediately),
            "after kill_all the latch is held until end_killing()"
        );
    }
```

- [ ] **Step 2: Run it red.** Run: `cd src-tauri && cargo test -p agent-bus-app kill_all_leaves_the_latch_set`
  Expected: FAIL (register returns `Registered`).

- [ ] **Step 3: Implement.** At the top of `kill_all` (and the soon-to-exist `kill_workers`), call `self.begin_killing()` *before* taking the snapshot. Do NOT clear it at the end of `kill_all` — the latch stays set until `end_killing()` is called explicitly (on brake-off / resume). Keep the existing drain-the-set behaviour at the end.

- [ ] **Step 4: Run it green.** Run: `cd src-tauri && cargo test -p agent-bus-app kill_all_leaves_the_latch_set` then `cargo test -p agent-bus-app process_registry`.
  Expected: PASS. (Note: the existing `kill_all_terminates_a_well_behaved_group…` tests construct a fresh registry per test so a held latch does not leak between tests.)

- [ ] **Step 5: Commit.**

```bash
cd src-tauri && git add app/src/process_registry.rs && \
  git commit -m "feat(lifecycle): kill_all sets the killing latch before sweeping (LH1)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

### Task LH1c: Spawner self-kills its child when registration is latched

**Files:**
- Modify: `src-tauri/app/src/process_registry.rs` (`killable_spawn` at `:119`, `killable_chat_spawn` at `:178`)
- Test: same file

- [ ] **Step 1: Write the failing test.** Add (real short-lived child; latch set before spawn):

```rust
    #[cfg(unix)]
    #[test]
    fn spawner_self_kills_when_latch_is_set_and_does_not_register() {
        use std::time::{Duration, Instant};
        let reg = Arc::new(ProcessRegistry::new());
        reg.begin_killing(); // a Stop already happened; this claim raced past the gate
        // Spawn a child that would live 30s if it escaped.
        let out = (super::killable_spawn(&reg))(
            &["sh".into(), "-c".into(), "sleep 30".into()],
            None,
        );
        // The spawner must have killed its own child and surfaced a failure.
        assert!(out.is_err(), "a latched spawn must not succeed");
        assert!(reg.is_empty(), "the self-killed child is never registered");
        // The child group must actually be dead (no escape).
        // (We cannot easily read the pgid here; rely on is_empty + the no-deadlock
        // timeout below. The dedicated escape test in LH2 asserts liveness.)
        let _ = (Instant::now(), Duration::from_secs(0));
    }
```

- [ ] **Step 2: Run it red.** Run: `cd src-tauri && cargo test -p agent-bus-app spawner_self_kills_when_latch_is_set`
  Expected: FAIL (the child is registered and the call returns Ok).

- [ ] **Step 3: Implement.** In `killable_spawn`, replace the `registry.register(pgid);` line (`:139`) with the decision flow. Spawn the child, capture `pgid`, then:

```rust
        let pgid = child.id() as i32;
        match registry.register_pgid(pgid) {
            RegisterDecision::Registered => {}
            RegisterDecision::KillImmediately => {
                // A Stop/exit kill is in progress; this claim raced past the
                // brake gate. Kill the child we just spawned (whole group) so it
                // cannot escape, then surface an interrupt-class failure.
                #[cfg(unix)]
                unsafe {
                    libc::kill(-pgid, libc::SIGKILL);
                }
                let _ = child.wait();
                return Err(RunnerError::Other("spawn aborted: kill in progress".into()));
            }
        }
```

Do the equivalent in `killable_chat_spawn` (return `ChatError::Spawn("spawn aborted: kill in progress".into())`). Import `RegisterDecision` via `use super::RegisterDecision;` or fully-qualify.

- [ ] **Step 4: Run it green.** Run: `cd src-tauri && cargo test -p agent-bus-app spawner_self_kills_when_latch_is_set killable_spawn`
  Expected: PASS (and the no-deadlock regressions still pass).

- [ ] **Step 5: Commit.**

```bash
cd src-tauri && git add app/src/process_registry.rs && \
  git commit -m "feat(lifecycle): spawner self-kills its child on a latched register (LH1)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

### Task LH2a: Hold the owned `Child` per entry; reaper removes it under the lock before `.wait()`

**Files:**
- Modify: `src-tauri/app/src/process_registry.rs` (the `Inner` struct from LH1a; `killable_spawn`/`killable_chat_spawn`)
- Test: same file

This makes "registered ⇒ not yet reaped" an invariant: `kill_all` only ever signals a pgid whose `Child` is still owned by the registry (not yet `.wait()`ed), so the OS cannot have recycled it.

- [ ] **Step 1: Write the failing test.** Add a test that races a real child's natural exit+reap against `kill_all` and asserts the registry never signals a reaped pgid. Use an instrumented kill seam — add a private `kill_targets` recording vector behind a test-only flag is heavy; instead assert the structural invariant directly:

```rust
    #[cfg(unix)]
    #[test]
    fn reaping_takes_the_child_out_before_wait_so_kill_all_skips_it() {
        // A child that exits immediately; the spawner reaps it. After the spawner
        // returns, the registry holds NO live Child for that pgid, so a later
        // kill_all has nothing to signal (the pgid is unreachable, not recycled).
        let reg = Arc::new(ProcessRegistry::new());
        let out = (super::killable_spawn(&reg))(
            &["sh".into(), "-c".into(), "printf done".into()],
            None,
        );
        assert_eq!(out.unwrap(), "done");
        assert!(reg.is_empty(), "reaped child removed from the registry");
        // kill_all over the now-empty registry is a no-op and signals nothing.
        reg.kill_all();
        assert!(reg.is_empty());
    }
```

- [ ] **Step 2: Run it red (compile-driven).** Run: `cd src-tauri && cargo test -p agent-bus-app reaping_takes_the_child_out`
  Expected: After changing the stored type to hold `Child`, the spawner must take the child out before `.wait()`; until implemented this fails to compile or leaves a stale entry.

- [ ] **Step 3: Implement.** Change `Inner.groups` to `HashMap<i32, Arc<Mutex<Option<std::process::Child>>>>`. Add:

```rust
    /// Register the owned Child under the lock (LH2). Returns the decision; on
    /// `Registered` the entry holds the Child so kill_all signals only live pids.
    pub(crate) fn register_child(&self, pgid: i32, child: std::process::Child)
        -> RegisterDecision
    {
        let mut g = self.inner.lock().unwrap();
        if g.killing {
            return RegisterDecision::KillImmediately;
        }
        g.groups.insert(pgid, Arc::new(Mutex::new(Some(child))));
        RegisterDecision::Registered
    }

    /// Take the owned Child out of the registry under the lock (the reaper calls
    /// this immediately before `.wait()`), so kill_all can never observe a pgid
    /// whose process was already reaped+recycled.
    pub(crate) fn take_child(&self, pgid: i32) -> Option<std::process::Child> {
        let mut g = self.inner.lock().unwrap();
        g.groups.remove(&pgid).and_then(|c| c.lock().unwrap().take())
    }
```

In `killable_spawn`: after `cmd.spawn()`, do NOT keep `child` directly for the drain — the drain reads `child.stdout`/`stderr` which need `&mut child`. Restructure: take the stdout/stderr handles off `child` *first* (as today), then `register_child(pgid, child)` moving the (now-pipe-drained-handles-removed) child in. To `.wait()`, call `let mut child = registry.take_child(pgid).expect("our own entry");` then `child.wait()`. On the `KillImmediately` branch, SIGKILL `-pgid` and `child.wait()` the locally-held child (you still own it — registration moved nothing). Mirror in `killable_chat_spawn`. `kill_all` signals each live pgid via `libc::kill(-pgid, …)` using the map keys (the `Child` is still owned), then drains.

- [ ] **Step 4: Run it green.** Run: `cd src-tauri && cargo test -p agent-bus-app process_registry` and `cargo test -p agent-bus-app killable`
  Expected: PASS (all existing spawn/drain/kill/no-deadlock tests + the new invariant test).

- [ ] **Step 5: Commit.**

```bash
cd src-tauri && git add app/src/process_registry.rs && \
  git commit -m "feat(lifecycle): registry holds owned Child; reaper removes before wait (LH2)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

### Task LH2b: Concurrency regression — race register vs kill_all, assert no escape and no reaped-pgid signal

**Files:**
- Test only: `src-tauri/app/src/process_registry.rs`

- [ ] **Step 1: Write the failing/guard test.** A real `sleep`-in-its-own-group child, raced against `kill_all`. Whichever order, the child must end dead and the registry empty:

```rust
    #[cfg(unix)]
    #[test]
    fn register_racing_kill_all_never_lets_a_child_escape() {
        use std::time::{Duration, Instant};
        use std::os::unix::process::CommandExt;
        use std::process::Command;
        let reg = Arc::new(ProcessRegistry::new());
        // Spawn a long-lived child OURSELVES in its own group, then register it.
        let child = Command::new("sh").arg("-c").arg("sleep 30 & wait")
            .process_group(0).spawn().expect("spawn");
        let pgid = child.id() as i32;
        let dec = reg.register_child(pgid, child);
        assert!(matches!(dec, RegisterDecision::Registered));
        reg.kill_all();
        assert!(reg.is_empty(), "registry drained");
        let alive = |p: i32| unsafe { libc::kill(-p, 0) == 0 };
        let deadline = Instant::now() + Duration::from_secs(3);
        while alive(pgid) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(!alive(pgid), "the registered child must be dead after kill_all");
    }
```

- [ ] **Step 2: Run it red, then green.** Run: `cd src-tauri && cargo test -p agent-bus-app register_racing_kill_all`
  Expected: PASS with the LH2a implementation (kill_all signals the still-owned Child's group).

- [ ] **Step 3: Commit.**

```bash
cd src-tauri && git add app/src/process_registry.rs && \
  git commit -m "test(lifecycle): register-vs-kill_all race leaves no escaped child (LH2)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

## Section LH5 — Scope entries (Worker vs Chat): Stop kills workers, exit kills both

The chat runner and worker runner register into the **same** registry today. `kill_all` kills both — so a manual Stop (or worse, an auto-meter trip) kills the user's in-flight chat mid-stream. Add a `Scope` tag per entry; `kill_workers()` kills only `Worker` entries (chat survives); `kill_all()` (exit) kills both.

### Task LH5a: Tag registry entries with `Scope`; add `kill_workers()`

**Files:**
- Modify: `src-tauri/app/src/process_registry.rs` (`Inner`, `register_child`, add `kill_workers`)
- Test: same file

- [ ] **Step 1: Write the failing test.** Real children, one Worker-scoped one Chat-scoped:

```rust
    #[cfg(unix)]
    #[test]
    fn kill_workers_spares_chat_kill_all_takes_both() {
        use std::time::{Duration, Instant};
        use std::os::unix::process::CommandExt;
        use std::process::Command;
        let alive = |p: i32| unsafe { libc::kill(-p, 0) == 0 };
        let spawn = || {
            let c = Command::new("sh").arg("-c").arg("sleep 30 & wait")
                .process_group(0).spawn().expect("spawn");
            (c.id() as i32, c)
        };
        let reg = Arc::new(ProcessRegistry::new());
        let (wpgid, wchild) = spawn();
        let (cpgid, cchild) = spawn();
        reg.register_child_scoped(wpgid, wchild, Scope::Worker);
        reg.register_child_scoped(cpgid, cchild, Scope::Chat);

        reg.kill_workers();
        let deadline = Instant::now() + Duration::from_secs(3);
        while alive(wpgid) && Instant::now() < deadline { std::thread::sleep(Duration::from_millis(50)); }
        assert!(!alive(wpgid), "worker child dead after kill_workers");
        assert!(alive(cpgid), "chat child SURVIVES kill_workers");
        assert_eq!(reg.len(), 1, "only the chat entry remains");

        reg.kill_all();
        let deadline = Instant::now() + Duration::from_secs(3);
        while alive(cpgid) && Instant::now() < deadline { std::thread::sleep(Duration::from_millis(50)); }
        assert!(!alive(cpgid), "chat child dead after kill_all (exit)");
        assert!(reg.is_empty());
    }
```

- [ ] **Step 2: Run it red.** Run: `cd src-tauri && cargo test -p agent-bus-app kill_workers_spares_chat`
  Expected: FAIL (`Scope` / `register_child_scoped` / `kill_workers` missing).

- [ ] **Step 3: Implement.** Add `#[derive(Clone, Copy, PartialEq, Eq)] pub enum Scope { Worker, Chat }`. Change the map value to carry the scope: `HashMap<i32, (Scope, Arc<Mutex<Option<Child>>>)>`. Make `register_child` delegate to a `register_child_scoped(pgid, child, scope)`; keep `register_child` as `register_child_scoped(.., Scope::Worker)` for back-compat OR update the two call sites directly. Add `kill_workers()` = the existing SIGTERM→grace→SIGKILL loop but only over entries whose scope is `Worker` (set the latch first — but note: the latch is global; a held latch from `kill_workers` would also self-kill a racing *chat* spawn. To preserve "chat survives a Stop", make the latch scope-aware: store `killing: Option<Scope-or-All>` and have `register_child_scoped` self-kill only when the latch covers its scope). Concretely:

```rust
    // Latch granularity: None = open; Some(KillScope::Workers) = workers self-kill
    // but chat may still spawn; Some(KillScope::All) = nothing spawns (exit).
    enum KillScope { Workers, All }
    // Inner.killing: Option<KillScope>
```

`register_child_scoped` returns `KillImmediately` when `killing == Some(All)`, or when `killing == Some(Workers) && scope == Worker`. `kill_workers` sets `Some(Workers)`; `kill_all` sets `Some(All)`. Update `register_pgid` (LH1) accordingly or fold it into `register_child_scoped`.

- [ ] **Step 4: Run it green.** Run: `cd src-tauri && cargo test -p agent-bus-app kill_workers_spares_chat process_registry`
  Expected: PASS. Re-run the LH1 latch tests — adjust them to the new `Option<KillScope>` latch (`begin_killing` becomes `begin_killing(KillScope::All)` or keep a no-arg `begin_killing()` defaulting to `All` for the LH1 tests).

- [ ] **Step 5: Commit.**

```bash
cd src-tauri && git add app/src/process_registry.rs && \
  git commit -m "feat(lifecycle): scope registry entries; kill_workers spares chat (LH5)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

### Task LH5b: Worker spawner registers Worker-scoped; chat spawner registers Chat-scoped

**Files:**
- Modify: `src-tauri/app/src/process_registry.rs` (`killable_spawn` → `Scope::Worker`; `killable_chat_spawn` → `Scope::Chat`)
- Test: same file (assert the chat spawner registers as Chat)

- [ ] **Step 1: Write the failing test.** Drive the chat spawner through the fake-`claude`-on-PATH idiom already used by `killable_chat_spawn_does_not_deadlock_on_large_stderr`, but with the latch set to `Workers` only, and assert the chat invocation is NOT self-killed (it completes):

```rust
    #[cfg(unix)]
    #[test]
    fn chat_spawn_survives_a_workers_only_latch() {
        // Stage a fake `claude` that prints a result (reuse the staging idiom).
        let dir = std::env::temp_dir().join(format!("abtest-claude-chat-scope-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("claude");
        std::fs::write(&bin, "#!/bin/sh\nprintf '{\"result\":\"hi\"}'\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let old_path = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("{}:{}", dir.display(), old_path));

        let reg = Arc::new(ProcessRegistry::new());
        reg.begin_kill_workers(); // workers-only latch (a Stop is in progress)
        let res = (super::killable_chat_spawn(&reg))(&["-p".into(), "hi".into()], None);

        std::env::set_var("PATH", old_path);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(res.is_ok(), "chat is exempt from a workers-only latch: {res:?}");
    }
```

- [ ] **Step 2: Run it red.** Run: `cd src-tauri && cargo test -p agent-bus-app chat_spawn_survives_a_workers_only_latch`
  Expected: FAIL (chat is currently latched/self-killed, or `begin_kill_workers` missing).

- [ ] **Step 3: Implement.** Add a public `begin_kill_workers()` (sets `Some(KillScope::Workers)`) for the test. In `killable_spawn` call `register_child_scoped(pgid, child, Scope::Worker)`; in `killable_chat_spawn` call `register_child_scoped(pgid, child, Scope::Chat)`.

- [ ] **Step 4: Run it green.** Run: `cd src-tauri && cargo test -p agent-bus-app chat_spawn_survives killable`
  Expected: PASS.

- [ ] **Step 5: Commit.**

```bash
cd src-tauri && git add app/src/process_registry.rs && \
  git commit -m "feat(lifecycle): worker spawner Worker-scoped, chat spawner Chat-scoped (LH5)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

### Task LH5c: Wire Stop → kill_workers, exit → kill_all at the root

**Files:**
- Modify: `src-tauri/app/src/lib.rs` — `brake_on` command (`:792`), RootDispatcher `"brake_on"` arm (`:1103`), the exit handler (`:1658`)
- Test: `src-tauri/app/src/lib.rs` (structural — assert the functions exist / a smoke compile; behavioral coverage lives in `process_registry.rs`)

- [ ] **Step 1: Change the call sites.** Replace `registry.kill_all();` at `:792` (the `brake_on` command) and `self.process_registry.kill_all();` at `:1103` (the dispatcher `brake_on` arm) with `kill_workers()` (Stop must NOT kill chat). LEAVE `process_registry_for_exit.kill_all();` at `:1658` (exit kills both). The auto-meter site (`:1583`) is handled by LH7 (no kill at all).

- [ ] **Step 2: Verify build + existing tests.** Run: `cd src-tauri && cargo build -p agent-bus-app && cargo test -p agent-bus-app`
  Expected: PASS (no behavioral test here; the scoped-kill behavior is proven in `process_registry.rs`).

- [ ] **Step 3: Commit.**

```bash
cd src-tauri && git add app/src/lib.rs && \
  git commit -m "feat(lifecycle): Stop kills workers only; exit still kills both (LH5)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

## Section LH3 — Offload the blocking kill grace off the Tokio runtime at the async Stop sites

`kill_workers`/`kill_all` block ~2.5s in the SIGTERM→grace→SIGKILL loop. The three Stop-path sites run inside async contexts (`brake_on` command, dispatcher `brake_on` arm, the auto-meter sweep), so a bare synchronous call freezes the runtime worker thread. The exit-path call (`:1658`, sync `RunEvent` thread) stays as-is. Offload via `spawn_blocking`. Decision: the brake flips on immediately (blocks new claims now); the kill completes on the blocking pool; the user-facing Stop awaits the bounded kill so the UI is truthful.

### Task LH3a: Add `kill_workers_blocking` async helper that offloads the grace

**Files:**
- Modify: `src-tauri/app/src/process_registry.rs`
- Test: same file (`#[tokio::test]`)

- [ ] **Step 1: Write the failing test.** A registered slow-to-die child; the async helper returns within the bounded window and the child is dead; assert the call did not block (a concurrent future ran). Keep it simple — assert the helper compiles + kills:

```rust
    #[cfg(unix)]
    #[tokio::test]
    async fn kill_workers_blocking_offloads_and_kills() {
        use std::time::{Duration, Instant};
        use std::os::unix::process::CommandExt;
        use std::process::Command;
        let reg = Arc::new(ProcessRegistry::new());
        let child = Command::new("sh").arg("-c").arg("sleep 30 & wait")
            .process_group(0).spawn().expect("spawn");
        let pgid = child.id() as i32;
        reg.register_child_scoped(pgid, child, Scope::Worker);
        // A concurrent tick must make progress while the kill runs on the blocking pool.
        let ticker = tokio::spawn(async { tokio::time::sleep(Duration::from_millis(10)).await; 7 });
        reg.kill_workers_blocking().await;
        assert_eq!(ticker.await.unwrap(), 7, "the executor was not starved");
        let alive = |p: i32| unsafe { libc::kill(-p, 0) == 0 };
        let deadline = Instant::now() + Duration::from_secs(3);
        while alive(pgid) && Instant::now() < deadline { std::thread::sleep(Duration::from_millis(50)); }
        assert!(!alive(pgid));
    }
```

- [ ] **Step 2: Run it red.** Run: `cd src-tauri && cargo test -p agent-bus-app kill_workers_blocking_offloads`
  Expected: FAIL (no method `kill_workers_blocking`).

- [ ] **Step 3: Implement.**

```rust
    /// Async wrapper for the Stop path: runs the blocking grace loop on the
    /// blocking thread pool so it never stalls a Tokio worker. Brake-on callers
    /// flip the brake first (new claims gated immediately), then await this.
    pub async fn kill_workers_blocking(self: &Arc<Self>) {
        let me = self.clone();
        let _ = tauri::async_runtime::spawn_blocking(move || me.kill_workers()).await;
    }
```

(If `tauri::async_runtime::spawn_blocking` is unavailable in this Tauri version, use `tokio::task::spawn_blocking`. Confirm by `grep -rn spawn_blocking src-tauri/`.) The `self: &Arc<Self>` receiver means callers hold `Arc<ProcessRegistry>` (they do — it is managed State).

- [ ] **Step 4: Run it green.** Run: `cd src-tauri && cargo test -p agent-bus-app kill_workers_blocking_offloads`
  Expected: PASS.

- [ ] **Step 5: Commit.**

```bash
cd src-tauri && git add app/src/process_registry.rs && \
  git commit -m "feat(lifecycle): kill_workers_blocking offloads the grace off Tokio (LH3)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

### Task LH3b: Use the async helper at the async Stop sites

**Files:**
- Modify: `src-tauri/app/src/lib.rs` — `brake_on` command (`:786`, make it `async`), dispatcher `"brake_on"` arm (`:1098`, already in async `dispatch`)
- Test: `cargo build` + existing tests

- [ ] **Step 1: Change the sites.** Make the `brake_on` command `async fn` and replace `registry.kill_workers();` with `registry.kill_workers_blocking().await;` (the `tauri::State` derefs to `&Arc<…>`; pass `&*registry` or `registry.inner()` per the State API — confirm with the existing async command idiom in the file). In the dispatcher arm (`:1098`, the enclosing `dispatch` is already `async`), replace `self.process_registry.kill_workers();` with `self.process_registry.kill_workers_blocking().await;`.

- [ ] **Step 2: Verify.** Run: `cd src-tauri && cargo build -p agent-bus-app && cargo clippy -p agent-bus-app --all-targets -- -D warnings && cargo test -p agent-bus-app`
  Expected: PASS.

- [ ] **Step 3: Commit.**

```bash
cd src-tauri && git add app/src/lib.rs && \
  git commit -m "feat(lifecycle): async Stop sites offload the kill grace (LH3)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

## Section LH4 — Crash-orphan reaping on boot (persist liveness, reap survivors before recovery)

An unclean death (panic / SIGKILL / OOM / power loss) bypasses every kill trigger; the surviving `claude` children keep writing the worktree, and the next boot re-queues their tasks → two invocations on one task. Fix: persist `(run_id, task_id, pgid, started_ts)` when the spawner registers; on boot, BEFORE `release_orphaned_running`/`reconcile_occupancy`, reap any still-alive survivors (with a PID-reuse/start-time guard) and clear the records.

This section adds migration `013`, the `LiveProcessStore`, the boot reap, and threads the store into the spawner. The spawner currently has no `run_id`/`task_id` in scope (it is a generic `SpawnFn(args, cwd)`); the persisted record therefore keys on the **pgid + started_ts** as identity and stores `run_id`/`task_id` as `NULL` in v1 (the reap only needs pgid + start-time to safely signal; the task identity is best-effort metadata). This keeps the `SpawnFn` signature unchanged.

### Task LH4a: Add migration 013 (live_processes + brake_state) and register it everywhere

**Files:**
- Create: `src-tauri/app/migrations/013_lifecycle_hardening.sql`
- Modify: `src-tauri/app/src/lib.rs` — `MIGRATIONS` array (`:39-52`), `tauri_plugin_sql` `migrations` vec (insert after the version-12 `Migration{}` at `:1249-1254`), idempotency test (`:1873`)
- Test: `src-tauri/app/src/lib.rs` (`fresh_file_is_created_migrated_and_idempotent`)

- [ ] **Step 1: Write the migration file.** Create `src-tauri/app/migrations/013_lifecycle_hardening.sql`:

```sql
-- 013_lifecycle_hardening.sql — Lifecycle hardening (LH4 + LH6).
-- Append-only; migrations 001–012 are never edited. Additive only.

-- LH4: durable record of a spawned `claude` process group, so an UNCLEAN app
-- death (the in-memory ProcessRegistry is lost) can still be reaped on the next
-- boot before recovery re-queues the task. Deregister deletes the row; a clean
-- exit therefore leaves none. started_ts is the PID-reuse guard: on boot we only
-- signal a pgid whose process start-time still matches (a recycled pid won't).
CREATE TABLE IF NOT EXISTS live_processes (
  pgid        INTEGER PRIMARY KEY,
  run_id      TEXT,
  task_id     TEXT,
  started_ts  INTEGER NOT NULL
);

-- LH6: durable brake state, so an explicit Stop survives app exit/reboot. One
-- row. The runtime Brake aggregate stays persistence-unaware; the root writes
-- this on every set_on/set_off and reads it (reason-aware) on boot.
CREATE TABLE IF NOT EXISTS brake_state (
  id      INTEGER PRIMARY KEY CHECK (id = 1),
  on_flag INTEGER NOT NULL DEFAULT 0,
  reason  TEXT,
  ts      INTEGER NOT NULL DEFAULT 0
);
INSERT OR IGNORE INTO brake_state (id, on_flag, reason, ts) VALUES (1, 0, NULL, 0);
```

- [ ] **Step 2: Register in the `MIGRATIONS` array.** In `app/src/lib.rs` add to the `const MIGRATIONS` array (after the `(12, …)` entry at `:51`):

```rust
        (13, include_str!("../migrations/013_lifecycle_hardening.sql")),
```

- [ ] **Step 3: Register in the `tauri_plugin_sql` vec.** After the version-12 `Migration { … }` block (`:1249-1254`) add:

```rust
        Migration {
            version: 13,
            description: "lifecycle hardening — live_processes (crash reap) + brake_state (persist) (LH4/LH6)",
            sql: include_str!("../migrations/013_lifecycle_hardening.sql"),
            kind: MigrationKind::Up,
        },
```

- [ ] **Step 4: Bump the idempotency test.** In `fresh_file_is_created_migrated_and_idempotent` change the final assertion (`:1877`) from `12` to `13`, and add table-existence assertions:

```rust
        let lh_tables: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM sqlite_master WHERE type='table' \
             AND name IN ('live_processes','brake_state')",
        ).fetch_one(&pool).await.unwrap();
        assert_eq!(lh_tables, 2, "migration 013 created live_processes + brake_state");
```

```rust
        assert_eq!(version, 13, "all thirteen migrations recorded");
```

- [ ] **Step 5: Run it red→green.** Run: `cd src-tauri && cargo test -p agent-bus-app fresh_file_is_created_migrated_and_idempotent`
  Expected: PASS (the file is created, migrated twice idempotently, version 13, both tables present).

- [ ] **Step 6: Commit.**

```bash
cd src-tauri && git add app/migrations/013_lifecycle_hardening.sql app/src/lib.rs && \
  git commit -m "feat(lifecycle): migration 013 — live_processes + brake_state (LH4/LH6)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

### Task LH4b: `LiveProcessStore` — insert/delete/list + a PID-reuse-guarded boot reap

**Files:**
- Create: `src-tauri/app/src/process_records.rs`
- Modify: `src-tauri/app/src/lib.rs` (add `mod process_records;` near `mod process_registry;` at `:3`)
- Test: `src-tauri/app/src/process_records.rs` (`#[cfg(test)] mod tests`)

- [ ] **Step 1: Write the failing tests.** Create `process_records.rs` with the store skeleton and these tests:

```rust
//! LiveProcessStore — durable record of spawned `claude` process groups so an
//! UNCLEAN app death can be reaped on the next boot, BEFORE recovery re-queues
//! the task. A composition-root concern (the runtime crate stays unaware),
//! mirroring the persisted-brake pattern. Unix-only reap; the record itself is
//! cross-platform.

use sqlx::SqlitePool;

pub struct LiveProcessStore { pool: SqlitePool }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveProcess {
    pub pgid: i64,
    pub run_id: Option<String>,
    pub task_id: Option<String>,
    pub started_ts: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::str::FromStr;

    async fn fresh_pool() -> SqlitePool {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:").unwrap().foreign_keys(false);
        let pool = SqlitePoolOptions::new().connect_with(opts).await.unwrap();
        sqlx::query(include_str!("../migrations/013_lifecycle_hardening.sql"))
            .execute(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn insert_list_delete_round_trip() {
        let store = LiveProcessStore::new(fresh_pool().await);
        store.insert(1234, Some("R-1"), Some("T-1"), 500).await.unwrap();
        let all = store.list().await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].pgid, 1234);
        assert_eq!(all[0].started_ts, 500);
        store.delete(1234).await.unwrap();
        assert!(store.list().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn clear_all_empties_the_table() {
        let store = LiveProcessStore::new(fresh_pool().await);
        store.insert(1, None, None, 1).await.unwrap();
        store.insert(2, None, None, 2).await.unwrap();
        assert_eq!(store.clear_all().await.unwrap(), 2);
        assert!(store.list().await.unwrap().is_empty());
    }
}
```

- [ ] **Step 2: Run it red.** Run: `cd src-tauri && cargo test -p agent-bus-app insert_list_delete_round_trip`
  Expected: FAIL (no `new`/`insert`/`list`/`delete`/`clear_all`).

- [ ] **Step 3: Implement the store.** Add to `process_records.rs`:

```rust
impl LiveProcessStore {
    pub fn new(pool: SqlitePool) -> Self { Self { pool } }

    pub async fn insert(&self, pgid: i64, run_id: Option<&str>, task_id: Option<&str>, started_ts: i64)
        -> Result<(), sqlx::Error>
    {
        sqlx::query("INSERT OR REPLACE INTO live_processes (pgid, run_id, task_id, started_ts) VALUES (?,?,?,?)")
            .bind(pgid).bind(run_id).bind(task_id).bind(started_ts)
            .execute(&self.pool).await?;
        Ok(())
    }

    pub async fn delete(&self, pgid: i64) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM live_processes WHERE pgid = ?").bind(pgid)
            .execute(&self.pool).await?;
        Ok(())
    }

    pub async fn list(&self) -> Result<Vec<LiveProcess>, sqlx::Error> {
        let rows: Vec<(i64, Option<String>, Option<String>, i64)> =
            sqlx::query_as("SELECT pgid, run_id, task_id, started_ts FROM live_processes")
                .fetch_all(&self.pool).await?;
        Ok(rows.into_iter().map(|(pgid, run_id, task_id, started_ts)|
            LiveProcess { pgid, run_id, task_id, started_ts }).collect())
    }

    pub async fn clear_all(&self) -> Result<u64, sqlx::Error> {
        Ok(sqlx::query("DELETE FROM live_processes").execute(&self.pool).await?.rows_affected())
    }
}
```

Add `mod process_records;` to `lib.rs` next to `mod process_registry;` (`:3`).

- [ ] **Step 4: Run it green.** Run: `cd src-tauri && cargo test -p agent-bus-app process_records`
  Expected: PASS.

- [ ] **Step 5: Commit.**

```bash
cd src-tauri && git add app/src/process_records.rs app/src/lib.rs && \
  git commit -m "feat(lifecycle): LiveProcessStore persists spawned pgids (LH4)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

### Task LH4c: Boot `reap_orphans` — kill still-alive survivors with a start-time guard, then clear

**Files:**
- Modify: `src-tauri/app/src/process_records.rs` (add `reap_orphans` free fn + a process-start-time probe)
- Test: same file (real `sleep` child simulating a prior-session orphan)

- [ ] **Step 1: Write the failing test.** Real child in its own group simulating last session's survivor:

```rust
    #[cfg(unix)]
    #[tokio::test]
    async fn reap_orphans_kills_a_recorded_survivor_and_clears_records() {
        use std::time::{Duration, Instant};
        use std::os::unix::process::CommandExt;
        use std::process::Command;
        let store = LiveProcessStore::new(fresh_pool().await);
        let child = Command::new("sh").arg("-c").arg("sleep 30 & wait")
            .process_group(0).spawn().expect("spawn");
        let pgid = child.id() as i32;
        // Record it as a prior-session orphan. started_ts: read the live start-time
        // so the guard matches (in production the spawner records the same probe).
        let started = super::process_start_ts(pgid).unwrap_or(0);
        store.insert(pgid as i64, Some("R-1"), Some("T-1"), started).await.unwrap();

        super::reap_orphans(&store).await;

        assert!(store.list().await.unwrap().is_empty(), "records cleared after reap");
        let alive = |p: i32| unsafe { libc::kill(-p, 0) == 0 };
        let deadline = Instant::now() + Duration::from_secs(3);
        while alive(pgid) && Instant::now() < deadline { std::thread::sleep(Duration::from_millis(50)); }
        assert!(!alive(pgid), "the orphan was reaped");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn reap_orphans_skips_a_pid_with_a_mismatched_start_time() {
        // A record whose started_ts does NOT match the live process is a recycled
        // pid: it must NOT be signalled. Use our OWN pid with a bogus start-time.
        let store = LiveProcessStore::new(fresh_pool().await);
        let me = std::process::id() as i64;
        store.insert(me, None, None, /*bogus*/ 1).await.unwrap();
        super::reap_orphans(&store).await; // must not kill ourselves
        assert!(store.list().await.unwrap().is_empty(), "records still cleared");
        // If we are still running to assert this, we were not killed. :)
    }
```

- [ ] **Step 2: Run it red.** Run: `cd src-tauri && cargo test -p agent-bus-app reap_orphans`
  Expected: FAIL (no `reap_orphans` / `process_start_ts`).

- [ ] **Step 3: Implement.** Add a start-time probe + the reap. On macOS/BSD `ps -o lstart= -p <pid>` is portable enough; the simplest robust guard is the process *start time in clock ticks* via `ps -o etimes=` (elapsed seconds) compared loosely, OR — preferred for determinism — record and compare the value `ps -p <pid> -o lstart=` returns. Use `ps` so no extra crate is needed:

```rust
/// A coarse process-start fingerprint for the PID-reuse guard: the kernel's
/// reported start time for `pid` (via `ps -o lstart=`). None if the pid is gone.
/// Equality of this string across the spawn-record and the boot-reap means the
/// pid was not recycled. PURE-ish (shells out to ps); unix-only.
#[cfg(unix)]
pub fn process_start_ts(pid: i32) -> Option<i64> {
    // `ps -o lstart=` is a human date; hash it to a stable i64 so it fits the
    // INTEGER column and compares exactly. A recycled pid yields a different
    // start date -> different hash -> guard rejects it.
    let out = std::process::Command::new("ps")
        .args(["-o", "lstart=", "-p", &pid.to_string()])
        .output().ok()?;
    if !out.status.success() { return None; }
    let s = String::from_utf8_lossy(&out.stdout);
    let s = s.trim();
    if s.is_empty() { return None; }
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    Some(h.finish() as i64)
}

#[cfg(not(unix))]
pub fn process_start_ts(_pid: i32) -> Option<i64> { None }

/// Boot reap (LH4): for each recorded prior-session pgid that is STILL ALIVE and
/// whose start-time fingerprint still matches (not a recycled pid), SIGTERM the
/// group, grace, SIGKILL; then clear ALL records (clean exit left none anyway).
/// MUST run BEFORE release_orphaned_running / reconcile_occupancy so the re-run
/// has no surviving competitor. Best-effort + logged; never blocks boot fatally.
pub async fn reap_orphans(store: &LiveProcessStore) {
    let records = match store.list().await {
        Ok(r) => r,
        Err(e) => { eprintln!("app: reap_orphans list failed: {e}"); return; }
    };
    #[cfg(unix)]
    for rec in &records {
        let pgid = rec.pgid as i32;
        // Alive? probe with signal 0.
        if unsafe { libc::kill(-pgid, 0) } != 0 { continue; }
        // PID-reuse guard: the live start-time must match the recorded one.
        if process_start_ts(pgid) != Some(rec.started_ts) {
            eprintln!("app: reap_orphans skipping pgid {pgid} (start-time mismatch / recycled)");
            continue;
        }
        unsafe { libc::kill(-pgid, libc::SIGTERM); }
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(2500);
        while unsafe { libc::kill(-pgid, 0) } == 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        if unsafe { libc::kill(-pgid, 0) } == 0 {
            unsafe { libc::kill(-pgid, libc::SIGKILL); }
        }
    }
    if let Err(e) = store.clear_all().await {
        eprintln!("app: reap_orphans clear_all failed: {e}");
    }
}
```

- [ ] **Step 4: Run it green.** Run: `cd src-tauri && cargo test -p agent-bus-app reap_orphans`
  Expected: PASS (the recorded survivor is reaped; the mismatched-start-time record is NOT signalled; records cleared).

- [ ] **Step 5: Commit.**

```bash
cd src-tauri && git add app/src/process_records.rs && \
  git commit -m "feat(lifecycle): boot reap of crash-orphaned process groups with PID-reuse guard (LH4)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

### Task LH4d: Spawner persists a record on register, deletes on reap

**Files:**
- Modify: `src-tauri/app/src/process_registry.rs` (give the registry an optional `Arc<LiveProcessStore>` + record on `register_child_scoped`, delete on `take_child`/deregister)
- Test: same file (assert a record is written then deleted)

Because the spawner is a generic `SpawnFn(args, cwd)` with no `run_id`/`task_id`, the persisted `run_id`/`task_id` are `NULL` in v1; the pgid + start-time are the identity the reap needs. The store handle is optional so the existing unit tests (which build a bare `ProcessRegistry::new()`) keep working.

- [ ] **Step 1: Write the failing test.** A `#[tokio::test]` that wires a store into the registry, runs a quick child, and asserts the record was inserted then removed:

```rust
    #[cfg(unix)]
    #[tokio::test]
    async fn spawner_persists_a_live_record_then_removes_it_on_reap() {
        use crate::process_records::LiveProcessStore;
        use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
        use std::str::FromStr;
        let opts = SqliteConnectOptions::from_str("sqlite::memory:").unwrap().foreign_keys(false);
        let pool = SqlitePoolOptions::new().connect_with(opts).await.unwrap();
        sqlx::query(include_str!("../migrations/013_lifecycle_hardening.sql")).execute(&pool).await.unwrap();
        let store = Arc::new(LiveProcessStore::new(pool));
        let reg = Arc::new(ProcessRegistry::new().with_live_store(store.clone()));
        // A child that exits promptly: record written at register, deleted at reap.
        let out = (super::killable_spawn(&reg))(
            &["sh".into(), "-c".into(), "printf ok".into()], None);
        assert_eq!(out.unwrap(), "ok");
        // The spawner runs the blocking drain+wait on the calling thread; persistence
        // is fire-and-forget via the runtime handle, so allow a brief settle.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(store.list().await.unwrap().is_empty(), "record removed after reap");
    }
```

- [ ] **Step 2: Run it red.** Run: `cd src-tauri && cargo test -p agent-bus-app spawner_persists_a_live_record`
  Expected: FAIL (no `with_live_store`).

- [ ] **Step 3: Implement.** The spawner runs on a blocking thread (it does sync `read_to_end`/`wait`), but the store is async. Bridge via `tauri::async_runtime::spawn` (fire-and-forget) for the insert/delete, OR add a tiny blocking helper on the store using `tauri::async_runtime::block_on`. Prefer fire-and-forget so the spawner stays sync:
  - Add `live_store: Option<Arc<crate::process_records::LiveProcessStore>>` to `ProcessRegistry` (inside or beside `Inner`), a `with_live_store(self, store)` builder, and a private `record_spawn(pgid, started_ts)` / `record_reap(pgid)` that `spawn`s the async store call.
  - In `killable_spawn`, after a successful `Registered`, compute `let started = crate::process_records::process_start_ts(pgid).unwrap_or(0);` and call `registry.record_spawn(pgid, started);`. After `take_child`+`wait` (reap), call `registry.record_reap(pgid);`. Do the same in `killable_chat_spawn`.
  - On the `KillImmediately` self-kill branch nothing was recorded, so no delete is needed.

```rust
    pub fn with_live_store(mut self, store: Arc<crate::process_records::LiveProcessStore>) -> Self {
        self.live_store = Some(store);
        self
    }
    fn record_spawn(&self, pgid: i32, started_ts: i64) {
        if let Some(s) = &self.live_store {
            let s = s.clone();
            tauri::async_runtime::spawn(async move {
                let _ = s.insert(pgid as i64, None, None, started_ts).await;
            });
        }
    }
    fn record_reap(&self, pgid: i32) {
        if let Some(s) = &self.live_store {
            let s = s.clone();
            tauri::async_runtime::spawn(async move { let _ = s.delete(pgid as i64).await; });
        }
    }
```

(`with_live_store` consuming `self` requires `ProcessRegistry` fields be movable; if `Default`-derived this is fine. If the existing `new()` returns a value, `with_live_store` chains cleanly.)

- [ ] **Step 4: Run it green.** Run: `cd src-tauri && cargo test -p agent-bus-app spawner_persists_a_live_record process_registry`
  Expected: PASS.

- [ ] **Step 5: Commit.**

```bash
cd src-tauri && git add app/src/process_registry.rs && \
  git commit -m "feat(lifecycle): spawner persists+removes live-process records (LH4)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

### Task LH4e: Wire the store into the registry at boot + reap BEFORE recovery

**Files:**
- Modify: `src-tauri/app/src/lib.rs` — registry construction (`:1260`), boot sequence before `release_orphaned_running` (`:1337`)
- Test: `cargo build` (the reap-ordering behavior is covered by `process_records.rs` tests)

- [ ] **Step 1: Build the store + attach to the registry.** The registry is built at `:1260` (`Arc::new(ProcessRegistry::new())`) before the pool exists; the pool is created in `setup` at `:1278`. Move the `with_live_store` wiring into `setup` after the pool + `run_migrations`: construct `let live_processes = Arc::new(process_records::LiveProcessStore::new(pool.clone()));`, and build the registry with the store. Two options: (a) construct the `ProcessRegistry` inside `setup` and clone it for the exit handle via a channel — heavier; (b) keep the registry built outside but make `live_store` an interior-mutable `OnceLock`/`Mutex<Option<…>>` set in `setup`. Prefer (b): add `set_live_store(&self, store)` that fills an interior `Mutex<Option<Arc<…>>>`. Then at boot call `process_registry.set_live_store(live_processes.clone());`.

- [ ] **Step 2: Reap before recovery.** Immediately BEFORE `let _ = tasks.release_orphaned_running(now_unix()).await;` (`:1337`), add:

```rust
                // LH4: reap any crash-orphaned `claude` groups from a prior
                // session BEFORE re-queueing their tasks, so the re-run has no
                // surviving competitor. Best-effort; clears the records.
                process_records::reap_orphans(&live_processes).await;
```

- [ ] **Step 3: Verify.** Run: `cd src-tauri && cargo build -p agent-bus-app && cargo test -p agent-bus-app && cargo clippy -p agent-bus-app --all-targets -- -D warnings`
  Expected: PASS.

- [ ] **Step 4: Commit.**

```bash
cd src-tauri && git add app/src/lib.rs app/src/process_registry.rs && \
  git commit -m "feat(lifecycle): wire LiveProcessStore + reap orphans before boot recovery (LH4)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

## Section LH6 — Brake persistence + per-reason restore

A user hits Stop (brake on, work killed) then quits. Today `Brake::new()` comes up OFF and recovery re-runs the stopped work — silently un-Stopping. Fix: persist `(on, reason, ts)` on every `set_on`/`set_off` (wired at the root; `Brake` stays unaware), and on boot restore **authoritatively for a manual/reactive reason** (come up braked, no auto-resume) but **let an `AUTO_METER_REASON` brake re-derive from fresh telemetry** (don't strand the run — the next sweep decides). Share one `is_manual_reason`/`persists_across_reboot` discriminator.

### Task LH6a: `BrakeStore` + the reason discriminator

**Files:**
- Create: `src-tauri/app/src/brake_persist.rs`
- Modify: `src-tauri/app/src/lib.rs` (`mod brake_persist;` near `:3`)
- Test: `src-tauri/app/src/brake_persist.rs`

- [ ] **Step 1: Write the failing tests.**

```rust
//! BrakeStore — durable brake state (LH6). The runtime `Brake` aggregate stays
//! persistence-unaware; the root writes this row on every set_on/set_off and
//! reads it (reason-aware) on boot. Lives in `app` (a composition-root concern).

use sqlx::SqlitePool;
use usage_telemetry::brake_policy::AUTO_METER_REASON;

/// Does a brake reason represent operator/reactive INTENT that must survive a
/// reboot (come up braked, no auto-resume)? An auto-meter brake is a derived,
/// self-releasing function of a usage window — it must NOT be restored
/// authoritatively (the next sweep re-derives it from fresh telemetry).
pub fn persists_across_reboot(reason: Option<&str>) -> bool {
    !matches!(reason, Some(AUTO_METER_REASON))
}

pub struct BrakeStore { pool: SqlitePool }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedBrake { pub on: bool, pub reason: Option<String>, pub ts: i64 }

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::str::FromStr;

    async fn fresh_pool() -> SqlitePool {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:").unwrap().foreign_keys(false);
        let pool = SqlitePoolOptions::new().connect_with(opts).await.unwrap();
        sqlx::query(include_str!("../migrations/013_lifecycle_hardening.sql")).execute(&pool).await.unwrap();
        pool
    }

    #[test]
    fn manual_reasons_persist_auto_meter_does_not() {
        assert!(persists_across_reboot(Some("manual")));
        assert!(persists_across_reboot(Some("rate-limit")));
        assert!(persists_across_reboot(None));
        assert!(!persists_across_reboot(Some(AUTO_METER_REASON)));
    }

    #[tokio::test]
    async fn save_then_load_round_trip() {
        let store = BrakeStore::new(fresh_pool().await);
        store.save(true, Some("manual"), 500).await.unwrap();
        let p = store.load().await.unwrap();
        assert_eq!(p, PersistedBrake { on: true, reason: Some("manual".into()), ts: 500 });
        store.save(false, None, 600).await.unwrap();
        assert_eq!(store.load().await.unwrap(), PersistedBrake { on: false, reason: None, ts: 600 });
    }
}
```

- [ ] **Step 2: Run it red.** Run: `cd src-tauri && cargo test -p agent-bus-app brake_persist`
  Expected: FAIL (no `BrakeStore::new`/`save`/`load`).

- [ ] **Step 3: Implement.**

```rust
impl BrakeStore {
    pub fn new(pool: SqlitePool) -> Self { Self { pool } }

    pub async fn save(&self, on: bool, reason: Option<&str>, ts: i64) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE brake_state SET on_flag = ?, reason = ?, ts = ? WHERE id = 1")
            .bind(on as i64).bind(reason).bind(ts)
            .execute(&self.pool).await?;
        Ok(())
    }

    pub async fn load(&self) -> Result<PersistedBrake, sqlx::Error> {
        let (on_flag, reason, ts): (i64, Option<String>, i64) =
            sqlx::query_as("SELECT on_flag, reason, ts FROM brake_state WHERE id = 1")
                .fetch_one(&self.pool).await?;
        Ok(PersistedBrake { on: on_flag != 0, reason, ts })
    }
}
```

Add `mod brake_persist;` to `lib.rs`.

- [ ] **Step 4: Run it green.** Run: `cd src-tauri && cargo test -p agent-bus-app brake_persist`
  Expected: PASS.

- [ ] **Step 5: Commit.**

```bash
cd src-tauri && git add app/src/brake_persist.rs app/src/lib.rs && \
  git commit -m "feat(lifecycle): BrakeStore + persists_across_reboot discriminator (LH6)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

### Task LH6b: Persist on every brake set_on/set_off at the root

**Files:**
- Modify: `src-tauri/app/src/lib.rs` — `brake_on` command (`:786`), dispatcher `"brake_on"`/`"brake_off"` arms (`:1098`/`:1107`), auto-meter `SetOn`/`Release` (`:1583`/`:1584`)
- Test: structural via `cargo build`; persistence round-trip covered in LH6a

The runtime `Brake` stays unaware; the root writes the row in the same place it flips the brake. The `BrakeStore` is built at boot and reachable at each site. The dispatcher and auto-meter sweep can hold an `Arc<BrakeStore>`; the `brake_on` command resolves it from managed `State`.

- [ ] **Step 1: Build + manage `BrakeStore` at boot.** In `setup`, after the pool, construct `let brake_store = Arc::new(brake_persist::BrakeStore::new(pool.clone()));` and `handle.manage(brake_store.clone());`. Thread a clone into `RootDispatcher` (add a `brake_store: Arc<BrakeStore>` field) and into the auto-meter sweep's captured locals.

- [ ] **Step 2: Write on each transition.** At each on/off site add a persist call paired with the existing `set_on`/`set_off`:
  - `brake_on` command (`:791`): after `runtime.brake.set_on(reason)`, `let _ = brake_store.save(true, Some(&reason), now_unix()).await;` (capture `reason` before moving it into `set_on`). The command is now `async` (LH3b) so `.await` is fine.
  - Dispatcher `"brake_on"` (`:1101`): same. The arm is in async `dispatch`; `self.brake_store.save(...).await`.
  - Dispatcher `"brake_off"` (`:1107`): after `set_off()`, `let _ = self.brake_store.save(false, None, now_unix()).await;`.
  - Auto-meter `SetOn(reason)` (`:1583`): after `brake.set_on(reason)`, `let _ = brake_store.save(true, Some(&reason), now_unix()).await;`.
  - Auto-meter `Release` (`:1584`): after `brake.set_off()`, `let _ = brake_store.save(false, None, now_unix()).await;`.
  - The frontend `runtime::api::brake_off` (`:1638` in the invoke handler) is the runtime crate's command — it does NOT persist. To keep persistence consistent, replace it with a root `brake_off` wrapper (mirroring the existing root `brake_on` at `:786`) that calls `runtime.brake.set_off()` + `brake_store.save(false, None, ...)` + clears the registry latch (see LH7c `end_killing`). Register the new `brake_off` in the invoke handler instead of `runtime::api::brake_off`.

- [ ] **Step 3: Verify.** Run: `cd src-tauri && cargo build -p agent-bus-app && cargo test -p agent-bus-app`
  Expected: PASS.

- [ ] **Step 4: Commit.**

```bash
cd src-tauri && git add app/src/lib.rs && \
  git commit -m "feat(lifecycle): persist brake state on every set_on/set_off at the root (LH6)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

### Task LH6c: Boot restore — manual brake comes up braked; auto-meter does not strand the run

**Files:**
- Modify: `src-tauri/app/src/lib.rs` — brake construction (`:1334`)
- Test: `src-tauri/app/src/lib.rs` (a focused `#[tokio::test]` over a seeded `brake_state` + the restore helper)

- [ ] **Step 1: Write the failing test.** Add a small pure-ish restore helper and test it (full boot is not unit-testable; test the decision):

```rust
    #[tokio::test]
    async fn boot_restore_brakes_for_manual_not_for_auto_meter() {
        use crate::brake_persist::{BrakeStore, PersistedBrake};
        use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
        use std::str::FromStr;
        let opts = SqliteConnectOptions::from_str("sqlite::memory:").unwrap().foreign_keys(false);
        let pool = SqlitePoolOptions::new().connect_with(opts).await.unwrap();
        sqlx::query(include_str!("../migrations/013_lifecycle_hardening.sql")).execute(&pool).await.unwrap();
        let store = BrakeStore::new(pool);

        // Manual Stop persisted -> the Brake comes up ON with the same reason.
        store.save(true, Some("manual"), 1).await.unwrap();
        let b = crate::restore_brake_from(&store).await;
        assert!(b.is_on());
        assert_eq!(b.state().reason.as_deref(), Some("manual"));

        // Auto-meter persisted -> the Brake comes up OFF (sweep re-derives it).
        store.save(true, Some(usage_telemetry::brake_policy::AUTO_METER_REASON), 2).await.unwrap();
        let b2 = crate::restore_brake_from(&store).await;
        assert!(!b2.is_on(), "auto-meter brake must not strand the run across reboot");

        let _ = PersistedBrake { on: false, reason: None, ts: 0 }; // keep import used
    }
```

- [ ] **Step 2: Run it red.** Run: `cd src-tauri && cargo test -p agent-bus-app boot_restore_brakes_for_manual`
  Expected: FAIL (no `restore_brake_from`).

- [ ] **Step 3: Implement the restore helper.** Add a free fn in `lib.rs`:

```rust
/// LH6: build the boot `Brake`, restoring a persisted MANUAL/reactive brake
/// authoritatively (come up braked, no auto-resume) but letting a persisted
/// AUTO_METER_REASON brake stay OFF — the auto-meter sweep re-derives it from
/// fresh telemetry on its first tick, so the run is not stranded.
async fn restore_brake_from(store: &crate::brake_persist::BrakeStore) -> Arc<Brake> {
    let brake = Arc::new(Brake::new());
    if let Ok(p) = store.load().await {
        if p.on && crate::brake_persist::persists_across_reboot(p.reason.as_deref()) {
            brake.set_on(p.reason.unwrap_or_else(|| "manual".into()));
        }
    }
    brake
}
```

Replace `let brake = Arc::new(Brake::new());` (`:1334`) with `let brake = restore_brake_from(&brake_store).await;` (the `brake_store` is built earlier in LH6b's Step 1 — ensure it is constructed before `:1334`).

- [ ] **Step 4: Run it green.** Run: `cd src-tauri && cargo test -p agent-bus-app boot_restore_brakes_for_manual`
  Expected: PASS. Then `cargo build -p agent-bus-app && cargo test -p agent-bus-app`.

- [ ] **Step 5: Commit.**

```bash
cd src-tauri && git add app/src/lib.rs && \
  git commit -m "feat(lifecycle): boot restores manual brake; auto-meter re-derives (LH6)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

## Section LH7+LH8 — Reason-aware kill (auto-meter soft-brake) + killed task stays re-runnable

These are paired. LH7: the kill trigger is reason-aware — a manual Stop kills workers; an `AUTO_METER_REASON` brake is **soft only** (block new claims via the brake gate, let in-flight finish, NO kill, NO re-run). LH8: a *manual* Stop kill (brake on) must leave the worker task **non-terminal** (retain claim / re-queue without a terminal `Failed` verdict), so resume recovers it — otherwise LH7's manual path has nothing to recover.

**Confirmed against the code:** `transform_once` (`runtime/src/engine.rs:366-381`) maps an `invoke` error OR empty output through `operational_failure` (bumps `attempts`; at `MAX_ATTEMPTS` → terminal `NeedsHuman`) and returns `StepOutcome::Failed`. A SIGKILLed child surfaces as `interpret_runner_output(success=false, empty stderr)` → `RunnerError::Other("claude exited non-zero with no stderr")` → `EngineError::Invoke`. So a Stop-kill currently bumps attempts and can terminally escalate — exactly the loss LH8 must prevent. The cheapest fix is at the engine's failure branch: when `ctx.brake.is_on()`, treat the failure as an interrupt — re-queue WITHOUT bumping attempts and WITHOUT escalating, and return a non-`Failed` outcome (reuse `StepOutcome::Braked`, which the worker loop treats as non-settling). The runtime `Brake` is already in `EngineContext` (`ctx.brake`), so this needs no new wiring and keeps the engine Tauri-free.

### Task LH7a: Auto-meter `SetOn` does NOT kill (soft brake only)

**Files:**
- Modify: `src-tauri/app/src/lib.rs` — auto-meter sweep `SetOn` branch (`:1583`)
- Test: `src-tauri/app/src/lib.rs` (a focused structural test asserting the reason gate)

- [ ] **Step 1: Write the failing test.** A pure helper `should_kill_for_reason` gates kills; test it:

```rust
    #[test]
    fn only_manual_reasons_trigger_a_kill() {
        use usage_telemetry::brake_policy::AUTO_METER_REASON;
        assert!(crate::should_kill_for_reason("manual"));
        assert!(crate::should_kill_for_reason("rate-limit"));
        assert!(!crate::should_kill_for_reason(AUTO_METER_REASON));
    }
```

- [ ] **Step 2: Run it red.** Run: `cd src-tauri && cargo test -p agent-bus-app only_manual_reasons_trigger_a_kill`
  Expected: FAIL (no `should_kill_for_reason`).

- [ ] **Step 3: Implement.** Add to `lib.rs` (reuse the LH6 discriminator so the two cannot drift):

```rust
/// LH7: which brake reasons HARD-KILL in-flight work. A manual/reactive Stop
/// kills (the operator chose to halt now); an auto-meter brake is soft only —
/// block new claims, let in-flight finish, no kill, no re-run. Keyed off the
/// same discriminator as brake persistence (LH6).
pub fn should_kill_for_reason(reason: &str) -> bool {
    crate::brake_persist::persists_across_reboot(Some(reason))
}
```

In the auto-meter `SetOn(reason)` branch (`:1583`) REMOVE the `process_registry.kill_all();` (it was already not `kill_workers` — just delete the kill entirely). The branch becomes `brake.set_on(reason); let _ = brake_store.save(true, Some(&reason), now_unix()).await; let _ = handle.emit(...);` — no kill. (The brake gate at `engine::transform_once:300` already blocks new claims; in-flight invocations drain naturally.)

- [ ] **Step 4: Run it green.** Run: `cd src-tauri && cargo test -p agent-bus-app only_manual_reasons_trigger_a_kill && cargo build -p agent-bus-app`
  Expected: PASS.

- [ ] **Step 5: Commit.**

```bash
cd src-tauri && git add app/src/lib.rs && \
  git commit -m "feat(lifecycle): auto-meter brake is soft only — no kill (LH7)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

### Task LH8a: A braked failure in `transform_once` retains the claim (no attempts-bump, non-terminal)

**Files:**
- Modify: `src-tauri/runtime/src/engine.rs` — `transform_once` failure branch (`:371-382`)
- Test: `src-tauri/runtime/src/engine.rs` (`#[cfg(test)] mod tests`, using `FakeRunner`)

- [ ] **Step 1: Write the failing tests.** Drive `transform_once` with a `FakeRunner` that returns an error, once with the brake ON and once OFF; assert: brake-on → task stays `running`, attempts unchanged, outcome is NOT `Failed`; brake-off → terminal/requeue path as today. Reuse the existing engine test harness idioms (there is already a `StepOutcome::Failed` test at `:1990` and a rate-limit test at `:2736` — mirror their setup). Sketch:

```rust
    #[tokio::test]
    async fn a_braked_invocation_failure_retains_the_claim_non_terminal() {
        // Build a ctx whose runner errors, claim a task, then turn the brake ON
        // and step. (If transform_once's brake gate at the top would short-circuit
        // before claiming, set the brake AFTER the claim by using a runner that
        // blocks until the brake flips, OR assert via the failure-branch unit.)
        // Simplest deterministic form: a FakeRunner that errors, brake ON, and
        // assert the failure branch did NOT bump attempts / escalate.
        let ctx = /* engine test ctx with an erroring FakeRunner + brake */;
        ctx.brake.set_on("manual");
        // ... claim is gated by the brake at the top, so test operational_failure's
        // brake-awareness directly instead:
    }
```

Because `transform_once` brake-gates at the very top (`:300` returns `Braked` before claiming), the realistic kill race is: the claim already happened, the invocation was killed mid-flight, and the loop re-enters with the brake now on while the *previous* step is still resolving. The clean, testable seam is `operational_failure`: make IT brake-aware. Test `operational_failure` directly:

```rust
    #[tokio::test]
    async fn operational_failure_under_brake_requeues_without_bumping_attempts() {
        let (ctx, mut task) = /* ctx with brake + a claimed running task, attempts=0 */;
        ctx.brake.set_on("manual");
        super::operational_failure(&ctx, &mut task).await.unwrap();
        assert_eq!(task.attempts, 0, "a braked (killed) failure does not consume an attempt");
        assert_eq!(task.state, crate::task::TaskState::Queued, "re-queued, recoverable");
    }

    #[tokio::test]
    async fn operational_failure_without_brake_bumps_attempts_as_before() {
        let (ctx, mut task) = /* ctx, brake OFF, attempts=0 */;
        super::operational_failure(&ctx, &mut task).await.unwrap();
        assert_eq!(task.attempts, 1, "a genuine failure still consumes an attempt");
    }
```

Confirm `operational_failure`'s current signature (`async fn operational_failure(ctx: &EngineContext, task: &mut Task)` at `:1422`) — it already takes `ctx`, so `ctx.brake` is in scope. Build the test ctx via the existing engine-test ctx builder (grep the test module for how `EngineContext` is assembled — there is a helper around `:1638`/`:2617`).

- [ ] **Step 2: Run it red.** Run: `cd src-tauri && cargo test -p runtime operational_failure_under_brake`
  Expected: FAIL (attempts is bumped to 1 even under the brake).

- [ ] **Step 3: Implement.** Make `operational_failure` brake-aware:

```rust
async fn operational_failure(ctx: &EngineContext, task: &mut Task) -> Result<(), EngineError> {
    let now = now_unix();
    // LH8: a failure WHILE THE BRAKE IS ON is almost certainly a Stop-kill of a
    // live invocation, not a genuine verdict. Leave the task RE-RUNNABLE: re-queue
    // WITHOUT consuming an attempt and WITHOUT escalating to needs-human, so a
    // resume (LH6 manual-restore -> brake-off recovery) re-runs it cleanly.
    if ctx.brake.is_on() {
        task.state = TaskState::Queued;
        task.updated_at = now;
        ctx.tasks.update(task).await?;
        return Ok(());
    }
    if task.attempts >= MAX_ATTEMPTS {
        task.state = TaskState::NeedsHuman;
        task.current_stage = "needs-human".to_string();
    } else {
        task.attempts += 1;
        task.state = TaskState::Queued;
    }
    task.updated_at = now;
    ctx.tasks.update(task).await?;
    Ok(())
}
```

- [ ] **Step 4: Run it green.** Run: `cd src-tauri && cargo test -p runtime operational_failure`
  Expected: PASS (braked → attempts unchanged + Queued; unbraked → attempts bumped).

- [ ] **Step 5: Commit.**

```bash
cd src-tauri && git add runtime/src/engine.rs && \
  git commit -m "feat(lifecycle): a braked invocation failure stays re-runnable (LH8)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

### Task LH8b: Brake-off clears the registry latch (resume re-enables spawning) + manual-restore recovery

**Files:**
- Modify: `src-tauri/app/src/lib.rs` — the root `brake_off` wrapper (added in LH6b Step 2) + the auto-meter `Release` branch (`:1584`)
- Test: structural (`cargo build`); the latch-clear behavior is unit-proven in `process_registry.rs` (LH1)

The LH1 latch stays set after a Stop kill so stragglers self-kill. On Resume (brake-off) it must clear, and the killed `running` rows (LH8a left them `Queued`) are picked up by the live worker loops once the brake is off — no extra re-queue needed because LH8a already kept them `Queued`. For the boot-manual-restore path (LH6c), the run comes up braked; when the operator later hits Resume, `brake_off` clears the latch and the worker loops resume.

- [ ] **Step 1: Implement.** In the root `brake_off` wrapper: after `runtime.brake.set_off()` + the LH6 persist, add `registry.end_killing();` (clears the latch). In the auto-meter `Release` branch (`:1584`): the auto path never set the latch (LH7 removed its kill), so `end_killing()` there is a harmless no-op — add it anyway for symmetry, or omit. Prefer adding it so any reactive brake that DID kill is fully cleared on auto-release.

- [ ] **Step 2: Verify.** Run: `cd src-tauri && cargo build -p agent-bus-app && cargo test -p agent-bus-app && cargo clippy -p agent-bus-app --all-targets -- -D warnings`
  Expected: PASS.

- [ ] **Step 3: Commit.**

```bash
cd src-tauri && git add app/src/lib.rs && \
  git commit -m "feat(lifecycle): brake-off clears the kill latch so resume re-enables spawning (LH8)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

## Section V — Final verification gates

### Task V1: Full workspace gates green

**Files:** none (verification only)

- [ ] **Step 1: Tests.** Run: `cd src-tauri && cargo test`
  Expected: PASS (all crates, including the new real-process tests in `process_registry.rs` / `process_records.rs` and the engine brake-aware-failure tests).

- [ ] **Step 2: Clippy.** Run: `cd src-tauri && cargo clippy --all-targets -- -D warnings`
  Expected: no warnings. (Watch for `clippy::zombie_processes` on the new real-child tests — annotate with `#[allow(clippy::zombie_processes)]` as the existing kill tests do, since the kill path reaps the group rather than `.wait()`ing directly.)

- [ ] **Step 3: Build.** Run: `cd src-tauri && cargo build`
  Expected: clean build.

- [ ] **Step 4: Commit (only if any gate fix was needed).**

```bash
cd src-tauri && git add -A && \
  git commit -m "chore(lifecycle): pass cargo test/clippy/build gates

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Spec coverage check

- **LH1** (kill/spawn race latch): LH1a (latch + decision), LH1b (kill sets latch), LH1c (spawner self-kills), cleared on brake-off in LH8b. ✓
- **LH2** (PID-reuse guard): LH2a (owned `Child`, take-before-wait invariant), LH2b (race regression). ✓
- **LH3** (async-offload the kill grace): LH3a (`kill_workers_blocking`), LH3b (async Stop sites). Exit path stays sync. ✓
- **LH4** (crash-orphan reaping): LH4a (migration 013 registered everywhere + idempotency bump), LH4b (`LiveProcessStore`), LH4c (`reap_orphans` + PID-reuse guard), LH4d (spawner persists), LH4e (wire + reap BEFORE recovery). ✓
- **LH5** (chat-exempt-from-kill): LH5a (`Scope` + `kill_workers`), LH5b (worker/chat scoping), LH5c (Stop→workers, exit→all). ✓
- **LH6** (brake persistence + per-reason restore): LH6a (`BrakeStore` + `persists_across_reboot`), LH6b (persist on every transition + root `brake_off` wrapper), LH6c (manual restores braked, auto-meter re-derives). ✓
- **LH7** (auto-meter no-kill): LH7a (`should_kill_for_reason`; auto-meter `SetOn` does not kill). ✓
- **LH8** (killed task re-runnable): LH8a (braked `operational_failure` retains claim, no attempts-bump, non-terminal), LH8b (brake-off clears latch so resume re-runs the `Queued` rows). ✓
- **Out of scope (NOT planned):** git worktrees / worktree reset on resume. ✓

## Notes for the implementer

- **Latch granularity** (LH5a): the latch must distinguish "workers-only" (Stop — chat may still spawn) from "all" (exit). Model it as `Inner.killing: Option<KillScope>` and have `register_child_scoped` self-kill only when the latch covers the entry's scope. Revisit the LH1 tests when this lands (adjust `begin_killing()` to the `Option<KillScope>` shape).
- **`tauri::State` deref in async commands** (LH3b/LH6b): confirm the existing async-command idiom in `lib.rs` for resolving `tauri::State<'_, Arc<T>>` (`&*state` or `state.inner().clone()`); match it.
- **Spawner persistence is fire-and-forget** (LH4d): the spawner is a sync `SpawnFn`; the async store calls are dispatched via `tauri::async_runtime::spawn`. The LH4d test allows a brief settle. In production a record that lingers because the delete lost a race is harmless — the next boot's `reap_orphans` finds the pgid dead (signal-0 fails) and just clears it.
- **`run_id`/`task_id` are NULL in the v1 live-process record** (LH4): the `SpawnFn` has no run/task in scope, and the reap only needs pgid + start-time. If a future change threads run/task into the spawner, populate them (the columns already exist).
- **Event names** contain no dots — all emits in the touched code already use `crate::events::*` constants; do not introduce dotted names.
