# R2 — Auto-meter Reactive Brake Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the already-built reactive auto-meter brake user-controllable: a Settings toggle flips `auto_meter_enabled` via a new usage-telemetry command, the existing sweep trips/releases Runtime's brake by `AUTO_METER_REASON`, and the topbar meter reflects the auto-braked state. Ships defaulting OFF.

**Architecture:** The brake DECISION stays in `usage_telemetry` (pure `brake_policy::decide` + `snapshot::auto_brake_decision`, already present). The brake STATE stays in `runtime::brake::Brake`. They are wired only at the composition root (`app/src/lib.rs`) — the existing 15s sweep loop already applies the decision to the brake by reason; we change nothing there. The new work is: (a) a `usage_set_auto_meter(enabled)` Tauri command in the Usage Telemetry OHS that writes the single config row; (b) surfacing `auto_meter_enabled` on `UsageSnapshot` so the UI can render toggle state; (c) the Settings toggle + IPC; (d) tests proving the decision trips/releases by reason and that the command round-trips through `load_config`. No new cross-context dependency; `usage_telemetry` stays a pure supplier (depends only on `agent_bus_core` + `workspace`).

**Tech Stack:** Rust (sqlx/sqlite, tokio, tauri 2, serde), TypeScript/React (vitest, @testing-library/react), bun.

---

## File Structure

- `src-tauri/usage_telemetry/src/snapshot.rs` — add `auto_meter_enabled: bool` to `UsageSnapshot`; set it in `compute_snapshot`.
- `src-tauri/usage_telemetry/src/api.rs` — add `usage_set_auto_meter` command + register in `tools()`; tests.
- `src-tauri/app/src/lib.rs` — register `usage_set_auto_meter` in the invoke handler + the RootDispatcher match arm. The sweep loop is already correct — DO NOT change it.
- `src/ipc/usage.ts` — add `auto_meter_enabled` to the `UsageSnapshot` type + a `setAutoMeter(enabled)` IPC fn.
- `src/components/SettingsView.tsx` — add an "auto-brake" toggle in the usage section.
- `src/App.tsx` — pass `onSetAutoMeter={setAutoMeter}` to `SettingsView`.
- Tests: `usage.test.ts`, `SettingsView.test.tsx`, snapshot/api Rust tests, and one app-level sweep-decision test in `app/src/lib.rs`.

## Decisions

- **D-R2-1 — New command vs. extend `usage_set_budget`.** RECOMMENDED + CHOSEN: a dedicated `usage_set_auto_meter(enabled: bool)` command. Keeps each command single-responsibility (mirrors `usage_set_budget`), keeps the tool catalog entries clean, and avoids overloading budget semantics.
- **D-R2-2 — Default OFF.** CHOSEN (per item brief): migration already seeds `auto_meter_enabled=0`; we keep it. The toggle ships OFF so manual + reactive (rate-limit) braking stays the safe default; the capability is fully wired + tested.
- **D-R2-3 — Surface `auto_meter_enabled` on `UsageSnapshot`.** CHOSEN: the meter/Settings need to render the toggle's current state without a second round-trip. `compute_snapshot` already takes `&UsageConfig`, so it's a free field. Both the command and `usage_set_budget` return a fresh snapshot, so the toggle reflects immediately.
- **D-R2-4 — Topbar meter "auto-braked" reflection.** CHOSEN: no new meter field. When the sweep trips the brake, `Brake.set_on(AUTO_METER_REASON)` flips `is_braked()` → `snapshot.braked=true` → the existing `UsageMeter` braked styling + the topbar brake button's `(reason: auto-meter)` already render it. We add one app-level test that the sweep decision path sets the brake reason to `auto-meter`. No frontend meter change required beyond what already exists.
- **D-R2-5 — Sweep loop untouched.** CHOSEN: `app/src/lib.rs` lines ~752-780 already load config each tick, skip when disabled, compute `auto_on` by reason, and apply SetOn/Release/NoChange emitting `usage.changed`. It is correct. We only make the flag flippable + tested.

---

### Task 1: Add `auto_meter_enabled` to `UsageSnapshot`

**Files:**
- Modify: `src-tauri/usage_telemetry/src/snapshot.rs`
- Test: `src-tauri/usage_telemetry/src/snapshot.rs` (inline `#[cfg(test)]`)

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `snapshot.rs`:

```rust
    #[tokio::test]
    async fn snapshot_reflects_auto_meter_enabled_flag() {
        let pool = fresh_pool().await;
        let cc = CcUsageStore::new(pool.clone());
        let worker = WorkerUsageStore::new(pool.clone());
        let cfg = UsageConfig { auto_meter_enabled: true, ..UsageConfig::default() };
        let snap = compute_snapshot(&cc, &worker, &cfg, false, 1000).await.unwrap();
        assert!(snap.auto_meter_enabled);
        let off = UsageConfig { auto_meter_enabled: false, ..UsageConfig::default() };
        let snap_off = compute_snapshot(&cc, &worker, &off, false, 1000).await.unwrap();
        assert!(!snap_off.auto_meter_enabled);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test -p usage_telemetry snapshot_reflects_auto_meter_enabled_flag`
Expected: FAIL — compile error, no field `auto_meter_enabled` on `UsageSnapshot`.

- [ ] **Step 3: Add the field + populate it**

In the `UsageSnapshot` struct (after `braked: bool,`):

```rust
    /// Whether the reactive auto-meter brake is enabled (mirrors config; lets
    /// the UI render the Settings toggle + meter state). R2.
    pub auto_meter_enabled: bool,
```

In `compute_snapshot`'s returned `Ok(UsageSnapshot { ... })`, after `braked,`:

```rust
        auto_meter_enabled: cfg.auto_meter_enabled,
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd src-tauri && cargo test -p usage_telemetry snapshot_reflects_auto_meter_enabled_flag`
Expected: PASS. (The existing `snapshot_uses_cc_for_total_and_worker_for_breakdown` test still passes — it doesn't assert the new field.)

- [ ] **Step 5: Run the whole crate's tests**

Run: `cd src-tauri && cargo test -p usage_telemetry`
Expected: all PASS.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/usage_telemetry/src/snapshot.rs
git commit -m "feat(usage_telemetry): surface auto_meter_enabled on UsageSnapshot (R2 task 1)"
```

---

### Task 2: `usage_set_auto_meter` command + tools() entry

**Files:**
- Modify: `src-tauri/usage_telemetry/src/api.rs`
- Test: `src-tauri/usage_telemetry/src/api.rs` (inline `#[cfg(test)]`)

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `api.rs`:

```rust
    #[tokio::test]
    async fn set_auto_meter_persists_to_config() {
        let pool = fresh_pool().await;
        assert!(!load_config(&pool).await.auto_meter_enabled);
        set_auto_meter_inner(&pool, true).await.unwrap();
        assert!(load_config(&pool).await.auto_meter_enabled);
        set_auto_meter_inner(&pool, false).await.unwrap();
        assert!(!load_config(&pool).await.auto_meter_enabled);
    }

    #[test]
    fn tools_include_set_auto_meter() {
        let t = tools();
        assert!(t.iter().any(|s| s.name == "usage_set_auto_meter"));
    }
```

Note: the test targets a pure inner helper `set_auto_meter_inner(&pool, enabled)` so it's testable without Tauri `State`. The `#[tauri::command]` wrapper delegates to it (mirrors how `usage_set_budget`/`usage_snapshot` keep logic thin).

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test -p usage_telemetry set_auto_meter`
Expected: FAIL — `set_auto_meter_inner` not found / `usage_set_auto_meter` not in tools.

- [ ] **Step 3: Implement the helper + command + tools entry**

In `api.rs`, after `usage_set_budget`, add the inner helper and the command:

```rust
/// Write the auto-meter enable flag to the config row (R2). Pure DB write —
/// the brake STATE stays Runtime's; the root's sweep reads this each tick.
pub async fn set_auto_meter_inner(pool: &SqlitePool, enabled: bool) -> Result<(), String> {
    sqlx::query("UPDATE usage_config SET auto_meter_enabled = ? WHERE id = 1")
        .bind(if enabled { 1 } else { 0 })
        .execute(pool)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn usage_set_auto_meter(
    state: tauri::State<'_, UsageState>,
    enabled: bool,
) -> Result<UsageSnapshot, String> {
    set_auto_meter_inner(&state.pool, enabled).await?;
    usage_snapshot(state).await
}
```

In `tools()`, add a third `ToolSpec` after the `usage_set_budget` spec:

```rust
        ToolSpec {
            name: "usage_set_auto_meter".into(),
            description: "Enable or disable the reactive auto-meter brake (trips the system brake when the usage window crosses the threshold).".into(),
            input_schema: json!({ "type": "object", "properties": { "enabled": { "type": "boolean" } }, "required": ["enabled"] }),
            supplier_context: ctx.into(),
        },
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd src-tauri && cargo test -p usage_telemetry set_auto_meter`
Expected: PASS (both tests).

- [ ] **Step 5: Run the whole crate's tests**

Run: `cd src-tauri && cargo test -p usage_telemetry`
Expected: all PASS. The existing `tools_are_usage_telemetry_slug` still passes (new spec uses the same `ctx`).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/usage_telemetry/src/api.rs
git commit -m "feat(usage_telemetry): usage_set_auto_meter command + tools entry (R2 task 2)"
```

---

### Task 3: Wire the command into the app root (handler + dispatcher)

**Files:**
- Modify: `src-tauri/app/src/lib.rs`

- [ ] **Step 1: Add the dispatcher match arm**

In the `RootDispatcher` dispatch match in `app/src/lib.rs`, after the `"usage_snapshot" => { ... }` arm (around line 515), add:

```rust
            "usage_set_auto_meter" => {
                let enabled = args.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false);
                match usage_telemetry::api::set_auto_meter_inner(&self.usage.pool, enabled).await {
                    Ok(()) => { let _ = self.app.emit("usage.changed", ()); ok(serde_json::json!({ "auto_meter_enabled": enabled })) }
                    Err(e) => err(e),
                }
            }
```

(Match the existing arms' use of `args`/`ok`/`err`. Confirm the local arg accessor: the file uses `args.get(...)` directly for some arms and a `str_arg` helper for strings — `enabled` is a bool so read it via `args.get("enabled").and_then(|v| v.as_bool())`.)

- [ ] **Step 2: Register the command in the invoke handler**

In the `tauri::generate_handler![ ... ]` list, after `usage_telemetry::api::usage_set_budget,` add:

```rust
            usage_telemetry::api::usage_set_auto_meter,
```

- [ ] **Step 3: Build to verify wiring compiles**

Run: `cd src-tauri && cargo check -p app`
Expected: clean compile.

- [ ] **Step 4: Add an app-level test that the sweep decision trips/releases by reason**

The sweep loop itself is a `spawn`ed `loop` (not unit-testable directly), but its core — `auto_brake_decision` applied to `Brake` by reason — is. Add a test in the `#[cfg(test)]` block of `app/src/lib.rs` (alongside the existing `use runtime::brake::Brake;` tests) that exercises the exact apply logic the sweep uses:

```rust
    #[test]
    fn sweep_decision_trips_and_releases_brake_by_auto_meter_reason() {
        use runtime::brake::Brake;
        use usage_telemetry::brake_policy::{decide, BrakeDecision, AUTO_METER_REASON};

        let brake = Brake::new();
        // helper mirroring the root sweep's apply step
        fn apply(brake: &Brake, d: BrakeDecision) {
            match d {
                BrakeDecision::SetOn(reason) => brake.set_on(reason),
                BrakeDecision::Release => brake.set_off(),
                BrakeDecision::NoChange => {}
            }
        }

        // over threshold, not auto-on -> trips on with the auto-meter reason
        let auto_on = brake.state().reason.as_deref() == Some(AUTO_METER_REASON);
        apply(&brake, decide(0.96, auto_on, 0.95, 0.85));
        assert!(brake.is_on());
        assert_eq!(brake.state().reason.as_deref(), Some(AUTO_METER_REASON));

        // dropped below off threshold, auto-on -> releases
        let auto_on = brake.state().reason.as_deref() == Some(AUTO_METER_REASON);
        apply(&brake, decide(0.80, auto_on, 0.95, 0.85));
        assert!(!brake.is_on());
        assert_eq!(brake.state().reason, None);
    }

    #[test]
    fn sweep_never_releases_a_manual_brake() {
        use runtime::brake::Brake;
        use usage_telemetry::brake_policy::{decide, BrakeDecision, AUTO_METER_REASON};

        let brake = Brake::new();
        brake.set_on("rate-limit"); // reactive/manual reason, NOT auto-meter
        let auto_on = brake.state().reason.as_deref() == Some(AUTO_METER_REASON);
        // even with low pct, a non-auto brake must not be auto-released
        let d = decide(0.10, auto_on, 0.95, 0.85);
        assert_eq!(d, BrakeDecision::NoChange);
        assert!(brake.is_on());
        assert_eq!(brake.state().reason.as_deref(), Some("rate-limit"));
    }
```

- [ ] **Step 5: Run the app tests**

Run: `cd src-tauri && cargo test -p app sweep_`
Expected: both PASS.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/app/src/lib.rs
git commit -m "feat(app): wire usage_set_auto_meter command + sweep-decision brake tests (R2 task 3)"
```

---

### Task 4: Frontend IPC — `setAutoMeter` + snapshot field

**Files:**
- Modify: `src/ipc/usage.ts`
- Test: `src/ipc/usage.test.ts`

- [ ] **Step 1: Write the failing test**

Add to `usage.test.ts`:

```ts
  it("setAutoMeter passes the enabled flag", async () => {
    invokeMock.mockResolvedValueOnce({ auto_meter_enabled: true });
    await setAutoMeter(true);
    expect(invokeMock).toHaveBeenCalledWith("usage_set_auto_meter", { enabled: true });
  });
```

And update the import line at the top:

```ts
import { usageSnapshot, setBudget, setAutoMeter } from "./usage";
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bun vitest run src/ipc/usage.test.ts`
Expected: FAIL — `setAutoMeter` is not exported.

- [ ] **Step 3: Add the field + the IPC fn**

In `usage.ts`, add to the `UsageSnapshot` interface (after `braked: boolean;`):

```ts
  auto_meter_enabled: boolean;
```

And append the new fn:

```ts
export async function setAutoMeter(enabled: boolean): Promise<UsageSnapshot> {
  return await invoke<UsageSnapshot>("usage_set_auto_meter", { enabled });
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `bun vitest run src/ipc/usage.test.ts`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/ipc/usage.ts src/ipc/usage.test.ts
git commit -m "feat(ui): setAutoMeter IPC + auto_meter_enabled snapshot field (R2 task 4)"
```

---

### Task 5: Settings toggle for auto-brake

**Files:**
- Modify: `src/components/SettingsView.tsx`
- Test: `src/components/SettingsView.test.tsx`

- [ ] **Step 1: Write the failing test**

Add to `SettingsView.test.tsx`. First extend the `snap` fixture object with `auto_meter_enabled: false,`. Then add:

```ts
  it("renders the auto-brake toggle reflecting snapshot state", () => {
    render(<SettingsView usage={{ ...snap, auto_meter_enabled: false }} onSetBudget={vi.fn().mockResolvedValue(snap)} onSetAutoMeter={vi.fn().mockResolvedValue(snap)} />);
    const toggle = screen.getByRole("checkbox", { name: /auto-brake/i });
    expect(toggle).not.toBeChecked();
  });

  it("calls onSetAutoMeter when the toggle is flipped", async () => {
    const onSetAutoMeter = vi.fn().mockResolvedValue({ ...snap, auto_meter_enabled: true });
    render(<SettingsView usage={{ ...snap, auto_meter_enabled: false }} onSetBudget={vi.fn().mockResolvedValue(snap)} onSetAutoMeter={onSetAutoMeter} />);
    fireEvent.click(screen.getByRole("checkbox", { name: /auto-brake/i }));
    await waitFor(() => expect(onSetAutoMeter).toHaveBeenCalledWith(true));
  });
```

Also update the two existing `render(<SettingsView ... />)` calls in the other tests to pass `onSetAutoMeter={vi.fn().mockResolvedValue(snap)}` (required prop).

- [ ] **Step 2: Run test to verify it fails**

Run: `bun vitest run src/components/SettingsView.test.tsx`
Expected: FAIL — `onSetAutoMeter` not in props / no checkbox.

- [ ] **Step 3: Add the prop + toggle**

In `SettingsView.tsx`, extend props:

```tsx
export interface SettingsViewProps {
  usage: UsageSnapshot | null;
  onSetBudget: (budget: number) => Promise<UsageSnapshot>;
  onSetAutoMeter: (enabled: boolean) => Promise<UsageSnapshot>;
}
```

Update the signature:

```tsx
export function SettingsView({ usage, onSetBudget, onSetAutoMeter }: SettingsViewProps) {
```

Add local state near the other `useState` calls:

```tsx
  const [autoMeter, setAutoMeter] = useState<boolean>(usage?.auto_meter_enabled ?? false);
  const [autoSaving, setAutoSaving] = useState(false);
```

Add a handler near `saveBudget`:

```tsx
  async function toggleAutoMeter(next: boolean) {
    setAutoMeter(next);
    setAutoSaving(true);
    try {
      const s = await onSetAutoMeter(next);
      setAutoMeter(s.auto_meter_enabled);
    } finally {
      setAutoSaving(false);
    }
  }
```

In the `usage` section JSX, after the budget block (before the closing `</div>` of that section), add the toggle. Use a native checkbox so the test's `getByRole("checkbox")` resolves and the control stays accessible + consistent with the controlled-input pattern:

```tsx
        <label style={{ ...label, display: "flex", alignItems: "center", gap: 8, marginTop: 16, marginBottom: 0, cursor: "pointer" }}>
          <input
            type="checkbox"
            checked={autoMeter}
            disabled={autoSaving}
            onChange={(e) => toggleAutoMeter(e.target.checked)}
            aria-label="auto-brake"
            style={{ accentColor: "var(--accent)", cursor: "pointer" }}
          />
          auto-brake when the window crosses the threshold
        </label>
        <div style={{ marginTop: 6, fontSize: 11, color: "var(--text-3)" }}>
          off by default — manual + reactive (rate-limit) braking stays on either way.
        </div>
```

- [ ] **Step 4: Run test to verify it passes**

Run: `bun vitest run src/components/SettingsView.test.tsx`
Expected: PASS (all tests in the file).

- [ ] **Step 5: Commit**

```bash
git add src/components/SettingsView.tsx src/components/SettingsView.test.tsx
git commit -m "feat(ui): Settings auto-brake toggle (R2 task 5)"
```

---

### Task 6: Wire the toggle into App

**Files:**
- Modify: `src/App.tsx`

- [ ] **Step 1: Update the import**

Change the usage IPC import line:

```tsx
import { setBudget, setAutoMeter } from "./ipc/usage";
```

- [ ] **Step 2: Pass the prop to SettingsView**

In the `view === "settings"` branch:

```tsx
          <SettingsView usage={usage} onSetBudget={setBudget} onSetAutoMeter={setAutoMeter} />
```

- [ ] **Step 3: Verify App tests still pass**

Run: `bun vitest run src/App.test.tsx`
Expected: PASS (App test doesn't render Settings or asserts unaffected).

- [ ] **Step 4: Commit**

```bash
git add src/App.tsx
git commit -m "feat(ui): pass setAutoMeter into SettingsView (R2 task 6)"
```

---

### Task 7: Full verification

**Files:** none (verification only).

- [ ] **Step 1: Rust tests**

Run: `cd src-tauri && cargo test --workspace`
Expected: all PASS.

- [ ] **Step 2: cargo check + clippy**

Run: `cd src-tauri && cargo check --workspace && cargo clippy --workspace -- -D warnings`
Expected: clean (no warnings).

- [ ] **Step 3: Frontend tests**

Run: `bun vitest run`
Expected: all PASS.

- [ ] **Step 4: Frontend build**

Run: `bun run build`
Expected: success (tsc + vite build).

- [ ] **Step 5: Commit any incidental fixes (if needed)**

```bash
git add -A
git commit -m "chore(R2): verification fixes"  # only if anything changed
```

---

## Self-Review

**Spec coverage:**
- "make auto-meter user-controllable (Settings toggle that flips `auto_meter_enabled` via a usage-telemetry command)" → Task 2 (command) + Task 5 (toggle) + Task 6 (wire).
- "ensure the sweep actually trips/releases the brake by AUTO_METER_REASON" → sweep already does (D-R2-5); Task 3 step 4 proves the decision→brake apply path by reason + the manual-brake-not-released invariant.
- "ensure the topbar meter reflects the auto-braked state" → D-R2-4: `braked` already flows to `UsageMeter` + topbar reason; Task 1 adds `auto_meter_enabled` for the toggle's own state. No further meter change needed (the impeccable pass polishes visuals later).
- "ship the toggle defaulting OFF, capability fully wired + tested" → D-R2-2 + all tasks.

**Placeholder scan:** none — every code step is concrete.

**Type consistency:** `set_auto_meter_inner(&SqlitePool, bool) -> Result<(), String>` used identically in api.rs (Task 2) and app dispatcher (Task 3). `setAutoMeter(boolean) -> Promise<UsageSnapshot>` used identically in usage.ts (Task 4), test (Task 4), SettingsView prop (Task 5), App (Task 6). `auto_meter_enabled` field name identical across Rust struct, TS interface, and migration column.
