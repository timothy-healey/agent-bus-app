# P2 — Early-cancel outstanding lanes on first reject — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an opt-in per-join `cancel_on_reject` policy so that when one lane of a fork/join group fails (reject, or revise-cap → needs-human), the group resolves to needs-human IMMEDIATELY without waiting for the other in-flight lanes to finish, and the outstanding lane tasks stop running.

**Architecture:** The early-cancel decision is owned by the `FanOutGroup` aggregate (the barrier owner). A new store method `record_failure_and_early_cancel` completes the group to needs-human via the SAME `UPDATE fanout_groups SET completed=1 WHERE id=? AND completed=0` single-writer guard the full barrier already uses — so exactly-once-completion still holds. When the failing lane wins that guard, the pool (a) creates the single needs-human continuation and (b) cancels the group's not-yet-finished lane tasks (queued/revising) by transitioning them to the existing terminal `Done` state parked at their join, so queued workers don't claim+run them. Outstanding lanes that settle later find the group already `completed` and no-op (`BarrierOutcome::Parked`). The default full-barrier path is untouched.

**Tech Stack:** Rust (cargo workspace under `src-tauri/`), sqlx + in-memory SQLite for tests, serde; TypeScript IPC types + vitest for the frontend contract.

---

## Decisions

- **DD1 — Schema shape.** `Join.cancel_on_reject: bool` with `#[serde(default)]` (defaults `false`). Additive-optional; **no `SCHEMA_VERSION` bump** (still 2). Existing v1/v2 pipelines load unchanged with the full-barrier behavior.
- **DD2 — Policy owner.** The early-cancel policy lives in the `FanOutGroup` aggregate / `FanOutStore`. The pool reads `join.cancel_on_reject` from the pipeline and decides WHICH store call to make; the store owns the completes-once guard either way. (Matches the parallel-flow vet: "the `FanOutGroup` aggregate is the natural owner of that policy.")
- **DD3 — Exactly-once.** Early-cancel completes the group through the identical `WHERE completed=0` conditional UPDATE. A concurrent straggler that also tries to complete (full-barrier or early) sees `rows_affected==0` and parks. Exactly one continuation, always.
- **DD4 — Cancelling lane tasks.** No new `TaskState`. A new `TaskStore::cancel_outstanding_lanes(group_id, now)` transitions the group's still-active lane tasks (`state IN ('queued','revising')`) to `Done`, parked at their `join_target` (mirrors how `resolve_barrier` already parks a settled lane task as `Done`). This is a direct conditional UPDATE — like the existing barrier park, it does not route through `Task::transition_to` (the barrier already sets `state = Done` by hand). Running tasks are left alone (we never kill a claude subprocess — DD5); when they later settle they hit the already-`completed` guard and no-op.
- **DD5 — Don't kill running work.** Cancellation only stops the FLOW: it prevents queued lanes from being claimed and prevents any late continuation. A lane already `running` finishes its current invocation; its settle finds the group `completed` and parks without creating a continuation. We never signal/kill the subprocess.
- **DD6 — Failure trigger.** Early-cancel fires for a lane settling to the barrier with `Verdict::Reject` (the same signal the full barrier already routes via `resolve_barrier(... Reject)` when a lane task routes to needs-human — covers both an explicit reject AND the revise-cap escalation, since both reach the barrier as a reject per Decision D5). On `Verdict::Approve` the early path behaves exactly like the full barrier (record + try-complete only when all settled).
- **DD7 — Frontend surface.** Add `cancel_on_reject?: boolean` to the `Join` IPC type so a `cancel_on_reject` join round-trips through load/save and the contract test. Authoring UI affordance (a wizard toggle) is deferred to the impeccable pass — note it; backend + IPC type only here.

---

## File Structure

