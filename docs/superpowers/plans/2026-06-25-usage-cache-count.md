# Usage Meter: Count All Tokens (incl. Cache) + Recalibrate Budget — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Strict TDD: write the failing test FIRST, watch it fail, then make it pass. Small commits per task.

**Goal:** Fix **LF34**. The rolling-window usage meter's total counts only `input_tokens + output_tokens`, ignoring cache tokens — but `cache_read` dominates real throughput (66M vs 347k input+output in the live 5h window). The meter therefore reads ~13% while claude.ai shows ~35%. Decision: count **ALL** tokens (`input + output + cache_creation + cache_read`) as the window basis, **recalibrate** the default budget to the all-tokens scale (~190M), and **surface** the budget as an operator-tunable field (the budget setter/IPC/field already exist — this is mostly a recalibration + verification, not new plumbing).

**Architecture:** Three small, ordered changes:
1. `CcUsageStore::window_tokens` SQL sum gains the two cache columns (already ingested, just unused). `recent_tokens` delegates to it, so burn-rate follows automatically. `oldest_in_window` is untouched.
2. The default budget moves to the all-tokens scale in **three** coupled places: `UsageConfig::default()` (Rust), the `005_usage.sql` column DEFAULT + seed (for fresh installs), and a NEW idempotent migration **015** that bumps existing installs still sitting on the old `2600000` default — without clobbering a user-tuned value.
3. The Settings → Usage budget field, `usage_set_budget` command, and `setBudget` IPC ALREADY EXIST (confirmed below). No new command/IPC/field is required; we only update the default-value references and copy.

**Tech Stack:** Rust (Cargo workspace under `src-tauri/`), Tauri 2, sqlx + SQLite, tokio; React/TS frontend with Vitest. macOS. No new crates.

---

## Background facts confirmed by reading the code

Load-bearing; do not re-derive. Read the cited lines before editing.

- **`window_tokens`** — `src-tauri/usage_telemetry/src/cc_log.rs:58-67`: `SELECT COALESCE(SUM(input_tokens + output_tokens), 0) FROM cc_usage_log WHERE ts > ?`. The doc-comment (`:56-57`) says "SUM(input+output)". The `cc_usage_log` table already has `cache_creation` + `cache_read` columns, populated by `insert`/`ingest` (`:26-42`) — they are simply absent from the sum.
- **`recent_tokens`** — `cc_log.rs:71-73`: `self.window_tokens(now_ts - secs)`. The burn-rate numerator. It inherits the fix automatically — DO NOT change its body.
- **`oldest_in_window`** — `cc_log.rs:77-85`: `MIN(ts)`, basis-independent. UNAFFECTED.
- **Existing cache fixture** — `src-tauri/usage_telemetry/src/fixtures/transcript-sample.jsonl` (3 lines, `msg_aaa` appears twice and is deduped): `msg_aaa` = input 1200, output 300, cache_creation 40, cache_read 10; `msg_bbb` = input 500, output 120, cache 0/0. So:
  - **input+output only** (old): 1500 + 620 = **2120** (the value asserted today at `cc_log.rs:128` and `snapshot.rs:152`).
  - **all tokens** (new): (1200+300+40+10) + (500+120+0+0) = 1550 + 620 = **2170**. The cache delta is **50** tokens, entirely from `msg_aaa`.
