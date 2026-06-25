# Run Lifecycle UX (LF33) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make "Start run" idempotent + live-refreshing so the operator can't double-press into multiple runs, and demote the run dropdown to a read-only history affordance.

**Architecture:** The frontend `start_run` invoke currently registers the RAW `runtime::api::start_run`, which never emits `run-changed`, so the board and the chunk-4 `● Running`/`Stop` control don't update and a second press creates a second run. We add (1) a Tauri-unaware runtime resolver `start_or_resume_run_inner` that returns the existing incomplete run for the project if one exists (never creating a second) and otherwise creates one via `start_run_inner`; (2) an app-crate `#[tauri::command]` wrapper `start_run` — modeled exactly on the existing `brake_on`/`brake_off` root wrappers — that calls the resolver, clears the brake (resume) + persists when the active run was found braked, then ALWAYS emits `RUN_CHANGED` with the run id; and (3) a frontend change demoting the `<select>` to a "history" affordance shown only when completed runs exist. The `runtime` crate stays Tauri-unaware; the wrapper lives in the app crate where `AppHandle`, `RuntimeState`, the brake, and `BrakeStore` are in scope.

**Tech Stack:** Rust (Tauri 2 commands, sqlx, tokio), React + TypeScript (Vitest, Testing Library).

---

## Verified facts (confirmed against current code)

- `runtime::api::start_run_inner(state: &RuntimeState, _topic: Option<String>) -> Result<Run, String>` — `api.rs:294`. Creates a run + ensures stores. The `_topic` is currently ignored.
- `RunStore::latest_active_for_project(&self, project_id: &str) -> Result<Option<Run>, RunStoreError>` — `run_store.rs:110`. Newest not-completed run for a project.
- `RuntimeState` fields (`api.rs:107`): `pub runs: Arc<RunStore>`, `pub brake: Arc<Brake>`, `pub stores: Arc<StoreRepo>`; `pub fn active(&self) -> Arc<ActivePipeline>` (`api.rs:155`); `ActivePipeline.project_id: String`.
- `Brake` (`brake.rs`): `is_on() -> bool`, `set_off()`, `set_on(reason)`, `state() -> BrakeState`.
- Root wrapper model: `brake_on` (`lib.rs:872`) and `brake_off` (`lib.rs:896`) — app-crate `#[tauri::command(rename_all = "snake_case")]` taking `tauri::State<'_, Arc<RuntimeState>>`, `tauri::State<'_, Arc<brake_persist::BrakeStore>>`, doing root work then returning. `brake_off` does `runtime.brake.set_off(); brake_store.save(false, None, now_unix()).await; registry.end_killing();`.
- `RUN_CHANGED` constant = `crate::events::RUN_CHANGED` (`events.rs:12`, value `"run-changed"`). Emitted via `self.app.emit(...)` in RootDispatcher (`lib.rs:1171`). In a command wrapper the `AppHandle` is obtained as a `tauri::AppHandle` parameter (Tauri injects it) and `app.emit(RUN_CHANGED, &run.id)` is called — `use tauri::Emitter;` is already in scope in `lib.rs` (it backs the `self.app.emit` call).
- invoke_handler registration: `runtime::api::start_run,` at `lib.rs:1849` — this RAW line is what we replace with the app-crate `start_run`.
- Frontend IPC: `startRun(topic?) -> Promise<Run>` invokes `"start_run"` with `{ topic }` (`ipc/runtime.ts:89`). The `Run` shape has `completed: boolean` (`ipc/runtime.ts:65`).
- `RunSelector` props (`RunSelector.tsx:5`): `runs, selectedRun, activeRun, onSelect, onStartRun, braked, onStop, onResume, starting?, loading?`. The chunk-4 `● Running`/`Stop`/`Resume`/`Start run` control already exists (`RunSelector.tsx:72-119`). `useRuns` refetches on `run-changed` (`useRuns.ts:50`) and derives `activeRun` = newest not-completed (`useRuns.ts:53`).
- `App.tsx:90` `handleStartRun` calls `startRunCmd()` then `selectRun(run.id)`.
- Tests: `src/components/RunSelector.test.tsx`, `src/hooks/useRuns.test.ts`, runtime tests in `src-tauri/runtime/src/api.rs` (`mod tests` at `api.rs:816`, helper `state_with_two_team_pipeline()` at ~`api.rs:910`).

---

## File Structure

