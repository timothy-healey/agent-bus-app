# Model + Schema Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the additive model/schema changes that the graph-builder, A4, and the runtime redesign all build on — `Team.role`, the `workers.default→min` rename (F2), `Team.store.capacity`, and `SCHEMA_VERSION 2→3` — with TS contract mirrors and validation, keeping the existing single-task runtime green.

**Architecture:** Pure additive edits to the `pipeline` crate's `model.rs`/`validate.rs` + their TS mirrors (`src/ipc/pipeline.ts`, `src/wizard/draft.ts`) + the Rust↔TS contract tests. New fields default so every v1/v2 pipeline and the wizard load unchanged; the deeper assembly-line validation (one source, store reachability) is deferred to the runtime-behavior chunk. No runtime behavior changes here.

**Tech Stack:** Rust (serde, schemars), TypeScript, vitest, cargo.

---

### Task 1: `Team.role` (producer | reviewer)

**Files:**
- Modify: `src-tauri/pipeline/src/model.rs`
- Test: same file `#[cfg(test)] mod tests`

- [ ] **Step 1: Write the failing tests** (add to `mod tests`)

```rust
#[test]
fn team_role_defaults_to_producer_when_absent() {
    let json = r#"{"id":"t","name":"T","prompt":"p.md","scope":{},"outputs":{}}"#;
    let t: Team = serde_json::from_str(json).unwrap();
    assert_eq!(t.role, Role::Producer);
}

#[test]
fn role_serializes_lowercase_and_round_trips() {
    assert_eq!(serde_json::to_string(&Role::Reviewer).unwrap(), "\"reviewer\"");
    let r: Role = serde_json::from_str("\"producer\"").unwrap();
    assert_eq!(r, Role::Producer);
}
```

- [ ] **Step 2: Run, verify fail**

Run: `cd src-tauri && cargo test -p pipeline team_role_defaults`
Expected: FAIL (no `Role`, no `Team.role`).

- [ ] **Step 3: Add the enum + field**

In `model.rs`, after `NodeKind` (or near `Team`), add:

```rust
/// A team's role in the pipeline (graph-builder vet F8). A **reviewer** emits a
/// verdict (approve/revise/reject) on each item it sees; a **producer** (incl.
/// implementers) hands its output forward without a verdict. Additive enum,
/// default `producer` (matches L1 "producers default approve, reviewers judge").
/// Routing/verdict semantics consume this in the runtime-behavior chunk; the
/// graph builder reads it for role-aware edges (replacing the `teamRole` regex).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    #[default]
    Producer,
    Reviewer,
}
```

In `struct Team`, add the field (after `workers`):

```rust
    /// Producer vs reviewer (vet F8). Default producer; see `Role`.
    #[serde(default)]
    pub role: Role,
```

- [ ] **Step 4: Fix every `Team { .. }` struct literal** that now lacks `role`

The compiler will list them. Add `role: Role::default(),` (or the right role) to each. Known sites: `model.rs` `sample_team` (`mod tests`), `validate.rs:~292`, `store.rs:~132,162`, `resolve.rs:~61`, `runtime/src/router.rs:~139`, `runtime/src/pool.rs:~620`, `runtime/src/api.rs:~324,350`, `pipeline/src/draft.rs` `to_pipeline` (`model::Team{..}`), `pipeline/src/contract_tests.rs:~47`. Use `role: Role::default(),` everywhere except where a test specifically wants a reviewer.

- [ ] **Step 5: Run, verify pass**

Run: `cd src-tauri && cargo test -p pipeline role`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/pipeline/src/model.rs && git add -u src-tauri
git commit -m "feat(pipeline): add Team.role (producer|reviewer, default producer) — vet F8"
```

---

### Task 2: Rename `Workers.default → min` (F2)

**Files:**
- Modify: `src-tauri/pipeline/src/model.rs` (struct + Default + tests)
- Modify: `src-tauri/pipeline/src/contract_tests.rs:47,141,144`
- Modify: `src/ipc/pipeline.ts:47-50`, `src/wizard/draft.ts:30`
- Modify: any `Workers { default: .., max: .. }` literal (grep)

- [ ] **Step 1: Update the failing test first** (`model.rs`)

Change `workers_default_is_one_one`:

```rust
#[test]
fn workers_default_is_one_one() {
    assert_eq!(Workers::default(), Workers { min: 1, max: 1 });
}
```

- [ ] **Step 2: Run, verify fail**

Run: `cd src-tauri && cargo test -p pipeline workers_default_is_one_one`
Expected: FAIL (`min` unknown).

- [ ] **Step 3: Rename the field**

In `model.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workers {
    /// Minimum live workers for this team's pool (the always-on floor). Renamed
    /// from `default` to align code with the context-map invariant
    /// `workers.count ≥ team.workers.min` (vet F2). UI label: "Scale (min·max)".
    pub min: u32,
    pub max: u32,
}

