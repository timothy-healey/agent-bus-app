# Runtime Redesign ④a — Foundation (additive) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. Detailed design: `docs/superpowers/specs/2026-06-24-runtime-bounded-buffer-pipeline-design.md` (read first). This is the FIRST of the runtime sub-plans (the spec lists 8). It is deliberately **additive** — it adds the new aggregates/tables/validation WITHOUT touching the existing single-task pool, so the app stays green and runnable throughout.

**Goal:** Land the bounded-buffer foundation — `stores`/`runs`/`generator_ledger` tables + migrations, the `Store` aggregate (occupancy ≤ capacity, atomic), the `Run` aggregate (completes-once), the generator ledger, and the deferred schema validation (source-stage designation + store reachability) — all additive, with the existing runtime untouched and green.

**Architecture:** New modules in the `runtime` crate mirroring the existing `task_store.rs` / `fanout_store.rs` patterns and their atomic conditional-UPDATE guard (`UPDATE … WHERE … AND <cond>`; rows-affected = the single winner). No wiring into `pool.rs` yet — that is plan ④b (worker pools). Migrations are append-only after 011 (A4's skill_sources). SCHEMA_VERSION stays 3 (already bumped in chunk ①).

**Tech Stack:** Rust (sqlx, async), SQLite, the existing migration registration in `app/src/lib.rs`.

---

## Reconciliation (do FIRST)

- Chunk ① already added `Team.store.capacity` + `SCHEMA_VERSION = 3` and `validate.rs` accepts `1..=3` and rejects `store.capacity == 0`. This plan ADDS the remaining assembly-line validation (source designation, store reachability) that chunk ① deferred.
- Current migration max is **011** (A4 `skill_sources`). This plan adds **012**. Register every new migration in BOTH migration lists in `app/src/lib.rs` and bump the `user_version` idempotency test (11→ the new max).
- Mirror the atomic-guard idiom already proven in `runtime/src/task_store.rs` (`claim_next_for_stage`) and `runtime/src/fanout_store.rs` (`UPDATE fanout_groups SET completed=1 WHERE id=? AND completed=0`).

## Tasks (TDD)

- [ ] **Task 1 — Migration 012 (schema).** `src-tauri/app/migrations/012_runtime_stores.sql` (append-only):
  - `runs` (`id` PK, `pipeline` TEXT, `project_id` TEXT, `generator_dry` INTEGER DEFAULT 0, `completed` INTEGER DEFAULT 0, `created_at` INTEGER).
  - `stores` (`run_id` TEXT, `stage` TEXT, `capacity` INTEGER NOT NULL, `occupancy` INTEGER NOT NULL DEFAULT 0, PRIMARY KEY(`run_id`,`stage`)).
  - `generator_ledger` (`run_id` TEXT, `stage` TEXT, `candidate_key` TEXT, PRIMARY KEY(`run_id`,`stage`,`candidate_key`)).
  - `tasks` gains `run_id TEXT` + `item_key TEXT` (nullable; the work-item identity — used by ④b+).
  Register in BOTH `app/src/lib.rs` migration lists; bump the `user_version` test. Commit.
- [ ] **Task 2 — `Store` aggregate** (`runtime/src/store.rs`, new) + tests. Methods over the sqlx pool:
  - `ensure(run_id, stage, capacity)` — idempotent create (INSERT OR IGNORE).
  - `reserve(run_id, stage) -> bool` — `UPDATE stores SET occupancy=occupancy+1 WHERE run_id=? AND stage=? AND occupancy<capacity`; rows-affected==1 ⇒ won a slot (true), 0 ⇒ full (false, backpressure).
  - `release(run_id, stage)` — `occupancy=occupancy-1 WHERE occupancy>0` (undo a reservation on failure).
  - `occupancy(run_id, stage)` / `is_full`.
  Tests: reserve respects capacity (loop reserve to cap then fails); concurrent over-reserve yields exactly `capacity` winners (spawn N tasks); release frees a slot. Use an in-memory sqlite pool with the migration applied (mirror existing store tests). Commit.
- [ ] **Task 3 — `Run` aggregate** (`runtime/src/run_store.rs`, new) + tests. Methods:
  - `create(run) -> Run`; `get`; `set_generator_dry(run_id)`.
  - `try_complete(run_id) -> bool` — `UPDATE runs SET completed=1 WHERE id=? AND completed=0`; rows-affected==1 ⇒ this caller completed it (exactly-once). (The "dry + stores empty + no running workers" PRECONDITION check is computed by the caller in ④b/④h; this method is the single-writer guard.)
  Tests: completes exactly once under a simulated concurrent double-call. Commit.
- [ ] **Task 4 — Generator ledger** (`runtime/src/generator_ledger.rs`, new) + tests. Methods:
  - `record_keys(run_id, stage, keys: &[String]) -> u64` (INSERT OR IGNORE; returns rows inserted = how many were NEW).
  - `found_keys(run_id, stage) -> HashSet<String>` (the already-found set handed to the generator).
  Tests: dedup (re-recording a key inserts 0); found_keys returns the set. Commit.
- [ ] **Task 5 — Deferred schema validation** (`pipeline/src/validate.rs`) + tests:
  - **Source designation:** exactly one team has no inbound route (the entry/source); error `NoSource` / `MultipleSources` if zero / >1. (Compute inbound by scanning every node's routes + fork lanes + gate/join downstreams.)
  - **Store reachability:** every non-source team is reachable via routes from the source (extend the existing reachability check to assert each team's store is fed). Reuse the existing reachability helper if present.
  Add error variants matching the file's enum style. Tests: a 2-source pipeline rejected; an unreachable team rejected; the valid bundled-shape pipeline still passes. Commit.
- [ ] **Task 6 — DOMAIN.md / context-map.md.** Register **Run**, **Store** (aggregate, occupancy≤capacity invariant), **Generator ledger**, **Work-item (run_id+item_key)**; note migration 012. Commit.

## Verification gates (all must pass before tag)

- `cd src-tauri && cargo test --workspace`
- `cd src-tauri && cargo clippy --workspace --all-targets -- -D warnings`
- `cd /Users/tim/projects/agent-bus-app && npx vitest run` (should be untouched/green — backend-only chunk)
- `npx tsc --noEmit`
- `bun run build`
- Tag: `git tag plan-runtime-4a`

## Spec coverage (this sub-plan)

- Store aggregate + occupancy≤capacity atomic guard → Task 2. ✓
- Run aggregate + completes-once guard → Task 3. ✓
- Generator ledger (dedup/dry source) → Task 4. ✓
- stores/runs/generator_ledger tables + tasks run_id/item_key → Task 1. ✓
- Deferred source designation + store reachability validation → Task 5. ✓
- NOT in this sub-plan (later runtime sub-plans): worker pools + block-before-claim (④b), generator loop (④c), output contract/L1 (④d), gates-as-stores (④e), fork/join integration (④f), run lifecycle + frontend (④g/h). The existing single-task pool stays untouched and green here.

## Constraints

Local commits only, NEVER push — commit per task DIRECTLY ON `main`. Additive only: do NOT modify `pool.rs`/`router.rs`/the activator in this sub-plan. New aggregates mirror the existing atomic-guard pattern; each owns exactly one invariant.