- **`UsageConfig::default()`** — `src-tauri/usage_telemetry/src/snapshot.rs:32-42`: `window_budget: 2_600_000`, `window_secs: 18_000`, `brake_on_pct: 0.95`, `brake_off_pct: 0.85`, `auto_meter_enabled: false`.
- **`compute_snapshot`** — `snapshot.rs:76-112`: `window_total = cc.window_tokens(since) + worker.api_window_tokens(since)` (the worker term is zero in v1); `pct = window_pct(window_total, cfg.window_budget)`; `recent = cc.recent_tokens(now, 60)`. The window/pct LOGIC is correct and UNCHANGED — only the numeric basis (cache included) and denominator (recalibrated budget) shift.
- **`load_config`** — `src-tauri/usage_telemetry/src/api.rs:56-75`: reads the single `usage_config` row; falls back to `UsageConfig::default()` only when the row is ABSENT. In practice the row always exists (seeded by 005), so the LOADED budget comes from the DB column, NOT the Rust default.
- **Budget setter ALREADY EXISTS** — `api.rs:86-100`: `#[tauri::command] usage_set_budget(state, budget: i64)` validates `budget > 0`, runs `UPDATE usage_config SET window_budget = ? WHERE id = 1`, returns a fresh snapshot. Arg struct `args::SetBudgetArgs { budget: i64 }` (`api.rs:29-32`) and the `tools()` entry (`api.rs:140-145`) exist. **No new command is needed.**
- **IPC ALREADY EXISTS** — `src/ipc/usage.ts:29-31`: `export async function setBudget(budget): Promise<UsageSnapshot>` → `invoke("usage_set_budget", { budget })`. **No new IPC is needed.**
- **Settings field ALREADY EXISTS** — `src/components/SettingsView.tsx`: Usage section header (`:312`), `window budget (tokens / 5h)` label + numeric input (`:313-316`), `save budget` button calling `saveBudget()` → `onSetBudget(n)` (`:317`, `:251-260`), and a "currently X of Y used" readout (`:319-323`). `budgetInput` initial state falls back to `2_600_000` when `usage` is null (`:223`). **Only the fallback literal + copy need updating.**
- **Migration mechanism** — `src-tauri/app/src/lib.rs:40-72`: `run_migrations` applies a const `MIGRATIONS: &[(i64, &str)]` array (currently up to `(14, …)` at `:55`) gated on `PRAGMA user_version`, each via `sqlx::raw_sql`. New migrations are added as new array entries; **migrations 001–005 are never edited for production-applied schema changes**, BUT the `005_usage.sql` column DEFAULT + seed still matter for the in-test `fresh_pool` helpers (which run only 005) and for documentation accuracy. We will update 005's default/seed (cosmetic + test-pool truth) AND add a guarded 015 (production data fix).
- **Test pools run only 005** — `cc_log.rs:107-111`, `api.rs:160-164`, `snapshot.rs:133-137` build `fresh_pool` via `include_str!("../../app/migrations/005_usage.sql")`. They do NOT run 015. So any test asserting a *seeded* budget reads 005's value; tests asserting the Rust default read `UsageConfig::default()`.
- **`load_config_returns_seeded_defaults`** — `api.rs:166-172`: asserts `cfg.window_budget == 2_600_000` from a `fresh_pool` (i.e. 005's seed). This must move in lockstep with 005's seed value.
- **`2_600_000` literal occurrences** (must all be reconciled): `snapshot.rs:35` (Rust default), `api.rs:169` (seeded-default test), `app/migrations/005_usage.sql:39` (SQL column default), `src/components/SettingsView.tsx:223` (frontend fallback), plus DISPLAY-only fixtures in `src/components/SettingsView.test.tsx:9`, `src/components/UsageMeter.test.tsx:7`, `src/components/Topbar.test.tsx:7` (these are arbitrary mock snapshots — update for realism only, not correctness). `window.rs:95/119/122/124` use `2_600_000` as arithmetic test inputs for pure window math — those are basis-agnostic and may be LEFT AS-IS.
- **`by_team` breakdown** reads `worker_usage_log` via `worker.team_breakdown` (`snapshot.rs:90-95`) — a SEPARATE basis (our own worker invocations, zero in v1). LEFT UNCHANGED; see "Known inconsistency (out of scope)" below.
- **DO NOT TOUCH** `src-tauri/usage_telemetry/src/ingest.rs` or `transcript.rs::parse_transcript` — cache fields are already parsed and ingested.

### The calibration (document, don't treat as exact)

Live observation: all-tokens throughput in the 5h window ≈ **67.2M** corresponds to ≈ **35%** on claude.ai ⟹ implied full-window budget ≈ 67.2M / 0.35 ≈ **192M**. We round to **190_000_000** as the new default. This is a **tunable estimate, not an exact mirror** of claude.ai: Anthropic's exact metering formula (which token classes count, and at what weight) and the real per-plan rate-limit ceiling are NOT queryable (**G6**). The operator tunes the budget in Settings → Usage to pull the displayed % toward what claude.ai reports for their plan. Record this rationale in code comments and in the migration header.

### Known inconsistency (out of scope, note only)

After this change the **window total** (`cc_usage_log`) counts all tokens incl. cache, while the **per-team `by_team` breakdown** (`worker_usage_log`) is a different log and is NOT part of the window total in v1 (worker rows are CLI-tail, `api_window_tokens` is zero). The two are already different bases by design (D3); we are not unifying them here. No action — flagged for awareness.

---

## Tasks

### Task 1 — Count all tokens in `window_tokens` (and therefore burn) — RUST/TDD

- [ ] **Read** `src-tauri/usage_telemetry/src/cc_log.rs` (esp. `:56-73` and the test module `:98-166`).
- [ ] **Write the failing test FIRST.** Add a new test to the `#[cfg(test)] mod tests` block in `cc_log.rs` that proves cache is counted. Use the existing `SAMPLE` fixture (cache delta = 50). Name it `window_tokens_includes_cache`:

  ```rust
  #[tokio::test]
  async fn window_tokens_includes_cache() {
      let store = CcUsageStore::new(fresh_pool().await);
      store.ingest(&parse_transcript(SAMPLE)).await.unwrap();
      // msg_aaa: 1200+300+40+10 = 1550 ; msg_bbb: 500+120+0+0 = 620 ; total 2170
      // (the +50 over the old 2120 is the cache_creation+cache_read on msg_aaa)
      assert_eq!(store.window_tokens(0).await.unwrap(), 2170);
  }
  ```

- [ ] **Update the now-stale existing test.** Rename/retarget `window_tokens_sums_input_plus_output_in_window` (`cc_log.rs:123-129`): change its assertion from `2120` to `2170` and its comment to reflect all-tokens. (Or delete it in favour of the new test — but keeping a renamed `window_tokens_sums_all_token_classes_in_window` documents intent.) Run the suite and CONFIRM both fail with the current implementation (expected `2170`, got `2120`).
- [ ] **Make it pass.** Edit `window_tokens` (`cc_log.rs:58-67`) SQL to:

  ```sql
  SELECT COALESCE(SUM(input_tokens + output_tokens + cache_creation + cache_read), 0)
  FROM cc_usage_log WHERE ts > ?
  ```

- [ ] **Update the doc-comments.** `cc_log.rs:56-57` (`window_tokens`) and `:69-70` (`recent_tokens`): replace "SUM(input+output)" / "(input+output)" with wording like: *"all tokens incl. cache (input+output+cache_creation+cache_read) — the throughput basis the rate-limit window meters; cache_read dominates real usage."*
- [ ] **Verify `prune` test still holds.** `prune_removes_rows_before_cutoff_keeps_in_window` (`cc_log.rs:142-165`) inserts an `old` row with `input_tokens:10, output/cache:0` and asserts `window_tokens(0) > 0` after pruning — still true (SAMPLE rows survive). No change needed; just confirm green.
- [ ] **Run** `cd src-tauri && cargo test -p usage_telemetry cc_log`. All green.
- [ ] **Commit:** `usage: count cache tokens in window_tokens basis (LF34)`.

### Task 2 — Reconcile the snapshot test to the new basis — RUST/TDD

- [ ] **Read** `src-tauri/usage_telemetry/src/snapshot.rs:139-158` (`snapshot_uses_cc_for_total_and_worker_for_breakdown`). It asserts `window_total == 2120` and `window_pct ≈ 0.5` with `window_budget: 4240`. With the cache fix, cc now contributes **2170**.
- [ ] **Update the test (failing → passing):**
  - Change the cc total comment/assertion: `assert_eq!(snap.window_total, 2170);` (cc only; the worker row's 120 is breakdown-only, not in the total — D3).
  - To keep the clean 0.5 pct, set `let cfg = UsageConfig { window_budget: 4340, ..UsageConfig::default() };` (2170 * 2 = 4340) and keep `assert!((snap.window_pct - 0.5).abs() < 1e-9);`. Update the inline comment `// 2170 / 4340`.
  - Leave the `by_team` (`research` → 120) and `tokens_by_task` (`T-1` → 120) assertions unchanged — worker basis is untouched.
- [ ] **Run** `cd src-tauri && cargo test -p usage_telemetry snapshot`. Green. Confirm it FAILED first (old `2120`/`4240`).
- [ ] **Commit:** `usage: reconcile snapshot test to all-tokens cc basis`.

### Task 3 — Recalibrate the default budget (Rust default + 005 seed/default) — RUST/TDD

- [ ] **Read** `snapshot.rs:32-42`, `api.rs:166-172`, `app/migrations/005_usage.sql:34-45`.
- [ ] **Write/adjust the failing config-default test FIRST.** In `api.rs`, update `load_config_returns_seeded_defaults` (`:166-172`):
  - It currently asserts the *seeded* value via `fresh_pool` (which runs 005). Since we are ALSO updating 005's seed, keep it reading the seeded value but assert the new number:

    ```rust
    #[tokio::test]
    async fn load_config_returns_seeded_defaults() {
        let cfg = load_config(&fresh_pool().await).await;
        assert_eq!(cfg.window_budget, 190_000_000); // all-tokens basis (LF34); tunable estimate, not an exact claude.ai mirror (G6)
        assert_eq!(cfg.window_secs, 18_000);
        assert!(!cfg.auto_meter_enabled);
    }
    ```

  - ALSO add a focused unit test that pins the Rust default independently of SQL (so the two can't silently drift):

    ```rust
    #[test]
    fn default_window_budget_is_all_tokens_calibration() {
        assert_eq!(UsageConfig::default().window_budget, 190_000_000);
    }
    ```

    (`UsageConfig` is imported via `crate::snapshot::UsageConfig` — already in scope at `api.rs:7`.)
- [ ] **Run** `cd src-tauri && cargo test -p usage_telemetry api` — CONFIRM both fail (`2_600_000` ≠ `190_000_000`).
- [ ] **Make them pass:**
  - `snapshot.rs:35`: `window_budget: 190_000_000,` and add a comment: `// all-tokens basis incl. cache (LF34): ~67.2M live throughput ≈ 35% ⟹ ~192M; rounded. Tunable estimate, not an exact claude.ai mirror (G6).`
  - `app/migrations/005_usage.sql:39`: change the column to `window_budget INTEGER NOT NULL DEFAULT 190000000,` and update the comment block at `:34-36` to note the all-tokens calibration (replace "Pro-estimate default" wording). The `INSERT OR IGNORE … VALUES (1)` (`:45`) picks up the new column default for fresh test pools — so `load_config_returns_seeded_defaults` now reads 190M.
- [ ] **Run** `cd src-tauri && cargo test -p usage_telemetry`. All green.
- [ ] **Commit:** `usage: recalibrate default window budget to all-tokens scale (~190M)`.

### Task 4 — Migration 015: bump existing installs off the old default (production data fix) — RUST/TDD

> Editing 005's seed only affects FRESH databases. Existing users already have a `usage_config` row at `2600000`. A guarded migration bumps them WITHOUT clobbering anyone who tuned their budget.

- [ ] **Read** `app/src/lib.rs:40-72` (`run_migrations` + the `MIGRATIONS` array, last entry `(14, …)`).
- [ ] **Write the failing migration test FIRST.** Add a test (in `app/src/lib.rs` test module, or wherever migration tests live — search `run_migrations` usage; if no existing migration test harness, add a `#[cfg(test)] mod migration_tests` that builds an in-memory pool, runs `run_migrations`, and asserts). Test the GUARD behaviour:

  ```rust
  #[tokio::test]
  async fn migration_015_bumps_legacy_budget_but_preserves_tuned() {
      // legacy install: row exists at the old default
      let pool = sqlx::sqlite::SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
      sqlx::raw_sql(include_str!("../migrations/005_usage.sql")).execute(&pool).await.unwrap();
      // simulate an OLD install whose 005 had seeded 2_600_000 (override the fresh 190M):
      sqlx::query("UPDATE usage_config SET window_budget = 2600000 WHERE id = 1").execute(&pool).await.unwrap();
      sqlx::raw_sql(include_str!("../migrations/015_usage_budget_recalibrate.sql")).execute(&pool).await.unwrap();
      let b: i64 = sqlx::query_scalar("SELECT window_budget FROM usage_config WHERE id = 1").fetch_one(&pool).await.unwrap();
      assert_eq!(b, 190_000_000); // legacy default bumped

      // a tuned install is NOT clobbered
      let pool2 = sqlx::sqlite::SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
      sqlx::raw_sql(include_str!("../migrations/005_usage.sql")).execute(&pool2).await.unwrap();
      sqlx::query("UPDATE usage_config SET window_budget = 5000000 WHERE id = 1").execute(&pool2).await.unwrap();
      sqlx::raw_sql(include_str!("../migrations/015_usage_budget_recalibrate.sql")).execute(&pool2).await.unwrap();
      let b2: i64 = sqlx::query_scalar("SELECT window_budget FROM usage_config WHERE id = 1").fetch_one(&pool2).await.unwrap();
      assert_eq!(b2, 5_000_000); // user-tuned value preserved
  }
  ```

  (`include_str!` of a not-yet-created file makes this fail to COMPILE first — that is the failing state. Confirm the compile error, then create the file.)
- [ ] **Create** `src-tauri/app/migrations/015_usage_budget_recalibrate.sql`:

  ```sql
  -- 015 — LF34: recalibrate the window budget for the all-tokens basis (input +
  -- output + cache_creation + cache_read). cache_read dominates real throughput,
  -- so the old input+output-only budget (2_600_000) read far too low (~13% vs
  -- claude.ai's ~35%). New default 190_000_000 derived from live throughput
  -- (~67.2M ≈ 35% ⟹ ~192M, rounded). Tunable estimate, NOT an exact claude.ai
  -- mirror (G6). Only bump rows still on the OLD default — never clobber a
  -- budget the operator tuned in Settings.
  UPDATE usage_config SET window_budget = 190000000
    WHERE id = 1 AND window_budget = 2600000;
  ```

- [ ] **Register it.** In `app/src/lib.rs`, add to the `MIGRATIONS` array after `(14, …)`:

  ```rust
  (15, include_str!("../migrations/015_usage_budget_recalibrate.sql")),
  ```

- [ ] **Run** `cd src-tauri && cargo test -p app` (or the crate that owns `lib.rs`). Green.
- [ ] **Commit:** `usage: migration 015 bumps legacy window budget, preserves tuned values`.

### Task 5 — Frontend: update default-budget fallback + copy — TS/TDD

> The Settings field, IPC, and command already exist (confirmed above). This task only updates the hard-coded fallback literal and clarifying copy, plus mock-snapshot realism.

- [ ] **Read** `src/components/SettingsView.tsx:213-333`, `src/components/SettingsView.test.tsx:1-70`, `src/ipc/usage.ts`.
- [ ] **Write/adjust the failing Settings test FIRST.** In `SettingsView.test.tsx`:
  - Add a test that when `usage` is `null`, the budget input pre-fills the new default (the fallback at `SettingsView.tsx:223`):

    ```tsx
    it("pre-fills the recalibrated default budget when usage is null", () => {
      render(<SettingsView {...baseProps({ usage: null })} />);
      expect((screen.getByLabelText(/window budget/i) as HTMLInputElement).value).toBe("190000000");
    });
    ```

  - Keep the existing `calls onSetBudget with the entered number` test (`:52-58`) — it types `5000000` and asserts `onSetBudget(5_000_000)`; unaffected, leave as-is (proves the tunable path still works).
- [ ] **Run** `npx vitest run src/components/SettingsView.test.tsx` — CONFIRM the new test fails (`2600000` ≠ `190000000`).
- [ ] **Make it pass:** `SettingsView.tsx:223`: `useState(String(usage?.window_budget ?? 190_000_000))`. Update the label/help copy near `:313` if desired to mention the budget is a tunable estimate calibrated toward claude.ai's percentage (e.g. a small hint line: *"tune this so the % matches claude.ai for your plan; cache tokens are counted"*). Keep copy lowercase to match the section's style.
- [ ] **Update display-only mock fixtures for realism** (not correctness): bump `window_budget` in `SettingsView.test.tsx:9`, `UsageMeter.test.tsx:7`, `Topbar.test.tsx:7` from `2_600_000` to `190_000_000` and adjust the paired `window_total`/`window_pct` so they remain self-consistent (e.g. in `UsageMeter`/`Topbar`, `window_total: 66_500_000, window_pct: 0.35` reads as a realistic ~35%). Run those suites and confirm any pct/label assertions still hold (read each test's assertions before changing numbers).
- [ ] **Run** `npx vitest run` and `npx tsc --noEmit`. Green.
- [ ] **Commit:** `ui(settings): recalibrate default budget fallback + copy (LF34)`.

### Task 6 — Full verification gate — runs ALL gates

- [ ] `cd src-tauri && cargo test`
- [ ] `cd src-tauri && cargo clippy --all-targets -- -D warnings`
- [ ] `cd src-tauri && cargo build`
- [ ] `npx vitest run`
- [ ] `npx tsc --noEmit`
- [ ] If any gate fails, STOP and fix before claiming completion (superpowers:verification-before-completion: evidence before assertions).
- [ ] **Final commit** (if any cleanup): `usage: verify all-tokens basis + recalibrated budget (LF34)`.

---

## Sequencing & dependencies

1 → 2 (snapshot test depends on the new 2170 basis from Task 1) → 3 (default budget) → 4 (migration, depends on the new 190M constant from Task 3) → 5 (frontend, depends on the agreed 190M default) → 6 (gate). Tasks 1–2 are pure Rust; 3–4 touch SQL + Rust; 5 is TS-only; each is independently committable and green.

## Risks / watch-outs

- **Three coupled default-budget sites.** `UsageConfig::default()` (Rust), `005_usage.sql` column DEFAULT+seed, and the TS fallback must all read **190_000_000**. The `default_window_budget_is_all_tokens_calibration` unit test (Task 3) and the `load_config_returns_seeded_defaults` test pin the two Rust-visible paths so they can't drift silently.
- **Migration only helps existing users via 015.** Editing 005's seed alone does nothing for already-migrated databases (their `usage_config` row predates the edit). The guarded `WHERE … window_budget = 2600000` in 015 is what actually moves them — and the guard is what protects an operator who already tuned the value. Verify both branches with the Task 4 test.
- **Test pools run only 005, not 015.** `fresh_pool` helpers in `cc_log.rs`/`api.rs`/`snapshot.rs` include only `005_usage.sql`. That's why `load_config_returns_seeded_defaults` reads 005's seed — so 005's seed MUST be updated (Task 3), not just 015. Do not assume the test exercises 015.
- **Burn-rate / brake thresholds shift implicitly.** `recent_tokens` now includes cache, so `burn_per_min` and `est_brake_at` rise on the same basis as `window_total`/budget — proportions stay consistent (both numerator and denominator rescale). No threshold (`brake_on_pct`/`brake_off_pct`) needs changing. Sanity-check `window.rs` tests still pass (they use literal arithmetic inputs, basis-agnostic).
- **Calibration is an estimate, not a guarantee (G6).** 190M is derived from one live observation. Document it as tunable; the operator adjusts via the existing Settings field. Do not over-claim parity with claude.ai in copy or comments.
- **`by_team` basis inconsistency (out of scope).** The window total now counts cache; the per-team breakdown (`worker_usage_log`) does not feed the total in v1. Already divergent by design (D3); flagged, not fixed.
- **Don't touch the ingester or parser.** `ingest.rs` and `transcript.rs::parse_transcript` already persist cache fields; modifying them is out of scope and risks re-ingestion/dedup regressions.
- **`window_secs` is incidentally seeded in 005 as `18000` and as `18_000` in Rust** — leave untouched; only `window_budget` changes.
