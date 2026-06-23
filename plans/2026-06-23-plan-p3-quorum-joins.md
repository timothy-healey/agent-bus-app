# P3 — Quorum Joins (N-of-M) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an opt-in per-join **quorum** (`Join.quorum: Option<u32>`) so a fork/join proceeds to `downstream` as soon as **N of the M lanes approve**, resolving early to needs-human only when N becomes impossible.

**Architecture:** Extends P2's aggregate-owned barrier resolution. The `FanOutGroup` aggregate gains a single new decision method — `quorum_continuation(recorded, quorum)` — that returns `Some(Downstream)` once N lanes approve, `Some(NeedsHuman)` once N is provably unreachable (unsettled + approved-so-far < N), or `None` (keep waiting). The store routes a quorum join through a new `record_and_try_quorum` that drives this decision behind the SAME `UPDATE … SET completed=1 WHERE completed=0` single-writer guard, preserving exactly-once. Default (`None`) keeps the exact all-must-approve path untouched.

**Tech Stack:** Rust (cargo workspace under `src-tauri/`), `sqlx` + SQLite (in-memory pools for tests), `serde`; TypeScript IPC types verified by `bun vitest`.

---

## Decisions

- **DD1 — Field shape.** `Join.quorum: Option<u32>`, `#[serde(default, skip_serializing_if = "Option::is_none")]`. `None` = all-must-approve (current behavior); `Some(n)` = need `n` approvals. **Additive, no `SCHEMA_VERSION` bump** (stays 2), mirroring P2's `cancel_on_reject`.
- **DD2 — Where the decision lives.** On the `FanOutGroup` aggregate (the consistency boundary for the barrier), as `quorum_continuation`. The store asks the aggregate; it never hardcodes the rule (same discipline as P2's `early_cancel_continuation`, vet F1).
- **DD3 — Early-resolve on success.** Complete to `Downstream` the instant the N-th lane approves; do NOT wait for the remaining lanes. Outstanding lanes no-op via the `WHERE completed=0` guard and their unstarted tasks are parked via `cancel_outstanding_lanes` (the P2 mechanism).
- **DD4 — Quorum-impossible ⇒ needs-human.** If `(unsettled_lanes + approvals_so_far) < quorum`, N can never be reached → resolve to `NeedsHuman` immediately (a symmetrical early-resolve). Dissenting lanes (reject / revise-cap) simply count as non-approvals; the only thing that matters is whether enough approvals remain *possible*.
- **DD5 — Outstanding/dissenting-lane handling.** Once resolved (either direction), outstanding lanes are parked exactly like P2's early-cancel (`cancel_outstanding_lanes` parks `queued`/`revising` lane tasks at their `join_target`; a `running` lane finishes and its later settle no-ops on the completed guard). Exactly-once preserved.
- **DD6 — Validation bounds.** If `quorum` is `Some(n)`, require `1 <= n <= waits_for.len()` (number of lanes the join waits for). `quorum == lanes` is allowed and is equivalent to all-must-approve. Default (`None`) is always valid.
- **DD7 — Interplay precedence (quorum + cancel_on_reject both set).** **Quorum governs success.** A reject only matters insofar as it can make quorum impossible. Concretely: when a quorum join receives a failure, we do NOT short-circuit to needs-human on the first reject (that is `cancel_on_reject`'s rule for the *default* all-must-approve barrier). Instead the reject is recorded as a non-approval and the quorum decision is re-evaluated — if approvals are still reachable, we keep waiting; if not, we resolve to needs-human. This is strictly more permissive than `cancel_on_reject` and is the coherent reading ("N must still approve"). **Therefore: if `quorum` is set, the quorum path is taken and `cancel_on_reject` is ignored** (documented on the field + in DOMAIN.md). Default joins (`quorum: None`) keep honoring `cancel_on_reject` exactly as P2 shipped.
- **DD8 — Frontend surface.** Add `quorum?: number` to the `Join` IPC type (round-trips through load/save), mirroring `cancel_on_reject?`. A wizard authoring control is deferred to the impeccable pass (note in backlog), same as P2.

---

## File Structure

- `src-tauri/pipeline/src/model.rs` — add `Join.quorum: Option<u32>`; update all `Join { … }` literals in tests.
- `src-tauri/pipeline/src/validate.rs` — add `QuorumOutOfRange` error + bounds check; tests.
- `src-tauri/runtime/src/fanout_group.rs` — add `quorum_continuation(&self, recorded, quorum) -> Option<Continuation>`; tests.
- `src-tauri/runtime/src/fanout_store.rs` — add `record_and_try_quorum`; tests (early-success, impossible, exactly-once, concurrent).
- `src-tauri/runtime/src/pool.rs` — route a quorum join through the new store method; park outstanding lanes; tests (quorum-reached-early, quorum-impossible, default-unchanged).
- `src/ipc/pipeline.ts` — add `quorum?: number` to `Join`; `src/ipc/pipeline.test.ts` — round-trip + optionality tests.
- `DOMAIN.md` — register **Quorum** term + the quorum/cancel_on_reject precedence note.
- `docs/superpowers/specs/2026-06-23-parallel-flow-design.md` — P3 addendum.

---

## Task 1: Schema field `Join.quorum`

**Files:**
- Modify: `src-tauri/pipeline/src/model.rs`
- Test: `src-tauri/pipeline/src/model.rs` (inline `#[cfg(test)]`)

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `model.rs`:

```rust
    #[test]
    fn join_quorum_defaults_to_none_when_absent() {
        let json = r#"{"id":"join-1","waits_for":["a","b"],"downstream":"after"}"#;
        let j: Join = serde_json::from_str(json).unwrap();
        assert!(j.quorum.is_none());
        assert!(!j.cancel_on_reject);
    }

    #[test]
    fn join_round_trips_quorum_some() {
        let j = Join {
            id: "join-1".into(),
            waits_for: vec!["a".into(), "b".into(), "c".into()],
            downstream: "after".into(),
            cancel_on_reject: false,
            quorum: Some(2),
        };
        let s = serde_json::to_string(&j).unwrap();
        let back: Join = serde_json::from_str(&s).unwrap();
        assert_eq!(j, back);
        assert_eq!(back.quorum, Some(2));
    }

    #[test]
    fn join_omits_quorum_from_json_when_none() {
        let j = Join { id: "j".into(), waits_for: vec!["a".into(), "b".into()], downstream: "after".into(), cancel_on_reject: false, quorum: None };
        let s = serde_json::to_string(&j).unwrap();
        assert!(!s.contains("quorum"), "None quorum must not be serialized: {s}");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test -p pipeline join_quorum 2>&1 | tail -20`
Expected: FAIL — `Join` has no field `quorum`.

- [ ] **Step 3: Add the field**

In `model.rs`, extend the `Join` struct (after `cancel_on_reject`):

```rust
    /// Quorum policy (P3). When `Some(n)`, the join proceeds to `downstream` as
    /// soon as `n` of its lanes approve (early-resolve on success); if reaching
    /// `n` becomes impossible (unsettled lanes + approvals-so-far < n) it resolves
    /// to needs-human. `None` (default) = all-must-approve (the original barrier).
    /// Additive-optional, no SCHEMA_VERSION bump. The rule lives on the
    /// `FanOutGroup` aggregate. PRECEDENCE: when `quorum` is set it governs
    /// success and `cancel_on_reject` is ignored — a reject only matters insofar
    /// as it can make quorum impossible (see DOMAIN.md).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quorum: Option<u32>,
```

- [ ] **Step 4: Fix existing `Join { … }` literals**

Every existing `Join { … }` literal must add `quorum: None`. Locations (compiler will flag each):
- `model.rs` tests: `node_ids_includes_forks_and_joins`, `join_round_trips_cancel_on_reject_true`.
- `validate.rs` tests: `valid_v2_pipeline`.
- `pool.rs` tests: the `Join` in `pipeline_v2_forkjoin` helper area (~line 768).

Append `, quorum: None` to each (and the new test in Step 1 already sets it).

- [ ] **Step 5: Run tests to verify pass**

Run: `cd src-tauri && cargo test -p pipeline 2>&1 | tail -20`
Expected: PASS (all pipeline tests).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/pipeline/src/model.rs
git commit -m "feat(pipeline): add Join.quorum optional field (P3, additive)"
```

---

## Task 2: Validation — quorum bounds (1..=lanes)

**Files:**
- Modify: `src-tauri/pipeline/src/validate.rs`
- Test: `src-tauri/pipeline/src/validate.rs` (inline)

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `validate.rs`:

```rust
    #[test]
    fn quorum_within_bounds_is_accepted() {
        let mut p = valid_v2_pipeline();
        p.joins[0].quorum = Some(1);
        assert_eq!(validate(&p), Ok(()));
        p.joins[0].quorum = Some(2); // == number of lanes (all-must-approve equiv)
        assert_eq!(validate(&p), Ok(()));
    }

    #[test]
    fn quorum_zero_is_rejected() {
        let mut p = valid_v2_pipeline();
        p.joins[0].quorum = Some(0);
        assert_eq!(validate(&p), Err(PipelineValidationError::QuorumOutOfRange { join: "join-1".into(), quorum: 0, lanes: 2 }));
    }

    #[test]
    fn quorum_above_lane_count_is_rejected() {
        let mut p = valid_v2_pipeline();
        p.joins[0].quorum = Some(3); // only 2 lanes
        assert_eq!(validate(&p), Err(PipelineValidationError::QuorumOutOfRange { join: "join-1".into(), quorum: 3, lanes: 2 }));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test -p pipeline quorum 2>&1 | tail -20`
Expected: FAIL — no variant `QuorumOutOfRange`.

- [ ] **Step 3: Add the error variant**

In the `PipelineValidationError` enum:

```rust
    #[error("join '{join}' quorum {quorum} out of range (must be 1..={lanes})")]
    QuorumOutOfRange { join: String, quorum: u32, lanes: u32 },
```

- [ ] **Step 4: Add the bounds check**

In `validate()`, inside the `for join in &p.joins` loop (after the `waits_for` team checks, before/after the downstream check — anywhere in that loop body):

```rust
        if let Some(q) = join.quorum {
            let lanes = join.waits_for.len() as u32;
            if q < 1 || q > lanes {
                return Err(PipelineValidationError::QuorumOutOfRange { join: join.id.clone(), quorum: q, lanes });
            }
        }
```

- [ ] **Step 5: Run tests to verify pass**

Run: `cd src-tauri && cargo test -p pipeline 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/pipeline/src/validate.rs
git commit -m "feat(pipeline): validate Join.quorum bounds 1..=lanes (P3)"
```

---

## Task 3: Aggregate decision — `quorum_continuation`

**Files:**
- Modify: `src-tauri/runtime/src/fanout_group.rs`
- Test: `src-tauri/runtime/src/fanout_group.rs` (inline)

The aggregate already knows `expected_lanes`. Given the recorded (settled) lane verdicts and the quorum `n`:
- `approvals` = count of recorded verdicts that are `Approve`.
- `unsettled` = `expected_lanes.len() - recorded.len()` (recorded never exceeds expected; pending lanes are simply absent from `recorded`).
- If `approvals >= n` → `Some(Downstream)`.
- Else if `approvals + unsettled < n` → `Some(NeedsHuman)` (N now impossible).
- Else → `None` (keep waiting).

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `fanout_group.rs` (the `group()` helper has lanes a+b; add a 3-lane helper):

```rust
    fn group3() -> FanOutGroup {
        FanOutGroup {
            id: "G-3".into(),
            pipeline: "pipe".into(),
            join_target: "join-1".into(),
            downstream: "after".into(),
            expected_lanes: vec!["lane-a".into(), "lane-b".into(), "lane-c".into()],
            completed: false,
        }
    }

    #[test]
    fn quorum_reached_resolves_downstream_without_waiting() {
        let g = group3(); // 3 lanes, quorum 2
        let recorded = vec![
            LaneVerdict { lane: "lane-a".into(), verdict: Verdict::Approve },
            LaneVerdict { lane: "lane-b".into(), verdict: Verdict::Approve },
        ];
        assert_eq!(g.quorum_continuation(&recorded, 2), Some(Continuation::Downstream("after".into())));
    }

    #[test]
    fn quorum_not_yet_reached_keeps_waiting() {
        let g = group3(); // 3 lanes, quorum 2
        let recorded = vec![LaneVerdict { lane: "lane-a".into(), verdict: Verdict::Approve }];
        // 1 approval, 2 unsettled: 1+2=3 >= 2, not yet 2 approvals => wait
        assert_eq!(g.quorum_continuation(&recorded, 2), None);
    }

    #[test]
    fn quorum_impossible_resolves_needs_human() {
        let g = group3(); // 3 lanes, quorum 2
        let recorded = vec![
            LaneVerdict { lane: "lane-a".into(), verdict: Verdict::Reject },
            LaneVerdict { lane: "lane-b".into(), verdict: Verdict::Reject },
        ];
        // 0 approvals, 1 unsettled: 0+1=1 < 2 => impossible => needs-human
        assert_eq!(g.quorum_continuation(&recorded, 2), Some(Continuation::NeedsHuman));
    }

    #[test]
    fn quorum_one_resolves_on_first_approval() {
        let g = group3();
        let recorded = vec![LaneVerdict { lane: "lane-a".into(), verdict: Verdict::Approve }];
        assert_eq!(g.quorum_continuation(&recorded, 1), Some(Continuation::Downstream("after".into())));
    }

    #[test]
    fn quorum_equal_to_lanes_is_all_must_approve() {
        let g = group(); // 2 lanes, quorum 2
        let one = vec![LaneVerdict { lane: "lane-a".into(), verdict: Verdict::Approve }];
        assert_eq!(g.quorum_continuation(&one, 2), None); // still need lane-b
        let both = vec![
            LaneVerdict { lane: "lane-a".into(), verdict: Verdict::Approve },
            LaneVerdict { lane: "lane-b".into(), verdict: Verdict::Approve },
        ];
        assert_eq!(g.quorum_continuation(&both, 2), Some(Continuation::Downstream("after".into())));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test -p runtime quorum_ 2>&1 | tail -20`
Expected: FAIL — no method `quorum_continuation`.

- [ ] **Step 3: Implement the method**

In `impl FanOutGroup` (after `early_cancel_continuation`):

```rust
    /// The continuation under a quorum policy (P3): proceed to `downstream` as
    /// soon as `quorum` lanes approve (early-resolve on success); resolve to
    /// needs-human once reaching `quorum` is impossible (unsettled lanes plus
    /// approvals-so-far < quorum); otherwise `None` (keep waiting). The aggregate
    /// owns this rule — the store asks for it behind the completes-once guard.
    /// A reject/revise-cap lane simply counts as a non-approval; quorum governs
    /// success, so `cancel_on_reject` does not apply when a quorum is set.
    ///
    /// Precondition (vet F2): the caller passes a validated quorum in
    /// `1..=expected_lanes.len()` — `validate.rs` enforces this at load/save, so
    /// the aggregate never sees an out-of-range value. `saturating_sub` below
    /// keeps the arithmetic panic-free regardless.
    pub fn quorum_continuation(&self, recorded: &[LaneVerdict], quorum: u32) -> Option<Continuation> {
        let approvals = recorded.iter().filter(|r| r.verdict == Verdict::Approve).count() as u32;
        if approvals >= quorum {
            return Some(Continuation::Downstream(self.downstream.clone()));
        }
        let unsettled = (self.expected_lanes.len() as u32).saturating_sub(recorded.len() as u32);
        if approvals + unsettled < quorum {
            return Some(Continuation::NeedsHuman);
        }
        None
    }
```

- [ ] **Step 4: Run tests to verify pass**

Run: `cd src-tauri && cargo test -p runtime fanout_group 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/runtime/src/fanout_group.rs
git commit -m "feat(runtime): FanOutGroup.quorum_continuation early-resolve rule (P3)"
```

---

## Task 4: Store barrier — `record_and_try_quorum`

**Files:**
- Modify: `src-tauri/runtime/src/fanout_store.rs`
- Test: `src-tauri/runtime/src/fanout_store.rs` (inline)

Mirrors `record_and_try_complete` but drives `quorum_continuation`. Key difference: it must NOT bail out on pending lanes — quorum can resolve mid-flight. It records the lane, reads all NON-pending verdicts, asks the aggregate; only if the aggregate returns `Some(_)` does it attempt the completes-once guard.

- [ ] **Step 1: Write the failing tests**

Add to `fanout_store.rs` tests (the `seeded` helper seeds lanes a+b; add a 3-lane variant):

```rust
    fn group3() -> FanOutGroup {
        FanOutGroup {
            id: "G-3".into(), pipeline: "pipe".into(), join_target: "join-1".into(),
            downstream: "after".into(),
            expected_lanes: vec!["lane-a".into(), "lane-b".into(), "lane-c".into()],
            completed: false,
        }
    }

    async fn seeded3(store: &FanOutStore) {
        store.create(&group3()).await.unwrap();
        for l in ["lane-a", "lane-b", "lane-c"] {
            store.seed_lane("G-3", l).await.unwrap();
        }
    }

    #[tokio::test]
    async fn quorum_reached_early_completes_to_downstream() {
        let store = FanOutStore::new(fresh_pool().await);
        seeded3(&store).await; // 3 lanes
        let first = store.record_and_try_quorum("G-3", "lane-a", Verdict::Approve, 2).await.unwrap();
        assert_eq!(first, BarrierOutcome::Parked); // 1 of 2
        let second = store.record_and_try_quorum("G-3", "lane-b", Verdict::Approve, 2).await.unwrap();
        assert_eq!(second, BarrierOutcome::Completed(Continuation::Downstream("after".into())));
        assert!(store.load("G-3").await.unwrap().completed);
    }

    #[tokio::test]
    async fn quorum_impossible_completes_to_needs_human() {
        let store = FanOutStore::new(fresh_pool().await);
        seeded3(&store).await; // 3 lanes, quorum 2
        store.record_and_try_quorum("G-3", "lane-a", Verdict::Reject, 2).await.unwrap();
        // after second reject: 0 approvals, 1 unsettled < 2 => needs-human
        let out = store.record_and_try_quorum("G-3", "lane-b", Verdict::Reject, 2).await.unwrap();
        assert_eq!(out, BarrierOutcome::Completed(Continuation::NeedsHuman));
    }

    #[tokio::test]
    async fn quorum_straggler_after_completion_parks() {
        let store = FanOutStore::new(fresh_pool().await);
        seeded3(&store).await;
        store.record_and_try_quorum("G-3", "lane-a", Verdict::Approve, 2).await.unwrap();
        store.record_and_try_quorum("G-3", "lane-b", Verdict::Approve, 2).await.unwrap(); // completes
        let straggler = store.record_and_try_quorum("G-3", "lane-c", Verdict::Approve, 2).await.unwrap();
        assert_eq!(straggler, BarrierOutcome::Parked);
    }

    #[tokio::test]
    async fn quorum_concurrent_settle_yields_exactly_one_completion() {
        let store = std::sync::Arc::new(FanOutStore::new(fresh_pool().await));
        seeded3(&store).await;
        store.record_and_try_quorum("G-3", "lane-a", Verdict::Approve, 2).await.unwrap();
        let s1 = store.clone();
        let s2 = store.clone();
        // two lanes approve concurrently; both observe quorum reached
        let h1 = tokio::spawn(async move { s1.record_and_try_quorum("G-3", "lane-b", Verdict::Approve, 2).await.unwrap() });
        let h2 = tokio::spawn(async move { s2.record_and_try_quorum("G-3", "lane-c", Verdict::Approve, 2).await.unwrap() });
        let (a, b) = (h1.await.unwrap(), h2.await.unwrap());
        let completions = [&a, &b].iter().filter(|o| matches!(o, BarrierOutcome::Completed(_))).count();
        assert_eq!(completions, 1, "exactly one caller completes the quorum barrier");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test -p runtime quorum 2>&1 | tail -20`
Expected: FAIL — no method `record_and_try_quorum`.

- [ ] **Step 3: Implement the method**

In `impl FanOutStore` (after `record_failure_and_early_cancel`):

```rust
    /// Quorum path (P3): record this lane's verdict, then ask the aggregate
    /// whether the quorum is decided. The aggregate resolves to Downstream once
    /// `quorum` lanes approve (early, without waiting for the rest) or to
    /// needs-human once that becomes impossible; until then it returns None and
    /// this lane parks. When decided, the SAME completes-once conditional UPDATE
    /// guard the full barrier uses arbitrates the single winner — exactly-once is
    /// preserved against concurrent settles and stragglers.
    pub async fn record_and_try_quorum(
        &self,
        group_id: &str,
        lane: &str,
        verdict: Verdict,
        quorum: u32,
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

        // 2. Read settled (non-pending) lane verdicts + ask the aggregate.
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT lane, verdict FROM fanout_lanes WHERE group_id = ? AND verdict != 'pending'")
                .bind(group_id)
                .fetch_all(&self.pool)
                .await?;
        let recorded: Vec<LaneVerdict> = rows
            .into_iter()
            .map(|(lane, v)| {
                parse_verdict(&v)
                    .map(|verdict| LaneVerdict { lane, verdict })
                    .ok_or_else(|| FanOutStoreError::BadVerdict(v.clone()))
            })
            .collect::<Result<_, _>>()?;
        let g = self.load(group_id).await?;
        let decided = match g.quorum_continuation(&recorded, quorum) {
            Some(c) => c,
            None => return Ok(BarrierOutcome::Parked),
        };

        // 3. Quorum decided — attempt the completes-once guard.
        let res = sqlx::query("UPDATE fanout_groups SET completed=1 WHERE id=? AND completed=0")
            .bind(group_id)
            .execute(&self.pool)
            .await?;
        if res.rows_affected() == 0 {
            return Ok(BarrierOutcome::Parked);
        }
        Ok(BarrierOutcome::Completed(decided))
    }
```

- [ ] **Step 4: Run tests to verify pass**

Run: `cd src-tauri && cargo test -p runtime fanout_store 2>&1 | tail -20`
Expected: PASS (existing + new quorum tests).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/runtime/src/fanout_store.rs
git commit -m "feat(runtime): FanOutStore.record_and_try_quorum guarded N-of-M barrier (P3)"
```

---

## Task 5: Pool — route quorum joins through the new barrier

**Files:**
- Modify: `src-tauri/runtime/src/pool.rs` (the `resolve_barrier` fn, ~line 358-418)
- Test: `src-tauri/runtime/src/pool.rs` (inline)

`resolve_barrier` currently chooses between `record_failure_and_early_cancel` and `record_and_try_complete`. Add a third branch: if the join has `quorum: Some(n)`, call `record_and_try_quorum` (quorum governs; ignore `cancel_on_reject` per DD7). On completion of a quorum join, park outstanding lanes (same as early-cancel) so unstarted siblings aren't run.

- [ ] **Step 1: Write the failing tests**

First locate the existing `pipeline_v2_forkjoin` test helper and the `early_cancel_*` tests (~line 760-950) to match their style. Add:

```rust
    fn pipeline_v2_forkjoin_quorum(q: u32) -> Pipeline {
        let mut p = pipeline_v2_forkjoin();
        p.joins[0].quorum = Some(q);
        p
    }

    #[tokio::test]
    async fn quorum_reached_resolves_downstream_before_other_lane_settles() {
        let ctx = test_ctx(fresh_pool().await).await;
        let p = pipeline_v2_forkjoin_quorum(1); // 2 lanes, quorum 1
        // (mirror early_cancel_resolves_to_needs_human_before_other_lane_runs:
        //  create the group + seed lanes a,b; build a lane-a task with
        //  group_id/lane/join_target set, settle it Approve via resolve_barrier)
        // assert resolve_barrier returns Some(("after", Queued)) — quorum 1 met by lane-a
    }

    #[tokio::test]
    async fn quorum_default_none_keeps_full_barrier() {
        // pipeline_v2_forkjoin() has quorum None: one approve must still park
        // (full barrier) — re-uses the existing all-approve path unchanged.
    }
```

NOTE for the implementing agent: copy the exact group-seed + lane-task-build + settle scaffolding from the adjacent `early_cancel_resolves_to_needs_human_before_other_lane_runs` and `cancel_on_reject_false_does_not_short_circuit` tests (same `ctx`, `seed`, `Task` field setup). Assert:
- quorum(1): settling lane-a Approve returns `Some(("after".into(), TaskState::Queued))` and the group is `completed`.
- quorum default None: settling one lane Approve returns the parked outcome (`Some((join_target, TaskState::Done))`) and group NOT completed — identical to today.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test -p runtime quorum_reached_resolves quorum_default 2>&1 | tail -20`
Expected: FAIL (quorum branch not wired → quorum(1) parks instead of completing).

- [ ] **Step 3: Wire the pool branch**

In `resolve_barrier`, replace the `early_cancel`/`is_failure` outcome selection. Read the join once:

```rust
    let join = task
        .join_target
        .as_deref()
        .and_then(|jt| ctx.pipeline.joins.iter().find(|j| j.id == jt));
    let quorum = join.and_then(|j| j.quorum);
    // Quorum governs success; cancel_on_reject only applies to the default
    // (all-must-approve) barrier (DD7).
    let early_cancel = quorum.is_none() && join.map(|j| j.cancel_on_reject).unwrap_or(false);
    let is_failure = verdict == agent_bus_core::Verdict::Reject;

    let outcome = if let Some(q) = quorum {
        ctx.fanout.record_and_try_quorum(&group_id, &lane, verdict, q).await?
    } else if early_cancel && is_failure {
        ctx.fanout.record_failure_and_early_cancel(&group_id, &lane, verdict).await?
    } else {
        ctx.fanout.record_and_try_complete(&group_id, &lane, verdict).await?
    };
```

Then, in the `if let BarrierOutcome::Completed(cont) = outcome` block, broaden the lane-cancellation condition so a completed quorum also parks outstanding lanes:

```rust
        // Stop outstanding lanes when an early resolution (early-cancel OR a
        // quorum decided before all lanes settled) wins, so queued siblings
        // aren't claimed+run wastefully. The continuation is created regardless.
        if (early_cancel && is_failure) || quorum.is_some() {
            ctx.tasks.cancel_outstanding_lanes(&group_id, now_unix()).await?;
        }
```

- [ ] **Step 4: Run tests to verify pass**

Run: `cd src-tauri && cargo test -p runtime 2>&1 | tail -25`
Expected: PASS (all runtime tests, incl. existing fork/join + P2 early-cancel).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/runtime/src/pool.rs
git commit -m "feat(runtime): route quorum joins through record_and_try_quorum + park lanes (P3)"
```

---

## Task 6: Frontend IPC — `Join.quorum`

**Files:**
- Modify: `src/ipc/pipeline.ts`
- Test: `src/ipc/pipeline.test.ts`

- [ ] **Step 1: Write the failing tests**

Add to `pipeline.test.ts` (near the existing `cancel_on_reject` tests):

```ts
  it("Join carries the optional quorum field", () => {
    const j: Join = { id: "join-1", waits_for: ["a", "b", "c"], downstream: "after", quorum: 2 };
    expect(j.quorum).toBe(2);
    const j2: Join = { id: "join-2", waits_for: ["a", "b"], downstream: "after" };
    expect(j2.quorum).toBeUndefined();
  });
```

Also extend the existing round-trip pipeline test (line ~61) to set `quorum: 2` on a join and assert it survives `result.joins[0].quorum === 2`.

- [ ] **Step 2: Run tests to verify they fail**

Run (export PATH if needed): `/opt/homebrew/bin/bun vitest run src/ipc/pipeline.test.ts 2>&1 | tail -20`
Expected: FAIL — `quorum` not on `Join`.

- [ ] **Step 3: Add the field**

In `src/ipc/pipeline.ts`, in `interface Join`, after `cancel_on_reject?`:

```ts
  /** Quorum (P3): proceed to downstream once `quorum` lanes approve (N-of-M);
   *  resolve to needs-human once reaching it is impossible. Omitted/undefined =
   *  all-must-approve (the default barrier). Backend field is `quorum`
   *  (`Option<u32>`, serde-skipped when None). When set, it governs success and
   *  `cancel_on_reject` is ignored. */
  quorum?: number;
```

- [ ] **Step 4: Run tests to verify pass**

Run: `/opt/homebrew/bin/bun vitest run src/ipc/pipeline.test.ts 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/ipc/pipeline.ts src/ipc/pipeline.test.ts
git commit -m "feat(ipc): Join.quorum optional field (P3 N-of-M)"
```

---

## Task 7: Docs — DOMAIN.md + spec addendum

**Files:**
- Modify: `DOMAIN.md`
- Modify: `docs/superpowers/specs/2026-06-23-parallel-flow-design.md`

- [ ] **Step 1a: DOMAIN.md — soften the Join definition (vet F1)**

The base **Join** line (~line 71) currently says "waits for all lanes … all must
approve" — now only the default. Replace it with:

```markdown
- **Join** — a barrier node that waits for its lanes, then continues; by default all-must-approve-else-needs-human, with opt-in policies **Quorum** (N-of-M) and **Early-cancel** varying the resolution
```

- [ ] **Step 1: DOMAIN.md — register the Quorum term**

After the **Early-cancel** bullet (line ~86), add:

```markdown
- **Quorum** — an opt-in per-join policy (`Join.quorum: Option<u32>`, P3): the fan-out group proceeds to `downstream` as soon as N of the M lanes approve (early-resolve on success), and resolves to needs-human once reaching N is impossible (unsettled lanes + approvals-so-far < N). Owned by the `FanOutGroup` aggregate; default (None) = all-must-approve. PRECEDENCE: when a quorum is set it **governs success** and **`cancel_on_reject` is ignored** — a reject counts only as a non-approval that can make quorum impossible.
```

- [ ] **Step 2: Spec addendum**

Append a P3 update note to the **Join semantics** decision bullet in `2026-06-23-parallel-flow-design.md` (parallel to the existing P2 parenthetical):

```markdown
*(v1.1 update — P3: a join may also opt into `quorum: Some(n)`, proceeding to `downstream` as soon as N of M lanes approve and resolving to needs-human once N is impossible. The `FanOutGroup` aggregate owns the rule (`quorum_continuation`); the completes-once guard is unchanged. When set, quorum governs success and `cancel_on_reject` is ignored.)*
```

- [ ] **Step 3: Commit**

```bash
git add DOMAIN.md docs/superpowers/specs/2026-06-23-parallel-flow-design.md
git commit -m "docs: register Quorum term + spec addendum (P3)"
```

---

## Task 8: Full verification + merge

- [ ] **Step 1: Full workspace verification**

```bash
cd src-tauri && cargo test --workspace 2>&1 | tail -30
cargo check --workspace 2>&1 | tail -5
cargo clippy --workspace 2>&1 | tail -15
cd .. && /opt/homebrew/bin/bun vitest run 2>&1 | tail -15
/opt/homebrew/bin/bun run build 2>&1 | tail -15
```

Expected: all green; clippy clean (no warnings).

- [ ] **Step 2: Merge --no-ff into main + tag**

```bash
git checkout main
git merge --no-ff plan-p3-quorum-joins -m "Merge plan-p3-quorum-joins: quorum (N-of-M) joins (P3)"
git tag plan-p3-quorum-joins
```

- [ ] **Step 3: Backlog**

Mark **P3** done in `docs/v1.1-backlog.md` with the tag + a one-line summary, then commit.