- `src-tauri/runtime/src/api.rs` — add `start_or_resume_run_inner` (Tauri-unaware resolver) + its unit tests.
- `src-tauri/app/src/lib.rs` — add app-crate `#[tauri::command] start_run` wrapper; swap it into the invoke_handler in place of `runtime::api::start_run`.
- `src/components/RunSelector.tsx` — demote the `<select>` to a history affordance gated on completed runs.
- `src/components/RunSelector.test.tsx` — update/extend tests for the demoted dropdown.

---

### Task 1: Runtime resolver — `start_or_resume_run_inner` (idempotent, Tauri-unaware)

**Files:**
- Modify: `src-tauri/runtime/src/api.rs` (add fn after `start_run_inner`, ~`api.rs:324`)
- Test: `src-tauri/runtime/src/api.rs` (`mod tests`, ~`api.rs:816`)

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `src-tauri/runtime/src/api.rs` (after the existing `list_runs_lists_the_projects_runs_newest_first` test, ~`api.rs:1008`):

```rust
    #[tokio::test]
    async fn start_or_resume_creates_when_none_active() {
        let state = state_with_two_team_pipeline().await;
        let outcome = start_or_resume_run_inner(&state).await.unwrap();
        assert!(matches!(outcome, StartOutcome::Started(_)));
        let run = outcome.run();
        assert!(run.id.starts_with("R-"));
        // stores ensured (it really created the run, not a no-op)
        assert_eq!(state.stores.occupancy(&run.id, "research").await.unwrap(), Some(0));
    }

    #[tokio::test]
    async fn start_or_resume_returns_existing_active_without_creating_a_second() {
        let state = state_with_two_team_pipeline().await;
        let first = start_or_resume_run_inner(&state).await.unwrap().run().clone();
        // second call must NOT create a new run — same id, and only one run exists.
        let second = start_or_resume_run_inner(&state).await.unwrap();
        assert!(matches!(second, StartOutcome::Resumed(_)));
        assert_eq!(second.run().id, first.id);
        assert_eq!(state.runs.list_for_project("proj").await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn start_or_resume_on_empty_pipeline_is_an_error() {
        let state = state_with_two_team_pipeline().await;
        state.activate_into(ActivePipeline {
            pipeline: Arc::new(Pipeline {
                id: "e".into(), name: "E".into(), description: String::new(), schema_version: 3,
                defaults: None, teams: vec![], gates: vec![], escalations: vec![], forks: vec![], joins: vec![],
            }),
            project_id: "proj".into(),
            project_root: "/p".into(),
            project_target_repo: None,
        });
        // No active run exists, so it falls through to create -> empty pipeline errors.
        assert!(start_or_resume_run_inner(&state).await.is_err());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd /Users/tim/projects/agent-bus-app/src-tauri && cargo test -p runtime start_or_resume 2>&1 | tail -20`
Expected: FAIL — `cannot find function start_or_resume_run_inner` / `cannot find type StartOutcome`.

- [ ] **Step 3: Write the resolver + outcome type**

In `src-tauri/runtime/src/api.rs`, immediately after `start_run_inner` (after its closing brace at ~`api.rs:324`), add:

```rust
/// Outcome of a Start press (LF33). `Started` = a new run was created; `Resumed`
/// = an incomplete run already existed for the project and is returned as-is (no
/// second run is ever created). The app-crate command turns `Resumed` into a
/// brake-clear when the brake was on, and emits `run-changed` either way.
pub enum StartOutcome {
    Started(Run),
    Resumed(Run),
}

impl StartOutcome {
    /// The resolved run, regardless of whether it was just created or pre-existing.
    pub fn run(&self) -> &Run {
        match self {
            StartOutcome::Started(r) | StartOutcome::Resumed(r) => r,
        }
    }

    /// Consume into the owned run.
    pub fn into_run(self) -> Run {
        match self {
            StartOutcome::Started(r) | StartOutcome::Resumed(r) => r,
        }
    }
}

/// LF33: idempotent Start. If an incomplete run already exists for the active
/// project, return it (`Resumed`) WITHOUT creating a second — this is what makes a
/// double-press safe. Otherwise create a fresh run via `start_run_inner`
/// (`Started`). Tauri-unaware: the app-crate wrapper owns the brake-clear + the
/// `run-changed` emit; this fn only owns the create-or-reuse decision.
pub async fn start_or_resume_run_inner(state: &RuntimeState) -> Result<StartOutcome, String> {
    let project_id = state.active().project_id.clone();
    if let Some(existing) = state
        .runs
        .latest_active_for_project(&project_id)
        .await
        .map_err(|e| e.to_string())?
    {
        return Ok(StartOutcome::Resumed(existing));
    }
    let run = start_run_inner(state, None).await?;
    Ok(StartOutcome::Started(run))
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd /Users/tim/projects/agent-bus-app/src-tauri && cargo test -p runtime start_or_resume 2>&1 | tail -20`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
cd /Users/tim/projects/agent-bus-app
git add src-tauri/runtime/src/api.rs
git commit -m "feat(runtime): idempotent start_or_resume_run_inner (LF33)"
```

---

### Task 2: App-crate `start_run` command wrapper + invoke_handler swap

**Files:**
- Modify: `src-tauri/app/src/lib.rs` (add command after `brake_off`, ~`lib.rs:906`; swap registration at `lib.rs:1849`)
- Test: `src-tauri/app/src/lib.rs` (new `#[cfg(test)]` module at end of file)