- `src-tauri/pipeline/src/model.rs` — add `cancel_on_reject: bool` to `Join` (`#[serde(default)]`).
- `src-tauri/pipeline/src/contract_tests.rs` — Join field-set assertion (only if it pins Join's keys; it currently pins the Pipeline top-level keys, so likely no change — verify).
- `src-tauri/runtime/src/fanout_store.rs` — new `record_failure_and_early_cancel` method on `FanOutStore` (completes-to-needs-human via the guard, regardless of pending lanes).
- `src-tauri/runtime/src/task_store.rs` — new `cancel_outstanding_lanes(group_id, now)` conditional UPDATE.
- `src-tauri/runtime/src/pool.rs` — `resolve_barrier` reads `join.cancel_on_reject` and, on a failing lane under that policy, calls the early-cancel store path + cancels outstanding lanes.
- `src/ipc/pipeline.ts` — add `cancel_on_reject?: boolean` to the `Join` interface.
- `src/ipc/pipeline.test.ts` — round-trip a `cancel_on_reject` join (if the test fixture is the place; else add an assertion).

---

## Task 1: Schema — `Join.cancel_on_reject`

**Files:**
- Modify: `src-tauri/pipeline/src/model.rs` (the `Join` struct ~line 183, and its test usages)
- Test: same file, `#[cfg(test)] mod tests`

- [ ] **Step 1: Write the failing test**

Add to `model.rs` tests:

```rust
    #[test]
    fn join_cancel_on_reject_defaults_to_false_when_absent() {
        // a join authored without the field loads with the full-barrier default
        let json = r#"{"id":"join-1","waits_for":["a","b"],"downstream":"after"}"#;
        let j: Join = serde_json::from_str(json).unwrap();
        assert!(!j.cancel_on_reject);
    }

    #[test]
    fn join_round_trips_cancel_on_reject_true() {
        let j = Join {
            id: "join-1".into(),
            waits_for: vec!["a".into(), "b".into()],
            downstream: "after".into(),
            cancel_on_reject: true,
        };
        let s = serde_json::to_string(&j).unwrap();
        let back: Join = serde_json::from_str(&s).unwrap();
        assert_eq!(j, back);
        assert!(back.cancel_on_reject);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test -p pipeline join_cancel_on_reject 2>&1 | tail -20`
Expected: compile error — `Join` has no field `cancel_on_reject`.

- [ ] **Step 3: Add the field to `Join`**

In `model.rs`, change the `Join` struct to:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Join {
    pub id: String,
    pub waits_for: Vec<String>,
    pub downstream: String,
    /// Early-cancel policy (P2). When true, the join resolves to needs-human the
    /// moment ONE lane fails (reject / revise-cap), cancelling the outstanding
    /// lanes instead of waiting for the full barrier. `#[serde(default)]` false
    /// keeps existing pipelines on the full-barrier behavior; additive-optional,
    /// no SCHEMA_VERSION bump. The policy is enforced by the FanOutGroup barrier.
    #[serde(default)]
    pub cancel_on_reject: bool,
}
```

- [ ] **Step 4: Fix existing `Join { .. }` literals in this file**

Every `Join { id: ..., waits_for: ..., downstream: ... }` in `model.rs` tests now needs `cancel_on_reject: false`. Locate them:

Run: `cd src-tauri && grep -n "Join {" pipeline/src/model.rs`

Add `cancel_on_reject: false,` to each (the two in `node_ids_includes_forks_and_joins` and any others). Example edit — change:

```rust
            joins: vec![Join { id: "join-1".into(), waits_for: vec!["a".into(), "b".into()], downstream: "research".into() }],
```
to:
```rust
            joins: vec![Join { id: "join-1".into(), waits_for: vec!["a".into(), "b".into()], downstream: "research".into(), cancel_on_reject: false }],
```

- [ ] **Step 5: Run the pipeline crate tests**

Run: `cd src-tauri && cargo test -p pipeline 2>&1 | tail -30`
Expected: PASS, including the two new tests. If other `Join { .. }` literals across the crate (e.g. `validate.rs`) fail to compile, add `cancel_on_reject: false` to them too:

Run: `cd src-tauri && grep -rn "Join {" pipeline/src/ | grep -v "cancel_on_reject"`
Fix each, then re-run.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/pipeline/src/
git commit -m "feat(pipeline): add Join.cancel_on_reject (additive, default false)

P2 early-cancel: opt-in per-join policy field. serde(default) false keeps
existing pipelines on the full-barrier behavior; no SCHEMA_VERSION bump.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: Fix downstream `Join { .. }` literals in the runtime crate

The runtime crate constructs `Join` in `pool.rs` tests (`pipeline_v2_forkjoin`). It won't compile until those get the new field.

**Files:**
- Modify: `src-tauri/runtime/src/pool.rs` (test helpers)

- [ ] **Step 1: Find every Join literal in runtime**

Run: `cd src-tauri && grep -rn "Join {" runtime/src/ | grep -v "cancel_on_reject"`

- [ ] **Step 2: Add `cancel_on_reject: false` to each**

In `pool.rs`'s `pipeline_v2_forkjoin`, change:
```rust
            joins: vec![Join { id: "join-1".into(), waits_for: vec!["lane-a".into(), "lane-b".into()], downstream: "after".into() }],
```
to:
```rust
            joins: vec![Join { id: "join-1".into(), waits_for: vec!["lane-a".into(), "lane-b".into()], downstream: "after".into(), cancel_on_reject: false }],
```
Apply to every match from Step 1.

- [ ] **Step 3: Verify the runtime crate compiles + existing tests stay green**

Run: `cd src-tauri && cargo test -p runtime 2>&1 | tail -30`
Expected: PASS (all existing fork/join + barrier tests green — the full-barrier default is unchanged).

- [ ] **Step 4: Commit**

```bash
git add src-tauri/runtime/src/pool.rs
git commit -m "test(runtime): thread Join.cancel_on_reject=false through fixtures

Keeps the runtime crate compiling after the additive Join field; the
full-barrier default path is unchanged.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: Aggregate + Store — early-cancel completes the group to needs-human via the guard

Add a sibling to `record_and_try_complete` that does NOT wait for all lanes: it records the failing lane's verdict, then attempts the same completes-once guard and, on winning, asks the `FanOutGroup` aggregate for the early-cancel continuation (keeping the aggregation rule on the aggregate root — vet F1).

**Files:**
- Modify: `src-tauri/runtime/src/fanout_group.rs` (the aggregate — vet F1)
- Modify: `src-tauri/runtime/src/fanout_store.rs`
- Test: both files, `#[cfg(test)] mod tests`

- [ ] **Step 0a (vet F1): Write the failing aggregate test**

Add to `fanout_group.rs` tests:

```rust
    #[test]
    fn early_cancel_continuation_is_needs_human() {
        // The FanOutGroup owns the early-cancel aggregation rule: a failed lane
        // forces needs-human, regardless of the other (unsettled) lanes.
        let g = group();
        assert_eq!(g.early_cancel_continuation(), Continuation::NeedsHuman);
    }
```

- [ ] **Step 0b: Run it to verify it fails**

Run: `cd src-tauri && cargo test -p runtime early_cancel_continuation 2>&1 | tail -15`
Expected: compile error — no method `early_cancel_continuation`.

- [ ] **Step 0c: Implement the method on `FanOutGroup`**

In `fanout_group.rs`, add to `impl FanOutGroup` (next to `continuation`):

```rust
    /// The continuation when the early-cancel policy (P2) fires: a lane has
    /// failed, so the group resolves to needs-human WITHOUT waiting for the other
    /// lanes. This is the aggregate root's early-cancel rule — the store asks for
    /// it after winning the completes-once guard, mirroring how the full barrier
    /// asks `continuation()`. Keeping it here (not hardcoded in the store) means a
    /// future divergent join policy has one home (vet F1).
    pub fn early_cancel_continuation(&self) -> Continuation {
        Continuation::NeedsHuman
    }
```

- [ ] **Step 0d: Run the aggregate test**

Run: `cd src-tauri && cargo test -p runtime early_cancel_continuation 2>&1 | tail -15`
Expected: PASS.

- [ ] **Step 1: Write the failing tests**

Add to the `fanout_store.rs` tests:

```rust
    #[tokio::test]
    async fn early_cancel_completes_to_needs_human_without_waiting_for_other_lanes() {
        let store = FanOutStore::new(fresh_pool().await);
        seeded(&store).await; // lanes a + b, both 'pending'
        // lane-a fails; with early-cancel the group completes NOW even though
        // lane-b is still pending.
        let outcome = store
            .record_failure_and_early_cancel("G-1", "lane-a", Verdict::Reject)
            .await
            .unwrap();
        assert_eq!(outcome, BarrierOutcome::Completed(Continuation::NeedsHuman));
        // group is marked completed
        assert!(store.load("G-1").await.unwrap().completed);
    }

    #[tokio::test]
    async fn early_cancel_is_exactly_once_against_a_concurrent_straggler() {
        // THE CRUX for P2: the failing lane early-cancels while the other lane
        // settles concurrently. Exactly one of them wins the completes-once guard.
        let store = std::sync::Arc::new(FanOutStore::new(fresh_pool().await));
        seeded(&store).await;
        let s1 = store.clone();
        let s2 = store.clone();
        let h1 = tokio::spawn(async move {
            s1.record_failure_and_early_cancel("G-1", "lane-a", Verdict::Reject).await.unwrap()
        });
        let h2 = tokio::spawn(async move {
            s2.record_and_try_complete("G-1", "lane-b", Verdict::Approve).await.unwrap()
        });
        let (a, b) = (h1.await.unwrap(), h2.await.unwrap());
        let completions = [&a, &b].iter().filter(|o| matches!(o, BarrierOutcome::Completed(_))).count();
        assert_eq!(completions, 1, "exactly one caller completes the barrier");
        // and if the early-cancel won, it completed to needs-human
        if let BarrierOutcome::Completed(c) = &a {
            assert_eq!(*c, Continuation::NeedsHuman);
        }
    }

    #[tokio::test]
    async fn early_cancel_after_group_already_completed_parks() {
        let store = FanOutStore::new(fresh_pool().await);
        seeded(&store).await;
        // first failing lane early-cancels (completes)
        store.record_failure_and_early_cancel("G-1", "lane-a", Verdict::Reject).await.unwrap();
        // a straggler lane settles later -> group already completed -> parks
        let again = store.record_and_try_complete("G-1", "lane-b", Verdict::Reject).await.unwrap();
        assert_eq!(again, BarrierOutcome::Parked);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test -p runtime early_cancel 2>&1 | tail -20`
Expected: compile error — `record_failure_and_early_cancel` does not exist.

- [ ] **Step 3: Implement the method**

In `fanout_store.rs`, add to `impl FanOutStore` (after `record_and_try_complete`):

```rust
    /// Early-cancel path (P2): record this failing lane's verdict, then complete
    /// the group to needs-human IMMEDIATELY — without waiting for the other lanes
    /// — using the SAME completes-once conditional UPDATE guard as the full
    /// barrier. This is the FanOutGroup aggregate's early-cancel policy; the
    /// caller invokes it only when the lane's join has `cancel_on_reject` and the
    /// lane settled as a failure (Verdict::Reject — covers explicit reject and
    /// revise-cap escalation per Decision D5).
    ///
    /// Returns `Completed(NeedsHuman)` for the sole winner; `Parked` if the group
    /// was already completed (a straggler / a concurrent settle won first). The
    /// exactly-one-completion invariant is preserved: the same `WHERE completed=0`
    /// row guard arbitrates between this path and `record_and_try_complete`.
    pub async fn record_failure_and_early_cancel(
        &self,
        group_id: &str,
        lane: &str,
        verdict: Verdict,
    ) -> Result<BarrierOutcome, FanOutStoreError> {
        // 1. Upsert this lane's verdict (idempotent on re-record — D6).
        sqlx::query(
            "INSERT INTO fanout_lanes (group_id, lane, verdict) VALUES (?,?,?)
             ON CONFLICT(group_id, lane) DO UPDATE SET verdict=excluded.verdict",
        )
        .bind(group_id)
        .bind(lane)
        .bind(verdict_str(verdict))
        .execute(&self.pool)
        .await?;

        // 2. Attempt the completes-once guard NOW — do not wait for other lanes.
        let res = sqlx::query("UPDATE fanout_groups SET completed=1 WHERE id=? AND completed=0")
            .bind(group_id)
            .execute(&self.pool)
            .await?;
        if res.rows_affected() == 0 {
            // Already completed (full barrier or another early-cancel won).
            return Ok(BarrierOutcome::Parked);
        }

        // 3. Sole winner: ask the aggregate root for the early-cancel
        //    continuation (keeps the aggregation rule on FanOutGroup — vet F1).
        let g = self.load(group_id).await?;
        Ok(BarrierOutcome::Completed(g.early_cancel_continuation()))
    }
```

- [ ] **Step 4: Run the tests**

Run: `cd src-tauri && cargo test -p runtime early_cancel 2>&1 | tail -20`
Expected: PASS (3 new tests).

- [ ] **Step 5: Run the whole runtime crate (regression)**

Run: `cd src-tauri && cargo test -p runtime 2>&1 | tail -15`
Expected: PASS — all existing barrier tests unchanged.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/runtime/src/fanout_group.rs src-tauri/runtime/src/fanout_store.rs
git commit -m "feat(runtime): FanOutGroup early-cancel completes to needs-human via the guard

FanOutGroup::early_cancel_continuation keeps the aggregation rule on the
aggregate root (vet F1); record_failure_and_early_cancel records the failing
lane and completes the group to needs-human immediately via the same WHERE
completed=0 single-writer guard — exactly-once-completion is preserved, incl.
against a concurrent straggler-settle.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: State machine + Store — cancel outstanding lane tasks of a group

A conditional UPDATE that parks the group's still-active lane tasks so queued ones aren't claimed and run wastefully. First make the early-cancel-park edge visible in the Task state machine (vet F2) — the bulk UPDATE moves `queued`/`revising` lanes to `done`, an edge `Task::can_transition_to` currently forbids; encode it explicitly so it isn't later "fixed" as a bug.

**Files:**
- Modify: `src-tauri/runtime/src/task.rs` (the state machine — vet F2)
- Modify: `src-tauri/runtime/src/task_store.rs`
- Test: both files, `#[cfg(test)] mod tests`

- [ ] **Step 0a (vet F2): Write the failing state-machine test**

Add to `task.rs` tests:

```rust
    #[test]
    fn early_cancel_park_edges_are_legal() {
        // P2 early-cancel parks an outstanding lane (queued or revising) to done.
        // These edges are intentional (see TaskStore::cancel_outstanding_lanes);
        // assert them so they aren't later removed as illegal.
        let mut q = t(); // queued
        assert!(q.can_transition_to(TaskState::Done));
        assert!(q.transition_to(TaskState::Done, 200).is_ok());

        let mut r = t();
        r.transition_to(TaskState::Running, 1).unwrap();
        r.transition_to(TaskState::Revising, 2).unwrap();
        assert!(r.can_transition_to(TaskState::Done));
        assert!(r.transition_to(TaskState::Done, 3).is_ok());
    }
```

- [ ] **Step 0b: Run it to verify it fails**

Run: `cd src-tauri && cargo test -p runtime early_cancel_park_edges 2>&1 | tail -15`
Expected: FAIL — `queued→done` / `revising→done` are illegal transitions.

- [ ] **Step 0c: Add the edges to `can_transition_to`**

In `task.rs`, in `can_transition_to`'s match, add the early-cancel-park edges. Change:

```rust
            (Queued, Running) | (Queued, Braked) => true,
```
to:
```rust
            (Queued, Running) | (Queued, Braked) => true,
            // P2 early-cancel parks an outstanding lane task to done (queued or
            // revising), so the group's resolution isn't blocked by lanes that
            // will never run. See TaskStore::cancel_outstanding_lanes.
            (Queued, Done) | (Revising, Done) => true,
```

- [ ] **Step 0d: Run the state-machine test**

Run: `cd src-tauri && cargo test -p runtime early_cancel_park_edges 2>&1 | tail -15`
Expected: PASS. Then re-run the existing task tests to confirm no regression:

Run: `cd src-tauri && cargo test -p runtime --lib task:: 2>&1 | tail -20`
Expected: PASS (e.g. `illegal_transition_is_rejected` still passes — it tests queued→gated, still illegal).

- [ ] **Step 1: Write the failing test**

Add to `task_store.rs` tests:

```rust
    #[tokio::test]
    async fn cancel_outstanding_lanes_parks_queued_and_revising_not_running_or_done() {
        let store = TaskStore::new(fresh_pool().await);
        let parent = task("entry");
        // queued lane task in group G-1
        let queued = Task::forked(&parent, "lane-a", "G-1", "join-1", 500);
        store.insert(&queued).await.unwrap();
        // a running lane task in the same group (must be left alone — DD5)
        let mut running = Task::forked(&parent, "lane-b", "G-1", "join-1", 500);
        running.state = TaskState::Running;
        store.insert(&running).await.unwrap();
        // a lane task in a DIFFERENT group (must be untouched)
        let other = Task::forked(&parent, "lane-c", "G-2", "join-2", 500);
        store.insert(&other).await.unwrap();

        let cancelled = store.cancel_outstanding_lanes("G-1", 900).await.unwrap();
        assert_eq!(cancelled, 1, "only the queued lane in G-1 is parked");

        // queued -> Done, parked at its join_target
        let q = store.get(&queued.id).await.unwrap();
        assert_eq!(q.state, TaskState::Done);
        assert_eq!(q.current_stage, "join-1");
        // running is left to finish (DD5)
        assert_eq!(store.get(&running.id).await.unwrap().state, TaskState::Running);
        // other group untouched
        assert_eq!(store.get(&other.id).await.unwrap().state, TaskState::Queued);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test -p runtime cancel_outstanding_lanes 2>&1 | tail -20`
Expected: compile error — `cancel_outstanding_lanes` does not exist.

- [ ] **Step 3: Implement the method**

In `task_store.rs`, add to `impl TaskStore`:

```rust
    /// Early-cancel (P2): park the still-active lane tasks of a fan-out group so
    /// queued ones are never claimed and run wastefully once the group has
    /// already resolved (to needs-human). Transitions tasks of `group_id` whose
    /// state is `queued` or `revising` to `done`, parked at their `join_target`
    /// (mirrors how the barrier parks a settled lane task). A `running` lane is
    /// deliberately left alone (DD5: never kill a live invocation — its later
    /// settle hits the already-`completed` group guard and no-ops). Returns the
    /// count parked. Like `claim_next_for_stage`, this is a direct conditional
    /// UPDATE — the early-cancel park does not route through Task::transition_to,
    /// consistent with the barrier's existing direct `state = Done` park.
    pub async fn cancel_outstanding_lanes(
        &self,
        group_id: &str,
        now_unix: i64,
    ) -> Result<u64, TaskStoreError> {
        let res = sqlx::query(
            "UPDATE tasks
             SET state='done',
                 current_stage=COALESCE(join_target, current_stage),
                 updated_at=?
             WHERE group_id=? AND state IN ('queued','revising')",
        )
        .bind(now_unix)
        .bind(group_id)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected())
    }
```

- [ ] **Step 4: Run the test**

Run: `cd src-tauri && cargo test -p runtime cancel_outstanding_lanes 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/runtime/src/task.rs src-tauri/runtime/src/task_store.rs
git commit -m "feat(runtime): TaskStore.cancel_outstanding_lanes parks queued/revising lanes

Early-cancel (P2): once a group resolves to needs-human, park its still-
active lane tasks (queued/revising -> done at join_target) so queued workers
don't claim+run them. Running lanes are left to finish (never kill a live
invocation); their later settle no-ops on the completed guard.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 5: Pool — wire the policy into the barrier-resolve path

`resolve_barrier` currently always calls `record_and_try_complete`. When the lane settles as a **reject** AND its join has `cancel_on_reject`, it should instead early-cancel + park the outstanding lanes.

**Files:**
- Modify: `src-tauri/runtime/src/pool.rs` (`resolve_barrier`)
- Test: `pool.rs` tests

- [ ] **Step 1: Write the failing tests**

Add to `pool.rs` tests (the `StageRunner` + `pipeline_v2_forkjoin` helpers already exist; add a helper that flips the join's flag).

```rust
    fn pipeline_v2_forkjoin_cancel_on_reject() -> Pipeline {
        let mut p = pipeline_v2_forkjoin();
        p.joins[0].cancel_on_reject = true;
        p
    }

    #[tokio::test]
    async fn early_cancel_resolves_to_needs_human_before_other_lane_runs() {
        let pool = fresh_pool().await;
        let p = pipeline_v2_forkjoin_cancel_on_reject();
        let reject = RunnerOutput { verdict: Verdict::Reject, artifact_path: None, final_text: "VERDICT: reject".into(), usage: RunnerUsage::default() };
        let ctx = ctx_with(pool.clone(), p.clone(), Arc::new(StageRunner::new(reject)), temp_root());
        let t = Task::injected("proj".into(), "p".into(), "entry".into(), "topic".into(), Some("/repo".into()), 100);
        ctx.tasks.insert(&t).await.unwrap();

        process_one_claim(&ctx, &p.teams[0]).await.unwrap(); // entry -> fork -> lane-a, lane-b queued
        // lane-b rejects FIRST, before lane-a ever runs.
        process_one_claim(&ctx, &p.teams[2]).await.unwrap(); // lane-b reject -> early-cancel

        // the joined task escalates to needs-human immediately
        let nh = ctx.tasks.list_by_state(TaskState::NeedsHuman).await.unwrap();
        assert_eq!(nh.len(), 1);
        assert_eq!(nh[0].current_stage, "needs-human");

        // lane-a was parked (cancelled) — it is no longer queued, so a worker
        // claiming lane-a finds nothing to run.
        let queued = ctx.tasks.list_by_state(TaskState::Queued).await.unwrap();
        assert!(queued.iter().all(|q| q.current_stage != "lane-a"),
            "outstanding lane-a is cancelled, not left queued");
        let idle = process_one_claim(&ctx, &p.teams[1]).await.unwrap(); // lane-a
        assert_eq!(idle, ClaimOutcome::Idle, "no outstanding lane work remains");
    }

    #[tokio::test]
    async fn early_cancel_straggler_settle_creates_no_second_continuation() {
        // lane-a parks (approve) first; lane-b then rejects with cancel_on_reject.
        // Exactly one needs-human continuation; the early-cancel guard arbitrates.
        let pool = fresh_pool().await;
        let p = pipeline_v2_forkjoin_cancel_on_reject();
        let reject = RunnerOutput { verdict: Verdict::Reject, artifact_path: None, final_text: "VERDICT: reject".into(), usage: RunnerUsage::default() };
        let ctx = ctx_with(pool.clone(), p.clone(), Arc::new(StageRunner::new(reject)), temp_root());
        let t = Task::injected("proj".into(), "p".into(), "entry".into(), "topic".into(), Some("/repo".into()), 100);
        ctx.tasks.insert(&t).await.unwrap();

        process_one_claim(&ctx, &p.teams[0]).await.unwrap(); // fork
        process_one_claim(&ctx, &p.teams[1]).await.unwrap(); // lane-a approve (parks; group not complete yet)
        process_one_claim(&ctx, &p.teams[2]).await.unwrap(); // lane-b reject -> early-cancel completes

        let nh = ctx.tasks.list_by_state(TaskState::NeedsHuman).await.unwrap();
        assert_eq!(nh.len(), 1, "exactly one needs-human continuation");
    }

    #[tokio::test]
    async fn default_join_keeps_full_barrier_even_with_a_reject() {
        // cancel_on_reject = false (default): the reject does NOT short-circuit;
        // both lanes settle, then the group routes to needs-human as before.
        let pool = fresh_pool().await;
        let p = pipeline_v2_forkjoin(); // cancel_on_reject defaults false
        let reject = RunnerOutput { verdict: Verdict::Reject, artifact_path: None, final_text: "VERDICT: reject".into(), usage: RunnerUsage::default() };
        let ctx = ctx_with(pool.clone(), p.clone(), Arc::new(StageRunner::new(reject)), temp_root());
        let t = Task::injected("proj".into(), "p".into(), "entry".into(), "topic".into(), Some("/repo".into()), 100);
        ctx.tasks.insert(&t).await.unwrap();

        process_one_claim(&ctx, &p.teams[0]).await.unwrap(); // fork
        process_one_claim(&ctx, &p.teams[2]).await.unwrap(); // lane-b reject FIRST

        // full barrier: NOT yet escalated — lane-a is still outstanding (queued).
        let nh = ctx.tasks.list_by_state(TaskState::NeedsHuman).await.unwrap();
        assert!(nh.is_empty(), "full barrier waits for all lanes before escalating");
        let queued = ctx.tasks.list_by_state(TaskState::Queued).await.unwrap();
        assert!(queued.iter().any(|q| q.current_stage == "lane-a"),
            "lane-a remains queued under the full barrier");

        // lane-a settles -> NOW the group completes to needs-human.
        process_one_claim(&ctx, &p.teams[1]).await.unwrap();
        let nh = ctx.tasks.list_by_state(TaskState::NeedsHuman).await.unwrap();
        assert_eq!(nh.len(), 1);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test -p runtime early_cancel_resolves early_cancel_straggler default_join_keeps 2>&1 | tail -25`
Expected: `early_cancel_resolves_*` and `early_cancel_straggler_*` FAIL (full barrier still parks lane-b's reject and waits); `default_join_keeps_*` should PASS already (it asserts the existing behavior).

- [ ] **Step 3: Implement the policy branch in `resolve_barrier`**

In `pool.rs`, replace the body of `resolve_barrier` so the failing-lane/early-cancel case is handled. The current function records via `record_and_try_complete` unconditionally; change it to choose the early-cancel path when the verdict is a reject AND the lane's join has `cancel_on_reject`. Full replacement:

```rust
async fn resolve_barrier(
    ctx: &PoolContext,
    task: &mut Task,
    verdict: agent_bus_core::Verdict,
) -> Result<Option<(String, TaskState)>, PoolError> {
    let group_id = task.group_id.clone().ok_or(PoolError::Route(RouteError::NoRoute))?;
    let lane = task.lane.clone().unwrap_or_default();
    let now = now_unix();

    // Does this lane's join opt into early-cancel? (P2) Read it from the pipeline
    // by the lane task's join_target. Default false => full-barrier behavior.
    let early_cancel = task
        .join_target
        .as_deref()
        .and_then(|jt| ctx.pipeline.joins.iter().find(|j| j.id == jt))
        .map(|j| j.cancel_on_reject)
        .unwrap_or(false);

    let is_failure = verdict == agent_bus_core::Verdict::Reject;

    let outcome = if early_cancel && is_failure {
        ctx.fanout
            .record_failure_and_early_cancel(&group_id, &lane, verdict)
            .await?
    } else {
        ctx.fanout
            .record_and_try_complete(&group_id, &lane, verdict)
            .await?
    };

    // Park this lane task as terminal for the lane.
    task.state = TaskState::Done;
    task.current_stage = task.join_target.clone().unwrap_or_else(|| task.current_stage.clone());
    task.updated_at = now;
    ctx.tasks.update(task).await?;

    if let BarrierOutcome::Completed(cont) = outcome {
        // If WE won the early-cancel guard, stop the outstanding lanes so queued
        // siblings aren't claimed+run wastefully (DD4/DD5). Best-effort ordering:
        // the continuation is created regardless; cancellation only trims waste.
        if early_cancel && is_failure {
            ctx.tasks.cancel_outstanding_lanes(&group_id, now_unix()).await?;
        }
        let (stage, state) = match cont {
            Continuation::Downstream(ds) => (ds, TaskState::Queued),
            Continuation::NeedsHuman => ("needs-human".to_string(), TaskState::NeedsHuman),
        };
        let mut next = Task::injected(
            task.project_id.clone(),
            task.pipeline.clone(),
            stage.clone(),
            task.topic.clone(),
            task.target_repo.clone(),
            now,
        );
        next.state = state;
        next.parent_artifact = task.parent_artifact.clone();
        ctx.tasks.insert(&next).await?;
        return Ok(Some((stage, state)));
    }
    Ok(Some((task.current_stage.clone(), TaskState::Done)))
}
```

- [ ] **Step 4: Run the new tests**

Run: `cd src-tauri && cargo test -p runtime early_cancel_resolves early_cancel_straggler default_join_keeps 2>&1 | tail -25`
Expected: all PASS.

- [ ] **Step 5: Run the whole runtime crate (regression — full barrier unchanged)**

Run: `cd src-tauri && cargo test -p runtime 2>&1 | tail -15`
Expected: PASS, including the pre-existing `one_lane_reject_routes_join_to_needs_human`, `all_lanes_approve_*`, `partial_completion_*`, `restart_mid_group_*`.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/runtime/src/pool.rs
git commit -m "feat(runtime): early-cancel barrier resolve on a cancel_on_reject join

resolve_barrier reads join.cancel_on_reject; a lane rejecting under that
policy completes the group to needs-human immediately and parks the
outstanding lanes. Default joins keep the full-barrier path byte-for-byte.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 6: Frontend IPC type + contract

Add the field to the TS `Join` so a `cancel_on_reject` join round-trips through the typed IPC and the contract test.

**Files:**
- Modify: `src/ipc/pipeline.ts` (`Join` interface ~line 81)
- Test: `src/ipc/pipeline.test.ts`

- [ ] **Step 1: Inspect the existing IPC contract test**

Run: `grep -n "Join\|waits_for\|cancel_on_reject" src/ipc/pipeline.test.ts`
Identify the fixture that builds a Join (around line 60) — that's where to assert the round-trip.

- [ ] **Step 2: Write/extend the failing test**

In `src/ipc/pipeline.test.ts`, add an assertion that a Join with `cancel_on_reject` survives the typed shape. If there's a round-trip/echo test, set `cancel_on_reject: true` on the fixture join and assert it comes back; otherwise add:

```ts
import type { Join } from "./pipeline";

it("Join carries the optional cancel_on_reject early-cancel flag", () => {
  const j: Join = { id: "join-1", waits_for: ["a", "b"], downstream: "after", cancel_on_reject: true };
  expect(j.cancel_on_reject).toBe(true);
  // omitting it is valid (optional, defaults to full-barrier on the backend)
  const j2: Join = { id: "join-2", waits_for: ["a", "b"], downstream: "after" };
  expect(j2.cancel_on_reject).toBeUndefined();
});
```

- [ ] **Step 3: Run to verify it fails**

Run: `export PATH="/opt/homebrew/bin:$PATH" && bun vitest run src/ipc/pipeline.test.ts 2>&1 | tail -20`
Expected: type error — `cancel_on_reject` not on `Join`.

- [ ] **Step 4: Add the field to the `Join` interface**

In `src/ipc/pipeline.ts`:

```ts
export interface Join {
  id: string;
  waits_for: string[];
  downstream: string;
  /** Early-cancel policy (P2). When true, the join resolves to needs-human the
   *  moment one lane fails, cancelling the outstanding lanes. Optional; absent =
   *  the full-barrier default. Backend field is `cancel_on_reject` (serde default
   *  false). */
  cancel_on_reject?: boolean;
}
```

- [ ] **Step 5: Run the test**

Run: `export PATH="/opt/homebrew/bin:$PATH" && bun vitest run src/ipc/pipeline.test.ts 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/ipc/pipeline.ts src/ipc/pipeline.test.ts
git commit -m "feat(ipc): Join.cancel_on_reject optional field (P2 early-cancel)

Round-trips the early-cancel flag through the typed pipeline IPC. Authoring
UI affordance deferred to the impeccable pass.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 7: Docs — DOMAIN.md term + spec addendum (vet F3)

No code change. Register the new ubiquitous-language term and correct the now-stale "not cancelled" sentence in the parallel-flow spec.

**Files:**
- Modify: `DOMAIN.md` (Runtime ubiquitous language, ~line 85)
- Modify: `docs/superpowers/specs/2026-06-23-parallel-flow-design.md` (~line 12)

- [ ] **Step 1: Add the DOMAIN.md term**

In `DOMAIN.md`, after the `Fan-out group` bullet (~line 85), add:

```markdown
- **Early-cancel** — an opt-in per-join policy (`Join.cancel_on_reject`, P2): when one lane fails (reject / revise-cap), the fan-out group resolves to needs-human IMMEDIATELY and its outstanding lane tasks are cancelled, instead of waiting for the full barrier. Owned by the `FanOutGroup` aggregate; default off (full-barrier).
```

- [ ] **Step 2: Add the spec addendum**

In `docs/superpowers/specs/2026-06-23-parallel-flow-design.md`, the Decisions bullet (~line 12) ends with "in-flight siblings are not cancelled on an early failure — simpler and predictable". Append a sentence so the spec isn't stale:

```markdown
*(v1.1 update — P2: this full-barrier behavior is now the DEFAULT; a join may opt into `cancel_on_reject: true`, which cancels outstanding lanes and resolves to needs-human on the first lane failure. The `FanOutGroup` aggregate owns the policy; the completes-once guard is unchanged.)*
```

- [ ] **Step 3: Commit**

```bash
git add DOMAIN.md docs/superpowers/specs/2026-06-23-parallel-flow-design.md
git commit -m "docs: register Early-cancel term + spec addendum (P2, vet F3)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 8: Full verification

- [ ] **Step 1: cargo test (workspace)**

Run: `cd src-tauri && cargo test --workspace 2>&1 | tail -30`
Expected: all PASS.

- [ ] **Step 2: cargo check (workspace)**

Run: `cd src-tauri && cargo check --workspace 2>&1 | tail -10`
Expected: clean.

- [ ] **Step 3: cargo clippy (workspace)**

Run: `cd src-tauri && cargo clippy --workspace 2>&1 | tail -20`
Expected: no warnings. Fix any introduced.

- [ ] **Step 4: vitest**

Run: `export PATH="/opt/homebrew/bin:$PATH" && bun vitest run 2>&1 | tail -20`
Expected: all PASS.

- [ ] **Step 5: frontend build**

Run: `export PATH="/opt/homebrew/bin:$PATH" && bun run build 2>&1 | tail -20`
Expected: clean build.

---

## Self-Review checklist (run before merge)

- Spec coverage: cancel_on_reject join completes to needs-human on first reject WITHOUT waiting (Task 5 test 1); outstanding lanes no-op via the completed-guard + exactly-once incl. concurrent straggler (Task 3 tests 2&3, Task 5 test 2); default joins keep full-barrier (Task 5 test 3); all-approve → downstream (pre-existing `all_lanes_approve_*`, still green via Task 8). ✓
- Vet fixes applied: F1 (aggregate owns the early-cancel rule, Task 3 steps 0a–0d), F2 (state-machine edges, Task 4 steps 0a–0d), F3 (docs, Task 7). ✓
- No SCHEMA_VERSION bump (Task 1 keeps it 2). ✓
- No new TaskState (Task 4 reuses Done). ✓
- Exactly-once preserved via the single `WHERE completed=0` guard for both paths. ✓
