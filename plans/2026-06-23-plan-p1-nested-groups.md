# P1 — Nested Groups / Non-Linear Lanes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Lift the flat-linear-lane restriction so a fork lane may itself contain a gate and/or a nested fork (a child `FanOutGroup`), with the completes-exactly-once barrier invariant preserved at every level.

**Architecture:** Make `FanOutGroup` recursive via an optional parent link (`parent_group_id` + `parent_lane`). When a lane's settlement walk reaches a nested fork, the pool expands it into a child group whose `expected_lanes` are the nested fork's lanes; that child group's parent link points back at the enclosing group + the enclosing lane. When the child group completes, its continuation **settles the parent lane's verdict** (Approve if the child resolved Downstream, Reject if NeedsHuman) through the SAME `record_and_try_complete` single-writer guard the flat barrier uses. The exactly-once invariant therefore holds independently at each level — each group row is its own conditional-UPDATE guard, no cross-root transaction. Validation is relaxed to allow gates + nested forks inside a lane, bounded by a max nesting depth (3) and extended reachability checks.

**Tech Stack:** Rust (cargo workspace under `src-tauri/`), `sqlx` + SQLite, `tokio`. Frontend untouched (backend-heavy item).

---

## Decisions

- **DD-P1-1 — Parent link on the group, not the lane row.** `FanOutGroup` gains `parent_group_id: Option<String>` and `parent_lane: Option<String>`. A root group (top-level fork) has both `None`. A child group (nested fork inside a lane) carries the enclosing group id + the enclosing lane name. Rationale: the group is the aggregate root / consistency boundary; the parent link belongs on it so a child's completion knows exactly which parent lane to settle. (Recommended option, auto-decided.)
- **DD-P1-2 — Child completion settles the parent lane via the existing guard.** When a child group completes (its own barrier fires), the pool maps the child's `Continuation` to a parent lane verdict: `Downstream(_) ⇒ Verdict::Approve`, `NeedsHuman ⇒ Verdict::Reject`, then calls the parent group's barrier (`record_and_try_complete` / quorum / early-cancel as the parent join dictates) with `lane = parent_lane`. This reuses the single-writer `completed`-guard at the parent level unchanged. No new barrier primitive. (Recommended.)
- **DD-P1-3 — Nested-fork expansion mirrors top-level fork expansion.** The pool already expands a fork on approve in `settle_and_route`. The same code path handles a nested fork: the difference is the new child group carries the parent link. A lane task whose `on_approve` reaches a (nested) fork id is detected the same way a top-level entry team's is. (Recommended.)
- **DD-P1-4 — Gate inside a lane: no special runtime work.** Gates already park `Gated` and the operator's approve forwards to downstream via `route()`. A gate inside a lane simply means the lane task parks gated mid-lane; on operator approve it continues toward its join. The only blocker was validation rejecting it. Relaxing validation is the entire change for the gate case. (Recommended.)
- **DD-P1-5 — Max nesting depth = 3.** Validation computes each fork's nesting depth (a top-level fork is depth 1; a fork reached from inside another fork's lane is depth 2; etc.) and rejects depth > 3 with `NestingTooDeep`. Bounds pathological configs and keeps the bounded lane-walk terminating. (Recommended.)
- **DD-P1-6 — Migration 008 is append-only.** `008_nested_groups.sql` adds two nullable columns to `fanout_groups`; 001–007 untouched. Registered in BOTH lib.rs migration lists; the `user_version` migration test bumps 7→8. (Recommended.)
- **DD-P1-7 — Lane-walk becomes hierarchical, not flat.** `check_lane_linear` is replaced by `check_lane_reachable` which walks a lane from its entry, allowing team→team, team→gate→(gate.downstream), and team→fork (recurse into the nested fork's own lanes, which must each reach the nested fork's paired join, whose downstream continues the walk). Reaching the enclosing join ends the walk Ok. The runtime's `lane_reaches` is extended to match so fork/join pairing stays consistent. (Recommended.)
  - **VET F2 — termination + the depth bound.** The runtime walk (`lane_reaches`) and the recursive `finish_group`/`settle_parent_lane` climb are **node-count-bounded** for termination and **trust `validate.rs` for the depth bound** — Runtime only ever runs validated pipelines (`store::load` resolves+validates before any consumer sees a `Pipeline`). `MAX_NESTING_DEPTH` lives in `validate.rs` (Pipeline Authoring owns the authoring rule); the runtime does NOT duplicate the constant — the two contexts own different concerns (authoring rule vs loop termination). No shared kernel constant is introduced.
- **DD-P1-8 — Child group's "downstream" is the nested join's downstream, which must rejoin the parent lane toward the parent join.** The nested join's `downstream` node is a team (or gate) that continues toward the parent join. The child group's continuation creates a normal task at that downstream — EXCEPT when the child completes, instead of creating a downstream task we settle the parent lane (DD-P1-2). Wait — refined: a child group has TWO conceptual outcomes. See Task 6 for the precise rule: the child's continuation node is consumed as the parent-lane verdict signal; we do NOT also spawn a downstream task, because the parent lane's continuation past the parent join is what carries the flow forward. The nested join's `downstream` is still validated (must resolve) and recorded on the child group for completeness, but for a child group the pool routes completion to the parent barrier rather than to `downstream`. (Recommended — keeps a single continuation per resolved group at every level.)

---

## File Structure

- `src-tauri/app/migrations/008_nested_groups.sql` — **Create.** Adds `parent_group_id`, `parent_lane` to `fanout_groups`.
- `src-tauri/app/src/lib.rs` — **Modify.** Register migration 008 in both lists; bump `user_version` test 7→8.
- `src-tauri/runtime/src/fanout_group.rs` — **Modify.** Add `parent_group_id` + `parent_lane` fields; add `is_child()` helper + `parent_lane_verdict(Continuation) -> Verdict` mapping.
- `src-tauri/runtime/src/fanout_store.rs` — **Modify.** `create` persists parent link; `load` reads it; new test fixtures.
- `src-tauri/runtime/src/pool.rs` — **Modify.** Fork-expansion accepts an optional parent link; `resolve_barrier`, on a completing CHILD group, settles the parent lane instead of (only) creating a downstream task; `lane_reaches` extended hierarchically.
- `src-tauri/pipeline/src/validate.rs` — **Modify.** Replace `check_lane_linear` with hierarchical `check_lane_reachable`; add `NestingTooDeep`; relax the two LaneNotLinear tests; add nested-lane acceptance + depth-bound tests.
- `src-tauri/pipeline/src/draft.rs` — **Modify.** Relax the best-effort "no gate inside a lane" rule (gates + nested forks now allowed); keep unknown-node checks.

---

## Task 1: Migration 008 — parent link on fanout_groups

**Files:**
- Create: `src-tauri/app/migrations/008_nested_groups.sql`
- Modify: `src-tauri/app/src/lib.rs` (two migration lists + user_version test)

- [ ] **Step 1: Write the migration file**

Create `src-tauri/app/migrations/008_nested_groups.sql`:

```sql
-- 008_nested_groups.sql — P1 (nested groups / non-linear lanes). Makes the
-- FanOutGroup aggregate RECURSIVE: a fork nested inside a lane spawns a CHILD
-- group whose completion settles its parent lane's verdict via the SAME
-- completes-once guard. Append-only; migrations 001–007 are never edited.
--
-- A root group (top-level fork) has both columns NULL. A child group carries the
-- enclosing group id + the enclosing lane name, so when the child barrier fires
-- the pool knows which parent lane to settle.
ALTER TABLE fanout_groups ADD COLUMN parent_group_id TEXT;  -- enclosing group, or NULL at the root
ALTER TABLE fanout_groups ADD COLUMN parent_lane     TEXT;  -- enclosing lane (entry team id), or NULL at the root

CREATE INDEX IF NOT EXISTS idx_fanout_groups_parent ON fanout_groups(parent_group_id);
```

- [ ] **Step 2: Register migration 008 in `run_migrations` MIGRATIONS list**

In `src-tauri/app/src/lib.rs`, after the `(7, include_str!("../migrations/007_invocation_audit.sql")),` line add:

```rust
        (8, include_str!("../migrations/008_nested_groups.sql")),
```

- [ ] **Step 3: Register migration 008 in the tauri-plugin-sql `migrations` vec**

In `src-tauri/app/src/lib.rs`, after the 007 entry in the `let migrations = vec![ ... ]` block add (match the existing `Migration { version, description, sql, kind }` shape used by entries 1–7):

```rust
        tauri_plugin_sql::Migration {
            version: 8,
            description: "nested groups — fanout_groups.parent_group_id + parent_lane",
            sql: include_str!("../migrations/008_nested_groups.sql"),
            kind: tauri_plugin_sql::MigrationKind::Up,
        },
```

(Inspect the exact field names/struct path of the existing 007 entry and mirror it byte-for-byte — the snippet above is the expected shape.)

- [ ] **Step 4: Bump the user_version assertion 7 → 8**

In the `migration_tests` module, change:

```rust
        assert_eq!(version, 7, "all seven migrations recorded");
```

to:

```rust
        assert_eq!(version, 8, "all eight migrations recorded");
```

- [ ] **Step 5: Run the migration test**