- [ ] **Step 1: Write the failing test**

The wrapper's emit needs a real `AppHandle`, which unit tests can't easily build. Test the brake-clear behaviour by extracting a Tauri-unaware helper and testing IT. Add this test module at the end of `src-tauri/app/src/lib.rs`:

```rust
#[cfg(test)]
mod start_run_wrapper_tests {
    use super::*;
    use runtime::brake::Brake;
    use std::sync::Arc;

    // The wrapper's side-effect decision: a Resumed run while braked must clear
    // the brake (resume); a Started run, or a Resumed run while not braked, leaves
    // the brake untouched. `should_clear_brake` is the pure predicate the command
    // uses before touching the brake/persistence.
    #[test]
    fn resumed_while_braked_clears_the_brake() {
        let brake = Arc::new(Brake::new());
        brake.set_on("manual");
        assert!(should_clear_brake(brake.is_on(), /* resumed = */ true));
    }

    #[test]
    fn started_never_clears_the_brake() {
        let brake = Arc::new(Brake::new());
        brake.set_on("manual");
        assert!(!should_clear_brake(brake.is_on(), /* resumed = */ false));
    }

    #[test]
    fn resumed_while_not_braked_is_a_noop() {
        assert!(!should_clear_brake(/* braked = */ false, /* resumed = */ true));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd /Users/tim/projects/agent-bus-app/src-tauri && cargo test -p app start_run_wrapper 2>&1 | tail -20`
Expected: FAIL — `cannot find function should_clear_brake`.

- [ ] **Step 3: Write the helper + command, swap the registration**

In `src-tauri/app/src/lib.rs`, after `brake_off` (after its closing brace at ~`lib.rs:906`), add:

```rust
/// LF33 predicate: a Start press should clear the brake (resume) only when it
/// RESUMED a pre-existing run AND the brake was on. A freshly Started run, or a
/// Resumed run with the brake already off, leaves the brake alone. Pure so it is
/// unit-testable without an AppHandle.
fn should_clear_brake(braked: bool, resumed: bool) -> bool {
    braked && resumed
}

/// LF33: the frontend "Start run" button. Root-crate wrapper over the runtime
/// resolver so the composition root can emit `run-changed` (the runtime crate
/// stays Tauri-unaware) and clear+persist the brake on a resume. Replaces the raw
/// `runtime::api::start_run` in the invoke handler. IDEMPOTENT: if an incomplete
/// run already exists for the active project it is returned as-is (a double-press
/// never creates a second run — the LF33 bug); if that run was Stopped (braked)
/// the press clears the brake = Resume. ALWAYS emits `run-changed` with the run id
/// so `useRuns` refetches and the board + the `● Running` control update live.
#[tauri::command(rename_all = "snake_case")]
async fn start_run(
    app: tauri::AppHandle,
    runtime: tauri::State<'_, Arc<RuntimeState>>,
    brake_store: tauri::State<'_, Arc<brake_persist::BrakeStore>>,
    _topic: Option<String>,
) -> Result<runtime::model::Run, String> {
    let outcome = runtime::api::start_or_resume_run_inner(runtime.as_ref()).await?;
    let resumed = matches!(outcome, runtime::api::StartOutcome::Resumed(_));
    if should_clear_brake(runtime.brake.is_on(), resumed) {
        // Mirror brake_off: clear the runtime brake + persist the OFF row so the
        // resume survives an app restart. (Worker-loop respawn is driven by the
        // existing brake-off path / activation; Start does not re-kill anything.)
        runtime.brake.set_off();
        let _ = brake_store.save(false, None, now_unix()).await;
    }
    let run = outcome.into_run();
    // ALWAYS emit so the board + the ● Running/Stop control update live even when
    // we resumed/returned the existing run (this is the LF33 close-the-loop fix).
    let _ = app.emit(crate::events::RUN_CHANGED, &run.id);
    Ok(run)
}
```