impl Default for Workers {
    fn default() -> Self {
        Self { min: 1, max: 1 }
    }
}
```

- [ ] **Step 4: Fix the literals**

`model.rs` test `Workers { default: 1, max: 3 }`→`min`; `contract_tests.rs:47` `Workers { default: 1, max: 3 }`→`min`; `contract_tests.rs:144` `Workers { default: 1, max: 4 }`→`min`; and update the comment at `contract_tests.rs:141` to `interface Workers { min; max }`.

- [ ] **Step 5: Mirror TS** — `src/ipc/pipeline.ts`:

```ts
export interface Workers {
  min: number;
  max: number;
}
```

`src/wizard/draft.ts:30`: `workers: { min: 1, max: 1 },`

- [ ] **Step 6: Run all affected tests**

Run: `cd src-tauri && cargo test -p pipeline` then `cd .. && npx vitest run src/ipc/pipeline` (contract) and `npx tsc --noEmit`.
Expected: PASS / clean.

- [ ] **Step 7: Commit**

```bash
git add -u && git commit -m "refactor(pipeline): rename Workers.default -> min, align code with context-map (vet F2)"
```

---

### Task 3: `Team.store.capacity` (bounded-buffer input store)

**Files:**
- Modify: `src-tauri/pipeline/src/model.rs`
- Modify: `src/ipc/pipeline.ts`, `src/wizard/draft.ts`
- Test: `model.rs` tests + contract test

- [ ] **Step 1: Failing tests** (`model.rs`)

```rust
#[test]
fn team_store_defaults_when_absent() {
    let json = r#"{"id":"t","name":"T","prompt":"p.md","scope":{},"outputs":{}}"#;
    let t: Team = serde_json::from_str(json).unwrap();
    assert_eq!(t.store.capacity, DEFAULT_STORE_CAPACITY);
}

#[test]
fn store_round_trips() {
    let s = Store { capacity: 3 };
    let back: Store = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
    assert_eq!(back.capacity, 3);
}
```

- [ ] **Step 2: Run, verify fail**

Run: `cd src-tauri && cargo test -p pipeline team_store_defaults`
Expected: FAIL.

- [ ] **Step 3: Add `Store` + the field + constant**

In `model.rs`:

```rust
/// Default bounded-store capacity when a team omits one (runtime redesign). A
/// generous default so existing v2 pipelines keep flowing; authors tune it in
/// the graph builder. The generator (source) team has no input store and ignores it.
pub const DEFAULT_STORE_CAPACITY: u32 = 8;

/// A team's input **store**: the bounded buffer feeding the team (runtime
/// redesign / DOMAIN Store). `capacity` is the WIP limit; a full store applies
/// backpressure to the upstream. Additive-optional with a default so v2
/// pipelines load unchanged; the runtime-behavior chunk enforces occupancy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Store {
    pub capacity: u32,
}

impl Default for Store {
    fn default() -> Self {
        Self { capacity: DEFAULT_STORE_CAPACITY }
    }
}
```

In `struct Team`, after `role`:

```rust
    /// The team's bounded input store (runtime redesign). Defaulted; see `Store`.
    #[serde(default)]
    pub store: Store,
```

- [ ] **Step 4: Fix `Team { .. }` literals** — add `store: Store::default(),` to every site from Task 1 Step 4.

- [ ] **Step 5: Mirror TS** — `src/ipc/pipeline.ts`:

```ts
export interface Store {
  capacity: number;
}
```

Add to the `Team` interface: `store: Store;` and `role: "producer" | "reviewer";`. `src/wizard/draft.ts` default team: add `role: "producer", store: { capacity: 8 },`.

- [ ] **Step 6: Run, verify pass**

Run: `cd src-tauri && cargo test -p pipeline store` then `cd .. && npx tsc --noEmit`.
Expected: PASS / clean.

- [ ] **Step 7: Commit**

```bash
git add -u && git commit -m "feat(pipeline): add Team.store.capacity (bounded-buffer input store)"
```

---

### Task 4: `SCHEMA_VERSION 2 → 3` + validation

**Files:**
- Modify: `src-tauri/pipeline/src/model.rs` (const + test)
- Modify: `src-tauri/pipeline/src/validate.rs` (accept v3; capacity ≥ 1)

- [ ] **Step 1: Failing tests**

`model.rs`: change `schema_version_constant_is_two` →

```rust
#[test]
fn schema_version_constant_is_three() {
    assert_eq!(SCHEMA_VERSION, 3);
}
```

`validate.rs` (add to its tests):

```rust
#[test]
fn rejects_zero_store_capacity() {
    let mut p = valid_minimal_pipeline(); // existing helper
    p.teams[0].store.capacity = 0;
    let err = validate(&p).unwrap_err();
    assert!(format!("{err:?}").contains("StoreCapacity"));
}