Run: `cd src-tauri && cargo test -p app migration_tests::fresh_file_is_created_migrated_and_idempotent -- --nocapture`
Expected: PASS (8 migrations recorded, idempotent second run).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/app/migrations/008_nested_groups.sql src-tauri/app/src/lib.rs
git commit -m "feat(migrations): 008 fanout_groups parent link (P1 nested groups)"
```

---

## Task 2: FanOutGroup gains the parent link + parent-lane verdict mapping

**Files:**
- Modify: `src-tauri/runtime/src/fanout_group.rs`

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `fanout_group.rs`:

```rust
    #[test]
    fn a_root_group_has_no_parent_and_a_child_group_does() {
        let root = group();
        assert!(!root.is_child());
        let child = FanOutGroup {
            id: "G-2".into(),
            pipeline: "pipe".into(),
            join_target: "join-2".into(),
            downstream: "after-2".into(),
            expected_lanes: vec!["x".into(), "y".into()],
            completed: false,
            parent_group_id: Some("G-1".into()),
            parent_lane: Some("lane-a".into()),
        };
        assert!(child.is_child());
        assert_eq!(child.parent_group_id.as_deref(), Some("G-1"));
        assert_eq!(child.parent_lane.as_deref(), Some("lane-a"));
    }

    #[test]
    fn parent_lane_verdict_maps_downstream_to_approve_and_needs_human_to_reject() {
        assert_eq!(
            FanOutGroup::parent_lane_verdict(&Continuation::Downstream("x".into())),
            Verdict::Approve
        );
        assert_eq!(
            FanOutGroup::parent_lane_verdict(&Continuation::NeedsHuman),
            Verdict::Reject
        );
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd src-tauri && cargo test -p runtime fanout_group:: 2>&1 | tail -20`
Expected: FAIL — missing fields `parent_group_id`/`parent_lane`, missing `is_child`, missing `parent_lane_verdict`.

- [ ] **Step 3: Add the fields + methods**

In `fanout_group.rs`, extend the struct:

```rust
pub struct FanOutGroup {
    pub id: String,
    pub pipeline: String,
    pub join_target: String,
    pub downstream: String,
    pub expected_lanes: Vec<String>,
    pub completed: bool,
    /// The enclosing group when this group is a nested fork inside a lane (P1).
    /// `None` at the root (a top-level fork). When `Some`, this group's
    /// completion settles the parent lane's verdict via the parent's barrier —
    /// the same completes-once guard, one level up (DD-P1-2).
    #[serde(default)]
    pub parent_group_id: Option<String>,
    /// The enclosing lane (entry team id) this group reports its resolution to.
    /// `Some` iff `parent_group_id` is `Some`.
    #[serde(default)]
    pub parent_lane: Option<String>,
}
```

Add to the `impl FanOutGroup` block:

```rust
    /// True when this group is a nested child (a fork inside a parent lane). A
    /// child settles its parent lane on completion rather than spawning the
    /// continuation directly (DD-P1-2).
    pub fn is_child(&self) -> bool {
        self.parent_group_id.is_some()
    }

    /// Map a resolved child group's continuation to the verdict its parent lane
    /// records at the parent barrier: the child resolving to its downstream means
    /// the lane "approved" into the parent join; needs-human means the lane
    /// failed. This is how a nested group's exactly-once resolution feeds the
    /// parent level's exactly-once barrier (DD-P1-2).
    pub fn parent_lane_verdict(cont: &Continuation) -> Verdict {
        match cont {
            Continuation::Downstream(_) => Verdict::Approve,
            Continuation::NeedsHuman => Verdict::Reject,
        }
    }
```

- [ ] **Step 4: Fix the existing test fixtures (add the two new fields)**

Every `FanOutGroup { ... }` literal in this file's tests needs `parent_group_id: None, parent_lane: None,`. Update the `group()` and `group3()` helpers:

```rust
    fn group() -> FanOutGroup {
        FanOutGroup {
            id: "G-1".into(),
            pipeline: "pipe".into(),
            join_target: "join-1".into(),
            downstream: "after".into(),
            expected_lanes: vec!["lane-a".into(), "lane-b".into()],
            completed: false,
            parent_group_id: None,
            parent_lane: None,
        }
    }
```

and likewise `group3()` (add the two `None` fields).

- [ ] **Step 5: Run to verify pass**

Run: `cd src-tauri && cargo test -p runtime fanout_group:: 2>&1 | tail -20`
Expected: PASS (all fanout_group tests, including the two new ones).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/runtime/src/fanout_group.rs
git commit -m "feat(runtime): FanOutGroup recursive parent link + parent-lane verdict (P1)"
```

---

## Task 3: FanOutStore persists + loads the parent link

**Files:**
- Modify: `src-tauri/runtime/src/fanout_store.rs`

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `fanout_store.rs` (and add `008_nested_groups.sql` to that module's `fresh_pool`):

```rust
    #[tokio::test]
    async fn create_and_load_round_trips_the_parent_link() {
        let store = FanOutStore::new(fresh_pool().await);
        let child = FanOutGroup {
            id: "G-child".into(),
            pipeline: "pipe".into(),
            join_target: "join-2".into(),
            downstream: "after-2".into(),
            expected_lanes: vec!["x".into(), "y".into()],
            completed: false,
            parent_group_id: Some("G-1".into()),
            parent_lane: Some("lane-a".into()),
        };
        store.create(&child).await.unwrap();
        store.seed_lane("G-child", "x").await.unwrap();
        store.seed_lane("G-child", "y").await.unwrap();
        let back = store.load("G-child").await.unwrap();
        assert_eq!(back.parent_group_id.as_deref(), Some("G-1"));
        assert_eq!(back.parent_lane.as_deref(), Some("lane-a"));
        assert!(back.is_child());
    }

    #[tokio::test]
    async fn a_root_group_loads_with_no_parent() {
        let store = FanOutStore::new(fresh_pool().await);
        seeded(&store).await;
        let back = store.load("G-1").await.unwrap();
        assert_eq!(back.parent_group_id, None);
        assert_eq!(back.parent_lane, None);
        assert!(!back.is_child());
    }
```

In this module's `fresh_pool()`, add after the 006 line:

```rust
        sqlx::query(include_str!("../../app/migrations/008_nested_groups.sql")).execute(&pool).await.unwrap();
```

Also update the local `group()` and `group3()` helpers in this module to include `parent_group_id: None, parent_lane: None,`.

- [ ] **Step 2: Run to verify it fails**

Run: `cd src-tauri && cargo test -p runtime fanout_store:: 2>&1 | tail -25`
Expected: FAIL — `create` doesn't bind parent columns / `load` doesn't select them / struct literals miss fields.

- [ ] **Step 3: Update `create` to persist the parent link**

Replace the `create` body's query:

```rust
    pub async fn create(&self, g: &FanOutGroup) -> Result<(), FanOutStoreError> {
        sqlx::query(
            "INSERT INTO fanout_groups
               (id, pipeline, join_target, downstream, completed, created_at, parent_group_id, parent_lane)
             VALUES (?,?,?,?,0,0,?,?)",
        )
        .bind(&g.id)
        .bind(&g.pipeline)
        .bind(&g.join_target)
        .bind(&g.downstream)
        .bind(&g.parent_group_id)
        .bind(&g.parent_lane)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
```

- [ ] **Step 4: Update `load` to read the parent link**

Replace the `load` body:

```rust
    pub async fn load(&self, group_id: &str) -> Result<FanOutGroup, FanOutStoreError> {
        let row = sqlx::query_as::<_, (String, String, String, i64, Option<String>, Option<String>)>(
            "SELECT pipeline, join_target, downstream, completed, parent_group_id, parent_lane
             FROM fanout_groups WHERE id = ?",
        )
        .bind(group_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| FanOutStoreError::NotFound(group_id.to_string()))?;
        let lanes: Vec<(String,)> =
            sqlx::query_as("SELECT lane FROM fanout_lanes WHERE group_id = ? ORDER BY lane")
                .bind(group_id)
                .fetch_all(&self.pool)
                .await?;
        Ok(FanOutGroup {
            id: group_id.to_string(),
            pipeline: row.0,
            join_target: row.1,
            downstream: row.2,
            expected_lanes: lanes.into_iter().map(|(l,)| l).collect(),
            completed: row.3 != 0,
            parent_group_id: row.4,
            parent_lane: row.5,
        })
    }
```

- [ ] **Step 5: Run to verify pass**

Run: `cd src-tauri && cargo test -p runtime fanout_store:: 2>&1 | tail -25`
Expected: PASS — all existing barrier/quorum/early-cancel tests stay green + the two new round-trip tests pass.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/runtime/src/fanout_store.rs
git commit -m "feat(runtime): FanOutStore persists/loads the parent link (P1)"
```

---

## Task 4: Pool — fork expansion carries an optional parent link

**Files:**
- Modify: `src-tauri/runtime/src/pool.rs`

This task refactors fork expansion into a helper that takes an optional parent link, so both top-level and nested forks share one path. No behavior change yet for the top-level path (parent = None).

- [ ] **Step 1: Write the failing test (nested expansion records a child group)**

Add to the `tests` module in `pool.rs`. First add `008` to this module's `fresh_pool`:

```rust
        sqlx::query(include_str!("../../app/migrations/008_nested_groups.sql")).execute(&pool).await.unwrap();
```

Add a nested pipeline fixture + test:

```rust
    fn pipeline_v2_nested() -> Pipeline {
        // entry -> fork-1 {lane-a, lane-b}; lane-a is itself a fork:
        //   lane-a -> fork-2 {a1, a2} -> join-2 -> mid-a -> join-1
        //   lane-b -> join-1
        // join-1 -> after
        Pipeline {
            id: "p".into(), name: "P".into(), description: String::new(), schema_version: 2,
            defaults: None,
            teams: vec![
                team("entry", Some("fork-1"), None),
                team("lane-a", Some("fork-2"), None),
                team("a1", Some("join-2"), None),
                team("a2", Some("join-2"), None),
                team("mid-a", Some("join-1"), None),
                team("lane-b", Some("join-1"), None),
                team("after", Some("done"), None),
            ],
            gates: vec![],
            escalations: vec![pipeline::model::Escalation { id: "needs-human".into(), triggers: vec![] }],
            forks: vec![
                Fork { id: "fork-1".into(), lanes: vec!["lane-a".into(), "lane-b".into()] },
                Fork { id: "fork-2".into(), lanes: vec!["a1".into(), "a2".into()] },
            ],
            joins: vec![
                Join { id: "join-1".into(), waits_for: vec!["lane-a".into(), "lane-b".into()], downstream: "after".into(), cancel_on_reject: false, quorum: None },
                Join { id: "join-2".into(), waits_for: vec!["a1".into(), "a2".into()], downstream: "mid-a".into(), cancel_on_reject: false, quorum: None },
            ],
        }
    }

    #[tokio::test]
    async fn nested_fork_inside_a_lane_spawns_a_child_group_linked_to_parent() {
        let pool = fresh_pool().await;
        let p = pipeline_v2_nested();
        let ctx = ctx_with(pool.clone(), p.clone(), Arc::new(FakeRunner::always(approve_output())), temp_root());
        let t = Task::injected("proj".into(), "p".into(), "entry".into(), "topic".into(), Some("/repo".into()), 100);
        ctx.tasks.insert(&t).await.unwrap();

        process_one_claim(&ctx, &p.teams[0]).await.unwrap(); // entry -> fork-1 -> lane-a, lane-b
        process_one_claim(&ctx, &p.teams[1]).await.unwrap(); // lane-a approve -> fork-2 (nested) -> a1, a2

        // Two queued lane tasks for the nested fork (a1, a2), each in a CHILD group.
        let queued = ctx.tasks.list_by_state(TaskState::Queued).await.unwrap();
        let child_lane_tasks: Vec<_> = queued.iter().filter(|q| q.current_stage == "a1" || q.current_stage == "a2").collect();
        assert_eq!(child_lane_tasks.len(), 2, "nested fork expands into 2 child lane tasks");
        let child_group = child_lane_tasks[0].group_id.clone().unwrap();
        assert!(child_lane_tasks.iter().all(|q| q.group_id.as_deref() == Some(child_group.as_str())));
        // The child group is linked to the parent group + lane-a.
        let cg = ctx.fanout.load(&child_group).await.unwrap();
        assert!(cg.is_child());
        assert_eq!(cg.parent_lane.as_deref(), Some("lane-a"));
        assert!(cg.parent_group_id.is_some());
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd src-tauri && cargo test -p runtime nested_fork_inside_a_lane_spawns_a_child_group_linked_to_parent 2>&1 | tail -30`
Expected: FAIL — currently a lane-a approve toward `fork-2` either errors (no join pairs) or doesn't create a child group with a parent link. (Also `lane_reaches` won't pair fork-1 with join-1 through the nested structure yet — Task 5 fixes pairing; for THIS test the fork-1 expansion already happened at entry, so the failure is specifically the nested fork-2 not being linked.)

- [ ] **Step 3: Extract a parent-aware fork-expansion helper**

In `pool.rs`, refactor the fork-expansion block inside `settle_and_route`. Replace the existing `if let Some(fork) = ...` block with a call to a new helper, and add the helper. The helper takes the optional parent link from the settling task:

```rust
    // FORK EXPANSION: an approve whose target is a Fork node fans the task out
    // into one sibling task per lane, then terminates the original (forked).
    // P1: if the SETTLING task is itself a lane of a parent group, the new group
    // is a CHILD — it carries the parent group id + the parent lane so its
    // completion settles the parent lane's verdict (DD-P1-2).
    if let Some(fork) = ctx.pipeline.forks.iter().find(|f| f.id == routed.next_stage) {
        let parent = task
            .group_id
            .clone()
            .zip(task.lane.clone());
        return expand_fork(ctx, task, fork, parent).await;
    }
```

Add the helper (after `settle_and_route`):

```rust
/// Expand a fork into one lane sibling per lane and create the (possibly child)
/// FanOutGroup. `parent` is `Some((parent_group_id, parent_lane))` when the fork
/// is nested inside a lane (P1) — the new group is then a child whose completion
/// settles that parent lane. Top-level forks pass `None`.
async fn expand_fork(
    ctx: &PoolContext,
    task: &mut Task,
    fork: &pipeline::model::Fork,
    parent: Option<(String, String)>,
) -> Result<Option<(String, TaskState)>, PoolError> {
    let join = ctx
        .pipeline
        .joins
        .iter()
        .find(|j| fork.lanes.iter().all(|lane| lane_reaches(&ctx.pipeline, lane, &j.id)))
        .ok_or(PoolError::Route(RouteError::NoRoute))?;
    let group_id = format!("G-{}", uuid::Uuid::new_v4());
    let (parent_group_id, parent_lane) = match parent {
        Some((g, l)) => (Some(g), Some(l)),
        None => (None, None),
    };
    let group = FanOutGroup {
        id: group_id.clone(),
        pipeline: task.pipeline.clone(),
        join_target: join.id.clone(),
        downstream: join.downstream.clone(),
        expected_lanes: fork.lanes.clone(),
        completed: false,
        parent_group_id,
        parent_lane,
    };
    ctx.fanout.create(&group).await?;
    let now = now_unix();
    for lane in &fork.lanes {
        ctx.fanout.seed_lane(&group_id, lane).await?;
        let sib = Task::forked(task, lane, &group_id, &join.id, now);
        ctx.tasks.insert(&sib).await?;
    }
    task.state = TaskState::Done;
    task.current_stage = fork.id.clone();
    task.updated_at = now;
    ctx.tasks.update(task).await?;
    Ok(Some((fork.id.clone(), TaskState::Done)))
}
```

Note: `lane_reaches` must reach `join-2` for the nested fork's lanes (a1, a2 directly approve into join-2 — already linear, so existing `lane_reaches` works for fork-2). The parent fork-1's pairing with join-1 needs the hierarchical walk (Task 5).

- [ ] **Step 4: Run to verify pass**

Run: `cd src-tauri && cargo test -p runtime nested_fork_inside_a_lane_spawns_a_child_group_linked_to_parent 2>&1 | tail -30`
Expected: PASS. (Top-level fork tests still green — parent is None there, identical behavior.)

- [ ] **Step 5: Run all pool fork/join tests to confirm no regression**

Run: `cd src-tauri && cargo test -p runtime pool:: 2>&1 | tail -30`
Expected: PASS (all existing fork/join/P2/P3 tests green).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/runtime/src/pool.rs
git commit -m "feat(runtime): parent-aware fork expansion (nested child groups) (P1)"
```

---

## Task 5: Pool — hierarchical `lane_reaches` (fork/join pairing through nested structure)

**Files:**
- Modify: `src-tauri/runtime/src/pool.rs`

`lane_reaches` is used to pair a fork with its join. For the nested pipeline, fork-1's lane `lane-a` reaches join-1 only by traversing: `lane-a -> fork-2 -> (join-2) -> mid-a -> join-1`. The flat `lane_reaches` stops at `fork-2` (no `on_approve` on a fork). Make it hierarchical: a fork hop jumps to its paired join's downstream and continues.

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn lane_reaches_traverses_a_nested_fork_to_the_outer_join() {
        let p = pipeline_v2_nested();
        // lane-a goes through fork-2 / join-2 / mid-a before hitting join-1
        assert!(lane_reaches(&p, "lane-a", "join-1"));
        // lane-b is a direct linear lane to join-1
        assert!(lane_reaches(&p, "lane-b", "join-1"));
        // the nested fork's own lanes reach join-2 directly
        assert!(lane_reaches(&p, "a1", "join-2"));
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd src-tauri && cargo test -p runtime lane_reaches_traverses_a_nested_fork_to_the_outer_join 2>&1 | tail -20`
Expected: FAIL — `lane_reaches(&p, "lane-a", "join-1")` returns false (stops at fork-2).

- [ ] **Step 3: Make `lane_reaches` hierarchical**

Replace `lane_reaches`:

```rust
/// Walk a lane from its entry team toward `join_id`, following on_approve.
/// Hierarchical (P1): a team hop follows on_approve; a GATE hop follows the
/// gate's downstream; a FORK hop jumps to that nested fork's paired join's
/// downstream (the nested group resolves to a single continuation there).
/// Reaching `join_id` is success. Bounded by total node count to terminate.
fn lane_reaches(p: &Pipeline, entry: &str, join_id: &str) -> bool {
    let bound = p.teams.len() + p.gates.len() + p.forks.len() + p.joins.len() + 1;
    let mut current = entry.to_string();
    for _ in 0..=bound {
        if current == join_id {
            return true;
        }
        if let Some(t) = p.teams.iter().find(|t| t.id == current) {
            match t.outputs.on_approve.as_deref() {
                Some(next) => { current = next.to_string(); continue; }
                None => return false,
            }
        }
        if let Some(g) = p.gates.iter().find(|g| g.id == current) {
            current = g.downstream.clone();
            continue;
        }
        if let Some(f) = p.forks.iter().find(|f| f.id == current) {
            // Pair this nested fork with its join: the join whose lanes are all
            // reachable from the fork's lanes. Continue from that join's downstream.
            match p.joins.iter().find(|j| f.lanes.iter().all(|lane| lane_reaches(p, lane, &j.id))) {
                Some(j) => { current = j.downstream.clone(); continue; }
                None => return false,
            }
        }
        return false;
    }
    false
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cd src-tauri && cargo test -p runtime lane_reaches_traverses_a_nested_fork_to_the_outer_join 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 5: Run all runtime tests**

Run: `cd src-tauri && cargo test -p runtime 2>&1 | tail -15`
Expected: PASS (all existing tests; flat lanes still pair correctly since the team/gate branches cover them).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/runtime/src/pool.rs
git commit -m "feat(runtime): hierarchical lane_reaches pairs nested fork/join (P1)"
```

---

## Task 6: Pool — a completing CHILD group settles its parent lane

**Files:**
- Modify: `src-tauri/runtime/src/pool.rs`

This is the crux. When `resolve_barrier` completes a group, check whether the completed group `is_child()`. If so, instead of (only) creating a downstream/needs-human task, settle the **parent lane** at the **parent group's barrier** using `parent_lane_verdict(continuation)`. The parent barrier obeys the parent join's policy (full / quorum / early-cancel) and may itself complete — recurse up.

- [ ] **Step 1: Write the failing tests (the two crux behaviors + exactly-once)**

```rust
    #[tokio::test]
    async fn nested_child_all_approve_settles_parent_lane_and_parent_completes() {
        // Full nested run: entry -> fork-1 -> (lane-a -> fork-2 -> a1,a2 -> join-2 -> mid-a -> join-1), lane-b -> join-1.
        // Everything approves; exactly one continuation at `after`.
        let pool = fresh_pool().await;
        let p = pipeline_v2_nested();
        let ctx = ctx_with(pool.clone(), p.clone(), Arc::new(FakeRunner::always(approve_output())), temp_root());
        let t = Task::injected("proj".into(), "p".into(), "entry".into(), "topic".into(), Some("/repo".into()), 100);
        ctx.tasks.insert(&t).await.unwrap();

        process_one_claim(&ctx, &p.teams[0]).await.unwrap(); // entry -> fork-1
        process_one_claim(&ctx, &p.teams[5]).await.unwrap(); // lane-b approve -> join-1 (parks; parent group not complete)
        process_one_claim(&ctx, &p.teams[1]).await.unwrap(); // lane-a approve -> fork-2 (child group)
        process_one_claim(&ctx, &p.teams[2]).await.unwrap(); // a1 approve -> join-2 (child parks)
        process_one_claim(&ctx, &p.teams[3]).await.unwrap(); // a2 approve -> join-2 (child COMPLETES -> settles parent lane-a)
        // child completing settled parent lane-a=approve; parent had lane-b=approve -> parent completes -> mid-a? No:
        // parent join-1 downstream is `after`. The child's join-2 downstream (mid-a) is a lane-internal continuation that
        // is consumed as the parent-lane verdict (DD-P1-8). So after the child completes, the parent lane-a is settled
        // and the PARENT barrier fires -> one continuation at `after`.
        let queued = ctx.tasks.list_by_state(TaskState::Queued).await.unwrap();
        let at_after: Vec<_> = queued.iter().filter(|q| q.current_stage == "after").collect();
        assert_eq!(at_after.len(), 1, "exactly one continuation past the outer join");
        assert_eq!(at_after[0].group_id, None, "continuation is back in linear flow");
    }

    #[tokio::test]
    async fn nested_child_reject_settles_parent_lane_reject_and_parent_needs_human() {
        // a1 rejects inside the nested fork -> child group resolves needs-human ->
        // parent lane-a settles as Reject -> parent (with lane-b approve) -> needs-human.
        let pool = fresh_pool().await;
        let p = pipeline_v2_nested();
        let reject = RunnerOutput { verdict: Verdict::Reject, artifact_path: None, final_text: "VERDICT: reject".into(), usage: RunnerUsage::default() };
        // reject only the `a1` nested lane
        struct A1Reject { reject: RunnerOutput }
        #[async_trait::async_trait]
        impl Runner for A1Reject {
            async fn invoke(&self, req: &InvocationRequest) -> Result<RunnerOutput, RunnerError> {
                if req.team_id == "a1" { Ok(self.reject.clone()) }
                else { Ok(RunnerOutput { verdict: Verdict::Approve, artifact_path: Some("a.md".into()), final_text: "VERDICT: approve".into(), usage: RunnerUsage::default() }) }
            }
        }
        let ctx = ctx_with(pool.clone(), p.clone(), Arc::new(A1Reject { reject }), temp_root());
        let t = Task::injected("proj".into(), "p".into(), "entry".into(), "topic".into(), Some("/repo".into()), 100);
        ctx.tasks.insert(&t).await.unwrap();

        process_one_claim(&ctx, &p.teams[0]).await.unwrap(); // fork-1
        process_one_claim(&ctx, &p.teams[5]).await.unwrap(); // lane-b approve (parks)
        process_one_claim(&ctx, &p.teams[1]).await.unwrap(); // lane-a -> fork-2
        process_one_claim(&ctx, &p.teams[2]).await.unwrap(); // a1 reject -> child barrier reject
        process_one_claim(&ctx, &p.teams[3]).await.unwrap(); // a2 approve -> child completes needs-human -> parent lane-a reject -> parent needs-human

        let nh = ctx.tasks.list_by_state(TaskState::NeedsHuman).await.unwrap();
        assert_eq!(nh.len(), 1, "the nested reject escalates the outer join exactly once");
        assert_eq!(nh[0].current_stage, "needs-human");
    }

    #[tokio::test]
    async fn nested_completion_is_exactly_once_against_a_concurrent_parent_straggler() {
        // lane-b and the nested child both try to complete the PARENT barrier concurrently.
        // Exactly one parent continuation results.
        use std::sync::Arc as StdArc;
        let pool = fresh_pool().await;
        let p = pipeline_v2_nested();
        let ctx = StdArc::new(ctx_with(pool.clone(), p.clone(), StdArc::new(FakeRunner::always(approve_output())), temp_root()));
        let t = Task::injected("proj".into(), "p".into(), "entry".into(), "topic".into(), Some("/repo".into()), 100);
        ctx.tasks.insert(&t).await.unwrap();
        process_one_claim(&ctx, &p.teams[0]).await.unwrap(); // fork-1
        process_one_claim(&ctx, &p.teams[1]).await.unwrap(); // lane-a -> fork-2
        process_one_claim(&ctx, &p.teams[2]).await.unwrap(); // a1 parks
        // Now run lane-b (settles parent lane-b) and a2 (completes child -> settles parent lane-a) concurrently.
        let c1 = ctx.clone(); let p1 = p.clone();
        let c2 = ctx.clone(); let p2 = p.clone();
        let h1 = tokio::spawn(async move { process_one_claim(&c1, &p1.teams[5]).await.unwrap() });
        let h2 = tokio::spawn(async move { process_one_claim(&c2, &p2.teams[3]).await.unwrap() });
        let _ = (h1.await.unwrap(), h2.await.unwrap());
        let queued = ctx.tasks.list_by_state(TaskState::Queued).await.unwrap();
        let at_after = queued.iter().filter(|q| q.current_stage == "after").count();
        assert_eq!(at_after, 1, "exactly one parent continuation despite concurrent settles");
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cd src-tauri && cargo test -p runtime nested_child 2>&1 | tail -40`
Expected: FAIL — the child group completes but its continuation is created as a downstream task (`mid-a`) instead of settling the parent lane; the parent never completes via the child.

- [ ] **Step 3: Implement child-completion → parent-lane settlement**

In `resolve_barrier`, after computing `outcome` and parking the lane task, replace the `if let BarrierOutcome::Completed(cont) = outcome { ... }` block so that when the completed group is a child, it recurses into the parent barrier instead of spawning the continuation. Extract the "create continuation OR settle parent" decision:

```rust
    // Park this lane task as terminal for the lane.
    task.state = TaskState::Done;
    task.current_stage = task.join_target.clone().unwrap_or_else(|| task.current_stage.clone());
    task.updated_at = now;
    ctx.tasks.update(task).await?;

    if let BarrierOutcome::Completed(cont) = outcome {
        // Stop outstanding lanes when an early resolution wins (P2/P3).
        if (early_cancel && is_failure) || quorum.is_some() {
            ctx.tasks.cancel_outstanding_lanes(&group_id, now_unix()).await?;
        }
        return finish_group(ctx, &group_id, task, cont).await;
    }
    Ok(Some((task.current_stage.clone(), TaskState::Done)))
}

/// A group has completed. If it is a ROOT group, spawn the single continuation
/// task (downstream on all-approve, needs-human otherwise) — the original flat
/// behavior. If it is a CHILD group (a nested fork inside a parent lane), settle
/// the PARENT lane's verdict at the parent group's barrier instead — the same
/// completes-once guard one level up (DD-P1-2). This may complete the parent,
/// which recurses again — exactly-once holds independently at every level.
async fn finish_group(
    ctx: &PoolContext,
    group_id: &str,
    task: &mut Task,
    cont: Continuation,
) -> Result<Option<(String, TaskState)>, PoolError> {
    let group = ctx.fanout.load(group_id).await?;
    if let (Some(parent_group), Some(parent_lane)) = (group.parent_group_id.clone(), group.parent_lane.clone()) {
        // Child group: feed the parent lane the derived verdict through the
        // parent's barrier (respecting the parent join's policy).
        let parent_verdict = FanOutGroup::parent_lane_verdict(&cont);
        return settle_parent_lane(ctx, task, &parent_group, &parent_lane, parent_verdict).await;
    }
    // Root group: create the single continuation task.
    spawn_continuation(ctx, task, cont).await
}

/// Create the single continuation task past a ROOT join (downstream queued, or
/// needs-human escalation). Carries the parent's lineage; clears lane fields so
/// the continuation is back in ordinary linear flow.
async fn spawn_continuation(
    ctx: &PoolContext,
    task: &Task,
    cont: Continuation,
) -> Result<Option<(String, TaskState)>, PoolError> {
    let now = now_unix();
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
    Ok(Some((stage, state)))
}

/// Settle a parent lane's verdict at the parent group's barrier when a nested
/// CHILD group resolves (DD-P1-2). Reuses the same barrier methods the lane
/// settlement path uses — the parent join's policy (full / quorum / early-cancel)
/// governs, and the parent's `completed` guard arbitrates exactly-once. If the
/// parent itself completes, recurse via `finish_group` (which handles a
/// grandparent, etc.).
///
/// VET F1 — INVARIANT: each call here is its OWN single-row guard
/// (`UPDATE fanout_groups SET completed=1 WHERE id=? AND completed=0`). The
/// child-complete write (in the caller) and this parent-settle write are
/// DELIBERATELY two separate writes — exactly-once holds per group row,
/// independently at each level. Do NOT merge them into one transaction spanning
/// two `fanout_groups` rows: that re-introduces the cross-root coupling the
/// two-aggregate model forbids (and risks a two-row deadlock). Reference by id,
/// settle eventually — the correct cross-aggregate shape.
async fn settle_parent_lane(
    ctx: &PoolContext,
    task: &mut Task,
    parent_group: &str,
    parent_lane: &str,
    verdict: agent_bus_core::Verdict,
) -> Result<Option<(String, TaskState)>, PoolError> {
    let parent = ctx.fanout.load(parent_group).await?;
    let join = ctx.pipeline.joins.iter().find(|j| j.id == parent.join_target);
    let quorum = join.and_then(|j| j.quorum);
    let early_cancel = quorum.is_none() && join.map(|j| j.cancel_on_reject).unwrap_or(false);
    let is_failure = verdict == agent_bus_core::Verdict::Reject;

    let outcome = if let Some(q) = quorum {
        ctx.fanout.record_and_try_quorum(parent_group, parent_lane, verdict, q).await?
    } else if early_cancel && is_failure {
        ctx.fanout.record_failure_and_early_cancel(parent_group, parent_lane, verdict).await?
    } else {
        ctx.fanout.record_and_try_complete(parent_group, parent_lane, verdict).await?
    };

    if let BarrierOutcome::Completed(cont) = outcome {
        if (early_cancel && is_failure) || quorum.is_some() {
            ctx.tasks.cancel_outstanding_lanes(parent_group, now_unix()).await?;
        }
        return finish_group(ctx, parent_group, task, cont).await;
    }
    Ok(Some((parent_lane.to_string(), TaskState::Done)))
}
```

Now delete the now-inlined continuation block that previously lived at the end of `resolve_barrier` (the `let (stage, state) = match cont ...` and the `Task::injected` block) — it has moved into `spawn_continuation`. Ensure `resolve_barrier` ends with the `finish_group` call shown above.

Add the needed import at the top of `pool.rs` if not present: `use crate::fanout_group::FanOutGroup;` is already imported; ensure `Continuation` is too (it is).

- [ ] **Step 4: Run the crux tests**

Run: `cd src-tauri && cargo test -p runtime nested_child 2>&1 | tail -40`
Expected: PASS — all three nested tests (all-approve, reject, exactly-once-concurrent).

- [ ] **Step 5: Run all runtime tests (no regression to flat / P2 / P3)**

Run: `cd src-tauri && cargo test -p runtime 2>&1 | tail -20`
Expected: PASS — every existing fork/join/P2/P3/barrier test green; flat root groups still spawn one continuation.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/runtime/src/pool.rs
git commit -m "feat(runtime): child group completion settles parent lane via parent barrier (P1)"
```

---

## Task 7: Validation — relax LaneNotLinear to hierarchical reachability + depth bound

**Files:**
- Modify: `src-tauri/pipeline/src/validate.rs`

- [ ] **Step 1: Write the failing tests**

Replace the two rejection tests (`a_gate_inside_a_lane_is_rejected`, `a_nested_fork_inside_a_lane_is_rejected`) with acceptance tests, and add a depth-bound test:

```rust
    #[test]
    fn a_gate_inside_a_lane_is_now_accepted() {
        // P1: a lane may contain a gate. lane-a -> gate-x -> join-1.
        let mut p = valid_v2_pipeline();
        p.teams[1].outputs.on_approve = Some("gate-x".into());
        p.gates.push(GateNode { id: "gate-x".into(), label: "X".into(), downstream: "join-1".into() });
        assert_eq!(validate(&p), Ok(()));
    }

    #[test]
    fn a_nested_fork_inside_a_lane_is_now_accepted() {
        // P1: lane-a is itself a fork. lane-a -> fork-2 {n1,n2} -> join-2 -> join-1.
        let mut p = valid_v2_pipeline();
        p.teams[1].outputs.on_approve = Some("fork-2".into());
        p.teams.push(lane_team("n1", "join-2"));
        p.teams.push(lane_team("n2", "join-2"));
        p.forks.push(Fork { id: "fork-2".into(), lanes: vec!["n1".into(), "n2".into()] });
        p.joins.push(Join { id: "join-2".into(), waits_for: vec!["n1".into(), "n2".into()], downstream: "join-1".into(), cancel_on_reject: false, quorum: None });
        assert_eq!(validate(&p), Ok(()));
    }

    #[test]
    fn nesting_deeper_than_the_bound_is_rejected() {
        // 4 levels of fork nesting exceeds the max depth (3).
        let mut p = valid_v2_pipeline();
        // lane-a -> fork-2 -> fork-3 -> fork-4 (each a single nested fork inside the prior lane)
        p.teams[1].outputs.on_approve = Some("fork-2".into());
        // fork-2 lanes
        p.teams.push(lane_team("b1", "fork-3"));
        p.teams.push(lane_team("b2", "join-2"));
        p.forks.push(Fork { id: "fork-2".into(), lanes: vec!["b1".into(), "b2".into()] });
        p.joins.push(Join { id: "join-2".into(), waits_for: vec!["b1".into(), "b2".into()], downstream: "join-1".into(), cancel_on_reject: false, quorum: None });
        // fork-3 (depth 3) lanes
        p.teams.push(lane_team("c1", "fork-4"));
        p.teams.push(lane_team("c2", "join-3"));
        p.forks.push(Fork { id: "fork-3".into(), lanes: vec!["c1".into(), "c2".into()] });
        p.joins.push(Join { id: "join-3".into(), waits_for: vec!["c1".into(), "c2".into()], downstream: "join-2".into(), cancel_on_reject: false, quorum: None });
        // fork-4 (depth 4 — too deep) lanes
        p.teams.push(lane_team("d1", "join-4"));
        p.teams.push(lane_team("d2", "join-4"));
        p.forks.push(Fork { id: "fork-4".into(), lanes: vec!["d1".into(), "d2".into()] });
        p.joins.push(Join { id: "join-4".into(), waits_for: vec!["d1".into(), "d2".into()], downstream: "join-3".into(), cancel_on_reject: false, quorum: None });
        assert!(matches!(validate(&p), Err(PipelineValidationError::NestingTooDeep { .. })));
    }
```

Note: keep `a_multi_team_linear_lane_is_accepted` unchanged (flat lanes still valid).

- [ ] **Step 2: Run to verify they fail**

Run: `cd src-tauri && cargo test -p pipeline validate:: 2>&1 | tail -25`
Expected: FAIL — gate/nested-fork currently rejected; `NestingTooDeep` variant missing.

- [ ] **Step 3: Add the `NestingTooDeep` error variant**

In the `PipelineValidationError` enum add:

```rust
    #[error("fork nesting exceeds the maximum depth of {max} (fork '{fork}' is at depth {depth})")]
    NestingTooDeep { fork: String, depth: u32, max: u32 },
```

- [ ] **Step 4: Replace `check_lane_linear` with hierarchical `check_lane_reachable`**

Replace the `check_lane_linear` fn with:

```rust
/// Max fork nesting depth (DD-P1-5). A top-level fork is depth 1; a fork reached
/// from inside another fork's lane is depth 2; etc.
const MAX_NESTING_DEPTH: u32 = 3;

/// Walk a fork lane from its entry forward toward the join. P1: a lane may
/// contain gates and nested forks (no longer strictly linear). A team hop
/// follows on_approve; a gate hop follows the gate's downstream; a fork hop
/// recurses — each nested lane must reach the nested fork's paired join, whose
/// downstream continues the walk. Reaching `join_id` is success. `depth` tracks
/// fork nesting for the bound. Bounded by node count to terminate.
fn check_lane_reachable(
    p: &Pipeline,
    kinds: &HashMap<&str, NodeKind>,
    entry: &str,
    join_id: &str,
    depth: u32,
) -> Result<(), PipelineValidationError> {
    let bound = p.teams.len() + p.gates.len() + p.forks.len() + p.joins.len() + 1;
    let mut current = entry.to_string();
    for _ in 0..=bound {
        if current == join_id {
            return Ok(());
        }
        match kinds.get(current.as_str()) {
            Some(NodeKind::Team) => {
                let team = p.teams.iter().find(|t| t.id == current).unwrap();
                match team.outputs.on_approve.as_deref() {
                    Some(next) => current = next.to_string(),
                    None => return Err(PipelineValidationError::LaneNotLinear {
                        entry: entry.to_string(), node: current.clone(), join: join_id.to_string() }),
                }
            }
            Some(NodeKind::Gate) => {
                let gate = p.gates.iter().find(|g| g.id == current).unwrap();
                current = gate.downstream.clone();
            }
            Some(NodeKind::Fork) => {
                if depth + 1 > MAX_NESTING_DEPTH {
                    return Err(PipelineValidationError::NestingTooDeep {
                        fork: current.clone(), depth: depth + 1, max: MAX_NESTING_DEPTH });
                }
                let fork = p.forks.iter().find(|f| f.id == current).unwrap();
                // Pair the nested fork with the join whose lanes are all reachable
                // from its lanes (one deeper level). Continue from that downstream.
                let nested_join = p.joins.iter().find(|j| {
                    fork.lanes.iter().all(|lane| check_lane_reachable(p, kinds, lane, &j.id, depth + 1).is_ok())
                });
                match nested_join {
                    Some(j) => current = j.downstream.clone(),
                    None => return Err(PipelineValidationError::ForkJoinMismatch { fork: current.clone() }),
                }
            }
            // An escalation, join (other than the target), or unknown node ends a
            // lane that never reaches its join.
            _ => return Err(PipelineValidationError::LaneNotLinear {
                entry: entry.to_string(), node: current.clone(), join: join_id.to_string() }),
        }
    }
    Err(PipelineValidationError::LaneNotLinear {
        entry: entry.to_string(), node: current, join: join_id.to_string() })
}
```

- [ ] **Step 5: Update the fork/join pairing loop to call `check_lane_reachable` at depth 1**

Replace the `for fork in &p.forks { let paired = ... }` block. To avoid double-counting nested forks as top-level, only treat a fork as top-level (depth 1) if it is NOT reachable from inside another fork's lane. Simpler + sufficient: pair EVERY fork with a join via `check_lane_reachable`, starting depth at 1 for forks reachable from a team's on_approve and letting the recursion enforce the bound. Use:

```rust
    for fork in &p.forks {
        let paired = p.joins.iter().find(|j| {
            fork.lanes.iter().all(|lane| check_lane_reachable(p, &kinds, lane, &j.id, 1).is_ok())
        });
        match paired {
            Some(_join) => {}
            None => {
                // Surface the most specific lane error (depth/linearity) instead
                // of a bare mismatch when possible.
                if let Some(j) = p.joins.first() {
                    for lane in &fork.lanes {
                        check_lane_reachable(p, &kinds, lane, &j.id, 1)?;
                    }
                }
                return Err(PipelineValidationError::ForkJoinMismatch { fork: fork.id.clone() });
            }
        }
    }
```

Note on the depth bound and the `nesting_deeper_than_the_bound_is_rejected` test: when validating the top-level `fork-1`, `check_lane_reachable("lane-a", "join-1", 1)` walks lane-a → fork-2 (depth 2) → fork-3 (depth 3) → fork-4 (depth 4 > 3) and returns `NestingTooDeep`. Because the top-level loop calls `check_lane_reachable(... , 1)?` in the mismatch arm, that error propagates. Confirm the test asserts the `NestingTooDeep` variant (it does).

- [ ] **Step 6: Run validate tests**

Run: `cd src-tauri && cargo test -p pipeline validate:: 2>&1 | tail -25`
Expected: PASS — gate-in-lane + nested-fork accepted; flat lanes accepted; depth-4 rejected; all pre-existing fork/join/quorum tests green.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/pipeline/src/validate.rs
git commit -m "feat(pipeline): hierarchical lane validation + nesting depth bound (P1)"
```

---

## Task 8: Best-effort validation — drop the no-gate-in-lane rule

**Files:**
- Modify: `src-tauri/pipeline/src/draft.rs`

- [ ] **Step 1: Update the failing test**

The test `best_effort_flags_a_gate_inside_a_fork_lane` now expects NO such issue. Rename + invert it:

```rust
    #[test]
    fn best_effort_allows_a_gate_inside_a_fork_lane() {
        // P1: gates (and nested forks) may sit inside a lane; best-effort no longer
        // flags it. (Hard validate confirms hierarchical reachability + depth.)
        let mut d = DraftPipeline::empty();
        // reuse the same draft this test built before; assert the gate-in-lane
        // message is absent.
        // (Keep the original draft construction from the deleted test body here.)
        let issues = best_effort_validate(&d);
        assert!(
            !issues.iter().any(|i| i.contains("no gates inside a lane")),
            "gate-in-lane is allowed under P1; issues were: {issues:?}"
        );
    }
```

When implementing, copy the exact draft construction from the original `best_effort_flags_a_gate_inside_a_fork_lane` test body (teams/forks/gates) into this renamed test so it exercises the same shape, then assert the message is absent.

- [ ] **Step 2: Run to verify it fails**

Run: `cd src-tauri && cargo test -p pipeline draft::tests::best_effort_allows_a_gate_inside_a_fork_lane 2>&1 | tail -20`
Expected: FAIL — the issue is still produced.

- [ ] **Step 3: Remove the no-gate-in-lane block from `best_effort_validate`**

Delete this block (lines ~255–273):

```rust
    // Parallel-flow v1 rule: no gate may sit inside a fork lane. ...
    let mut lane_teams: ... = ...;
    for f in &draft.forks { ... }
    let gate_ids: ... = ...;
    for t in &draft.teams {
        if lane_teams.contains(t.id.as_str()) { ... "no gates inside a lane" ... }
    }
```

Keep everything above it (unknown-node checks for teams/forks/joins/gates) intact.

- [ ] **Step 4: Run to verify pass**

Run: `cd src-tauri && cargo test -p pipeline draft:: 2>&1 | tail -25`
Expected: PASS — the renamed test passes; the other best-effort tests (unknown-node, no-prompt, etc.) stay green.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/pipeline/src/draft.rs
git commit -m "feat(pipeline): best-effort allows gates/nested forks in lanes (P1)"
```

---

## Task 9: Full verification

- [ ] **Step 1: cargo test workspace**

Run: `cd src-tauri && cargo test --workspace 2>&1 | tail -30`
Expected: all green.

- [ ] **Step 2: cargo check + clippy**

Run: `cd src-tauri && cargo check --workspace 2>&1 | tail -5 && cargo clippy --workspace 2>&1 | tail -15`
Expected: clean (no warnings escalated to errors; address any new clippy lints in the touched files).

- [ ] **Step 2b: VET F4 — confirm a single continuation site**

Run: `grep -n "Task::injected" src-tauri/runtime/src/pool.rs`
Expected: the lane-continuation construction appears in exactly ONE place (`spawn_continuation`); the old inline continuation block in `resolve_barrier` and the inline fork block in `settle_and_route` are gone (replaced by `spawn_continuation` / `expand_fork`). If a second continuation site exists, the extraction (Task 4/6) left a duplicate — fix before proceeding.

- [ ] **Step 3: frontend checks**

Run: `export PATH="/opt/homebrew/bin:$PATH" && cd /Users/tim/projects/agent-bus-app && bun vitest run 2>&1 | tail -15 && bun run build 2>&1 | tail -10`
Expected: vitest green, build succeeds. (No frontend code changed; these confirm no incidental breakage.)

- [ ] **Step 4: Commit any clippy fixes**

```bash
git add -A && git commit -m "chore(p1): clippy/test cleanup"
```

---

## Self-Review Notes

- **Spec coverage:** recursive `FanOutGroup` (Task 2/3), hierarchical lane-walk in runtime (Task 5) + validation (Task 7), child completion feeds parent barrier via the same guard (Task 6), gate-in-lane (Tasks 4/6 via existing gate route + Task 7/8 validation), depth bound (Task 7), migration 008 in both lists + version test (Task 1). Exactly-once preserved at every level: each group is its own `completed` conditional-UPDATE; child→parent settlement reuses that guard one level up (Task 6 concurrency test).
- **No cross-root transaction:** child completion and parent settlement are separate `record_and_try_complete` calls, each its own single-row guard — no shared transaction spans two group rows.
- **Flat path unchanged:** root groups (`parent = None`) take `spawn_continuation`, byte-identical to the prior inline block.