Then change the invoke_handler line at `src-tauri/app/src/lib.rs:1849` from:

```rust
            runtime::api::start_run,
```

to:

```rust
            start_run,
```

> Note: confirm `runtime::model::Run` is the correct path for the `Run` type (the runtime command returns `runtime::api::...` `Run`; if `start_or_resume_run_inner` returns `runtime::api::Run` use that exact path — match whatever `start_run_inner`'s return type resolves to in `api.rs`). If `now_unix()` is not in scope at this location it already is — `brake_on`/`brake_off` use it in the same file.

- [ ] **Step 4: Run the test + build to verify**

Run: `cd /Users/tim/projects/agent-bus-app/src-tauri && cargo test -p app start_run_wrapper 2>&1 | tail -20`
Expected: PASS (3 tests).

Run: `cd /Users/tim/projects/agent-bus-app/src-tauri && cargo build -p app 2>&1 | tail -20`
Expected: builds clean (the swapped command type-checks against the invoke_handler).

- [ ] **Step 5: Commit**

```bash
cd /Users/tim/projects/agent-bus-app
git add src-tauri/app/src/lib.rs
git commit -m "feat(app): idempotent start_run command emits run-changed + resumes brake (LF33)"
```

---

### Task 3: Demote the run dropdown to a history affordance

**Files:**
- Modify: `src/components/RunSelector.tsx`
- Test: `src/components/RunSelector.test.tsx`

The primary control is the chunk-4 `Start run`/`● Running`/`Stop`/`Resume` cluster on the right. The `<select>` should no longer be the operating surface: show it only when there are completed (past) runs to inspect, labeled "history", not "run". When the only run is the active one (nothing to inspect yet), the dropdown is hidden so Start/Running is the sole focus.

- [ ] **Step 1: Update the existing tests + add the history-gating tests**

Replace the body of `src/components/RunSelector.test.tsx` test `"lists the runs and marks the active one"` and `"calls onSelect with the chosen run id"` to reflect the new "history" label, and add gating tests. Concretely, edit these tests so the dropdown is queried by the `history` label and is asserted to appear only when a completed run exists:

```tsx
  it("shows a history dropdown (labeled history) only when completed runs exist", () => {
    const runs = [run("R-aaaaaaaa"), run("R-bbbbbbbb", true)];
    render(<RunSelector runs={runs} selectedRun={runs[0]} activeRun={runs[0]} onSelect={() => {}} onStartRun={() => {}} braked={false} onStop={() => {}} onResume={() => {}} />);
    const select = screen.getByRole("combobox", { name: /history/i }) as HTMLSelectElement;
    expect(select.value).toBe("R-aaaaaaaa");
    expect(screen.getByText(/aaaaaaaa · active/i)).toBeInTheDocument();
    expect(screen.getByText(/bbbbbbbb · completed/i)).toBeInTheDocument();
  });

  it("hides the dropdown when the only run is the active one (no past runs to inspect)", () => {
    const runs = [run("R-aaaaaaaa")];
    render(<RunSelector runs={runs} selectedRun={runs[0]} activeRun={runs[0]} onSelect={() => {}} onStartRun={() => {}} braked={false} onStop={() => {}} onResume={() => {}} />);
    // no past (completed) runs -> no history dropdown; the ● Running control carries the state.
    expect(screen.queryByRole("combobox")).not.toBeInTheDocument();
    expect(screen.getByText(/running/i)).toBeInTheDocument();
  });

  it("calls onSelect with the chosen run id from the history dropdown", () => {
    const runs = [run("R-aaaaaaaa"), run("R-bbbbbbbb", true)];
    const onSelect = vi.fn();
    render(<RunSelector runs={runs} selectedRun={runs[0]} activeRun={runs[0]} onSelect={onSelect} onStartRun={() => {}} braked={false} onStop={() => {}} onResume={() => {}} />);
    fireEvent.change(screen.getByRole("combobox", { name: /history/i }), { target: { value: "R-bbbbbbbb" } });
    expect(onSelect).toHaveBeenCalledWith("R-bbbbbbbb");
  });
```

Delete the now-superseded original `"lists the runs and marks the active one"` and `"calls onSelect with the chosen run id"` tests (they query by `name: /run/i` and assume the dropdown always renders when `runs.length > 0`, which is no longer true). The test `"shows an empty hint and a Start button when there are no runs"` stays as-is (it already asserts `queryByRole("combobox")` is absent).

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd /Users/tim/projects/agent-bus-app && npx vitest run src/components/RunSelector.test.tsx 2>&1 | tail -25`
Expected: FAIL — the new tests can't find a combobox named `/history/i` (current label is `run`), and the hide-when-only-active test fails because the dropdown currently renders for any non-empty `runs`.

- [ ] **Step 3: Demote the dropdown in `RunSelector.tsx`**

In `src/components/RunSelector.tsx`, compute whether there is history to show, change the label text + `aria-label`, and gate the `<select>` on completed runs.

After the `isRunning` line (`RunSelector.tsx:72`) add:

```tsx
  // LF33: the dropdown is a secondary "history" affordance — surface it only when
  // there are completed (past) runs to inspect. The Start/● Running/Stop cluster
  // is the operating surface; the dropdown is for reviewing finished runs.
  const hasHistory = runs.some((r) => r.completed);
```

Replace the label `<span>` (`RunSelector.tsx:76-78`) — change its text from `run` to `history`:

```tsx
      <span id="run-selector-label" style={label}>
        history
      </span>
```

Replace the conditional block (`RunSelector.tsx:79-96`) so the empty/loading hints stay but the `<select>` is gated on `hasHistory`:

```tsx
      {loading && runs.length === 0 ? (
        <span style={empty} aria-live="polite">loading runs…</span>
      ) : runs.length === 0 ? (
        <span style={empty}>no runs yet — start a run</span>
      ) : hasHistory ? (
        <select
          aria-labelledby="run-selector-label"
          style={select}
          value={selectedRun?.id ?? ""}
          onChange={(e) => onSelect(e.target.value)}
        >
          {runs.map((r) => (
            <option key={r.id} value={r.id}>
              {runOptionLabel(r, activeRun?.id === r.id)}
            </option>
          ))}
        </select>
      ) : null}
```

> The label `<span>` always renders (it is the accessible name source). When there is no history the `<select>` is `null` and only the `history` label + the right-hand control show; the label is innocuous next to the active control. No new tokens introduced — reuses `label`, `select`, `empty`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd /Users/tim/projects/agent-bus-app && npx vitest run src/components/RunSelector.test.tsx 2>&1 | tail -25`
Expected: PASS (all RunSelector tests, including the new history/gating ones and the unchanged `● Running`/`Stop`/`Resume`/`Start` tests).

- [ ] **Step 5: Commit**

```bash
cd /Users/tim/projects/agent-bus-app
git add src/components/RunSelector.tsx src/components/RunSelector.test.tsx
git commit -m "feat(ui): demote run dropdown to a history affordance (LF33)"
```

---

### Task 4: Confirm the loop closes (● Running reflects the active run after run-changed) + full gates

**Files:**
- Test: `src/hooks/useRuns.test.ts` (extend if a Start-then-refetch case is missing)
- Verify only: no production change expected here — Tasks 1-3 close the loop.

- [ ] **Step 1: Inspect the existing useRuns test for a refetch-on-run-changed case**

Run: `cd /Users/tim/projects/agent-bus-app && cat src/hooks/useRuns.test.ts`
Expected: confirm there is a test that `onRunChanged` triggers `reload`/refetch and `activeRun` becomes the newest incomplete run. If such a test exists, this step is a no-op confirmation; if NOT, add the test in Step 2.

- [ ] **Step 2: (Only if missing) add a refetch-derives-active-run test**

If Step 1 shows no coverage that a `run-changed` event refetches and re-derives `activeRun`, add to `src/hooks/useRuns.test.ts` (adapt the mock style already used in that file — mock `listRuns` from `../ipc/runtime` and the `useRuntimeEvents` `onRunChanged` callback):

```ts
  it("refetches on run-changed and derives the newest incomplete run as active", async () => {
    // first fetch: no runs; second fetch (after run-changed): one incomplete run.
    listRunsMock
      .mockResolvedValueOnce([])
      .mockResolvedValueOnce([
        { id: "R-new", pipeline: "p", project_id: "proj", generator_dry: false, completed: false, created_at: 1 },
      ]);
    const { result } = renderHook(() => useRuns("proj"));
    await waitFor(() => expect(result.current.activeRun).toBeNull());
    // simulate the backend run-changed emit closing the loop
    act(() => onRunChangedCb());
    await waitFor(() => expect(result.current.activeRun?.id).toBe("R-new"));
  });
```

> If `useRuns.test.ts` already exposes `listRunsMock` / `onRunChangedCb` (or equivalent) reuse them; otherwise match the file's existing mock wiring exactly. If coverage already exists, skip this step.

- [ ] **Step 3: Run the full verification gates**

Run: `cd /Users/tim/projects/agent-bus-app/src-tauri && cargo test 2>&1 | tail -30`
Expected: PASS — all runtime + app tests green (includes Task 1 + Task 2 tests).

Run: `cd /Users/tim/projects/agent-bus-app/src-tauri && cargo clippy --all-targets -- -D warnings 2>&1 | tail -30`
Expected: no warnings (clean exit). Fix any `clippy` lints introduced by the new code (e.g. needless borrows) before proceeding.

Run: `cd /Users/tim/projects/agent-bus-app/src-tauri && cargo build 2>&1 | tail -15`
Expected: builds clean.

Run: `cd /Users/tim/projects/agent-bus-app && npx vitest run 2>&1 | tail -30`
Expected: PASS — all frontend tests (RunSelector + useRuns + the rest).

Run: `cd /Users/tim/projects/agent-bus-app && npx tsc --noEmit 2>&1 | tail -20`
Expected: no type errors.

- [ ] **Step 4: Commit (only if Step 2 added a test)**

```bash
cd /Users/tim/projects/agent-bus-app
git add src/hooks/useRuns.test.ts
git commit -m "test(ui): run-changed refetch derives active run (LF33)"
```

---

## Self-Review

- **Spec coverage:** (1) Root idempotent `start_run` wrapper that emits `RUN_CHANGED` + resumes brake → Tasks 1-2. (2) Demote dropdown to history → Task 3. (3) Confirm `● Running`/`Stop` reflects active run after `run-changed` → Task 4. All three scope items covered.
- **Tauri-unaware runtime:** the resolver in Task 1 takes only `&RuntimeState`; the emit + brake-persist live in the app-crate command in Task 2. Confirmed.
- **Type consistency:** `start_or_resume_run_inner` / `StartOutcome` / `should_clear_brake` names used identically across tasks; `run.id` is the emit payload (matches the existing `inject_topic` emit at `lib.rs:1171`).
- **No second run on double-press:** Task 1's `start_or_resume_returns_existing_active_without_creating_a_second` asserts exactly one run after two presses.

## Risks / Watch-outs

- **`Run` type path in the app command return:** confirm whether it is `runtime::model::Run` or re-exported as `runtime::api::Run` (whatever `start_run_inner` returns). The plan flags this in Task 2 Step 3; pick the path that `cargo build` accepts.
- **`brake_persist::BrakeStore` State availability:** the command takes `tauri::State<'_, Arc<brake_persist::BrakeStore>>` exactly like `brake_off`; this is only registered if the app manages that state (it does, since `brake_off` works). If a test harness ever calls `start_run` without that state managed it would panic — but the command is only reached via the real invoke_handler, same as `brake_off`.
- **Resume semantics on Start:** clearing the brake on a Resumed+braked run mirrors `brake_off` but does NOT call `registry.end_killing()` (the kill latch). If the operator expects Start-while-stopped to fully re-enable worker spawning identically to the topbar Resume, consider adding `registry: tauri::State<'_, Arc<process_registry::ProcessRegistry>>` + `registry.end_killing()` to the command. The plan keeps Start minimal (brake clear + persist only); the dedicated Resume control remains the full-fidelity resume. Flag for the reviewer to decide.
- **`AppHandle` injection:** Tauri injects `app: tauri::AppHandle` into commands automatically (no `tauri::State` wrapper). The wrapper can't be unit-tested end-to-end for the emit; Task 2 tests the pure `should_clear_brake` predicate instead. The emit itself is exercised by the existing manual/e2e flow.
- **e2e/playwright:** there may be Playwright specs asserting the old `run` dropdown label or its always-present dropdown. After Task 3, grep `e2e/` and `playwright/` for `combobox`/`run`/`history` and update any spec that depends on the old label — not in the verification gates above but worth a pass before merge.