#[test]
fn accepts_schema_version_three() {
    let mut p = valid_minimal_pipeline();
    p.schema_version = 3;
    assert!(validate(&p).is_ok());
}
```

- [ ] **Step 2: Run, verify fail**

Run: `cd src-tauri && cargo test -p pipeline schema_version_constant_is_three rejects_zero_store_capacity`
Expected: FAIL.

- [ ] **Step 3: Bump the constant + accept v3 + capacity rule**

`model.rs`: `pub const SCHEMA_VERSION: u32 = 3;`

`validate.rs`: wherever the supported-version check lives (currently accepts 1 and 2), accept `1..=3`. Add a `StoreCapacityZero { team: String }` variant to the validation error enum and a rule in `validate`:

```rust
for t in &p.teams {
    if t.store.capacity == 0 {
        return Err(ValidationError::StoreCapacityZero { team: t.id.clone() });
    }
}
```

(Match the existing error-enum style/naming in `validate.rs`.)

- [ ] **Step 4: Run, verify pass + full pipeline suite**

Run: `cd src-tauri && cargo test -p pipeline`
Expected: PASS (update any test that asserted `schema_version` 1/2 is the *max* supported; v3 is now accepted, 1/2 still load).

- [ ] **Step 5: Commit**

```bash
git add -u && git commit -m "feat(pipeline): SCHEMA_VERSION 2->3; validate store.capacity >= 1"
```

---

### Task 5: Contract test for the new fields

**Files:**
- Modify: `src-tauri/pipeline/src/contract_tests.rs`

- [ ] **Step 1: Add a wire-contract test** pinning the new Team fields to the TS shape

```rust
#[test]
fn team_role_and_store_wire_shape() {
    let t = sample_full_team(); // existing helper that builds a Team
    let v = serde_json::to_value(&t).unwrap();
    assert_eq!(v["role"], serde_json::json!("producer"));
    assert_eq!(v["store"]["capacity"], serde_json::json!(8));
    assert!(v["workers"].get("min").is_some(), "workers.min replaces default");
    assert!(v["workers"].get("default").is_none());
}
```

(Adapt to the file's existing helper/naming; if no `sample_full_team`, build a `Team` inline with `role`/`store`/`workers` set.)

- [ ] **Step 2: Run, verify pass**

Run: `cd src-tauri && cargo test -p pipeline team_role_and_store_wire_shape`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add -u && git commit -m "test(pipeline): wire-contract for Team.role/store + workers.min"
```

---

### Task 6: DOMAIN.md / context-map.md language

**Files:**
- Modify: `DOMAIN.md`, `docs/context-map.md`

- [ ] **Step 1: Register the language**

- `DOMAIN.md` Pipeline Authoring ubiquitous language: **Role** ("a team is a *producer* — hands output forward — or a *reviewer* — emits an approve/revise/reject verdict; default producer"), **Store** ("a team's bounded input buffer; `capacity` is the WIP limit that drives backpressure"), and rename the **Scale** dial language to `workers.min`/`max`.
- `docs/context-map.md`: bump the `Pipeline ↔ Runtime` shared-kernel entry to `schema_version: 3`; fix the `workers.min` invariant wording (was `default`, code now matches).

- [ ] **Step 2: Commit**

```bash
git add DOMAIN.md docs/context-map.md
git commit -m "docs(domain): register Role + Store language; schema_version 3; workers.min"
```

---

### Final verification (whole chunk)

- [ ] `cd src-tauri && cargo test --workspace` — all green.
- [ ] `cd src-tauri && cargo clippy --workspace --all-targets -- -D warnings` — clean.
- [ ] `cd .. && npx vitest run` — all green.
- [ ] `npx tsc --noEmit` — clean.
- [ ] `npx vite build` (or `bun run build`) — green.
- [ ] Tag: `git tag plan-model-schema-foundation`.

## Spec coverage check

- `Team.role` (vet F8) → Task 1. ✓
- `workers.default→min` (vet F2) → Task 2. ✓
- `Team.store.capacity` + `SCHEMA_VERSION 3` (runtime spec, schema pulled forward) → Tasks 3–4. ✓
- TS mirrors + contract (DS-Schema/contract-test discipline) → Tasks 2,3,5. ✓
- Language (DOMAIN/context-map) → Task 6. ✓
- Deferred to runtime-behavior chunk: source-stage designation, store reachability, occupancy enforcement (NOT in this chunk — keeps the existing single-task runtime green).
