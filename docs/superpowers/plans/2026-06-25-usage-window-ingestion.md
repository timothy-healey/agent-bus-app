# Usage Window Ingestion (cc_usage_log) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Strict TDD: write the failing test FIRST, watch it fail, then make it pass. Small commits per task.

**Goal:** Fix LF28 (the usage meter headline stuck at 0%). `cc_usage_log` is never populated in production — `.ingest()` is only called in tests. Wire transcript ingestion so the rolling-window headline reflects all real Claude Code activity. Implements Approach A from the spec: poll on the existing 15s auto-meter sweep, mtime-gated, full re-parse of changed files; dedup via the table's `UNIQUE(message_id)`.

**Architecture:** A new pure-IO unit `TranscriptIngestor { root: PathBuf, cc: Arc<CcUsageStore> }` in `usage_telemetry/src/ingest.rs` walks `~/.claude/projects/<project-dir>/*.jsonl` one level deep, mtime-gates files, reads + `transcript::parse_transcript` + `CcUsageStore::ingest`. `CcUsageStore::prune` bounds table growth. The composition root resolves the scan root from `$HOME`, builds an `Arc<TranscriptIngestor>`, runs a boot backfill, and restructures the 15s sweep loop so it ALWAYS ingests + prunes + conditionally emits `USAGE_CHANGED`, with only the brake DECISION gated on `auto_meter_enabled`.

**Tech Stack:** Rust (Cargo workspace under `src-tauri/`), Tauri 2 (event names contain NO dots — reuse the existing `usage-changed` constant), sqlx + SQLite, tokio. macOS. No new crates: home via `std::env::var("HOME")` (matches existing app convention at `lib.rs:721`/`:869`); logging via `eprintln!` (usage_telemetry has no `log`/`tracing` dep); tests use `std::env::temp_dir()` + a unique subdir and `std::fs::File::set_modified` (stable since rustc 1.75; toolchain is 1.95) for the mtime gate — no `tempfile`/`filetime` dep.

---

## Background facts confirmed by reading the code

Load-bearing; do not re-derive, but read the cited code before editing.

- **`CcUsageStore`** — `usage_telemetry/src/cc_log.rs:15`: `pub struct CcUsageStore { pub(crate) pool: SqlitePool }`. `pool` is `pub(crate)` — `prune` lives in this same file so it can use `&self.pool`. Methods: `ingest(&self, &[CcUsageRecord]) -> Result<u64, CcUsageError>` (cc_log.rs:46, returns newly-inserted count, idempotent via `INSERT OR IGNORE` on `UNIQUE(message_id)`), `window_tokens(&self, since_ts: i64) -> Result<u64, _>` (cc_log.rs:58). `CcUsageError` (cc_log.rs:10) is `#[from] sqlx::Error`.
- **Parser** — `usage_telemetry/src/transcript.rs`: `parse_transcript(blob: &str) -> Vec<CcUsageRecord>` (transcript.rs:55) and `parse_transcript_line` (transcript.rs:26). Already format-compatible: reads `message.id`, `message.usage.{input_tokens,output_tokens,cache_creation_input_tokens,cache_read_input_tokens}`, top-level RFC3339 `timestamp`. Lenient — malformed/partial trailing lines yield `None` and are skipped. DO NOT MODIFY.
- **`compute_snapshot`** — `usage_telemetry/src/snapshot.rs:76`: window total = `cc.window_tokens(since) + worker.api_window_tokens(since)` (the latter zero in v1). DO NOT MODIFY — it is already correct once `cc_usage_log` fills.
- **`UsageConfig`** — `usage_telemetry/src/snapshot.rs:24`: has `window_secs: i64` (default 18_000 = 5h) and `auto_meter_enabled: bool` (default false).
- **`load_config`** — `usage_telemetry/src/api.rs:56`: `pub async fn load_config(pool: &SqlitePool) -> UsageConfig`.
- **`now_unix`** — TWO copies: `app/src/lib.rs:1103` (file-private, used by the sweep) and `usage_telemetry/src/api.rs:51` (private). Use the app one in `lib.rs`.
- **`USAGE_CHANGED`** — `app/src/events.rs:14`: `pub const USAGE_CHANGED: &str = "usage-changed";` (no dots). Emitted in the sweep via `handle.emit(crate::events::USAGE_CHANGED, ())`.
- **The sweep loop** — `app/src/lib.rs:1455-1480`. Spawned in `setup` AFTER boot activation. Captures clones of `cc_store` (`cc`), `worker_usage` (`worker`), `brake`, `pool`, `handle`. Currently: `sleep(15s)` → `load_config` → `if !cfg.auto_meter_enabled { continue; }` → `compute_snapshot` → `auto_brake_decision` match. The early `continue` is what must move.
- **Store construction** — `app/src/lib.rs:1329`: `let cc_store = Arc::new(CcUsageStore::new(pool.clone()));` (already `Arc`). `worker_usage` at `:1330`. Both are in scope where the ingestor must be built and where the sweep is spawned.
- **`lib.rs` exports** — `usage_telemetry/src/lib.rs:9-16` declares modules. Add `pub mod ingest;`.
- **No `tempfile`/`filetime`/`tracing` crate** anywhere in the workspace (`src-tauri/Cargo.toml` `[workspace.dependencies]`). Tests and logging must avoid them.
- **Home convention** — app already does `std::env::var("HOME").unwrap_or_default()` (lib.rs:721) and `.unwrap_or(...)` (lib.rs:869). Match it; do not add `dirs`.

---

## Task 1 — `CcUsageStore::prune` (cc_log.rs)

- [ ] **Failing test first.** In `usage_telemetry/src/cc_log.rs`, in the existing `#[cfg(test)] mod tests`, add:

```rust
    #[tokio::test]
    async fn prune_removes_rows_before_cutoff_keeps_in_window() {
        let store = CcUsageStore::new(fresh_pool().await);
        store.ingest(&parse_transcript(SAMPLE)).await.unwrap(); // ts ~ 1.7e9 (2023+)
        // a row older than the cutoff and a row newer
        store
            .insert(&CcUsageRecord {
                message_id: "old".into(),
                ts: 1_000,
                model: None,
                input_tokens: 10,
                output_tokens: 0,
                cache_creation: 0,
                cache_read: 0,
            })
            .await
            .unwrap();
        let removed = store.prune(2_000).await.unwrap();
        assert_eq!(removed, 1); // only "old" (ts=1000 < 2000)
        // the SAMPLE rows (ts ~1.7e9) survive
        assert!(store.window_tokens(0).await.unwrap() > 0);
        // pruning an empty range removes nothing
        assert_eq!(store.prune(2_000).await.unwrap(), 0);
    }
```

  Add `use crate::transcript::CcUsageRecord;` to the test module's `use super::*;` neighbourhood if not already imported (it is reachable via `super::*` only if re-exported — `CcUsageRecord` is `crate::transcript::CcUsageRecord`; add the explicit `use crate::transcript::CcUsageRecord;` inside `mod tests`).
- [ ] Run `cargo test -p usage_telemetry prune_removes_rows` — confirm it FAILS to compile (no `prune`).
- [ ] **Implement.** In `impl CcUsageStore` (after `oldest_in_window`), add:

```rust
    /// Delete rows older than `before_ts` (bounds table growth; the spec prunes
    /// `now - 2 * window_secs` each sweep). Returns rows removed.
    pub async fn prune(&self, before_ts: i64) -> Result<u64, CcUsageError> {
        let res = sqlx::query("DELETE FROM cc_usage_log WHERE ts < ?")
            .bind(before_ts)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected())
    }
```

- [ ] Run `cargo test -p usage_telemetry prune_removes_rows` — confirm GREEN.
- [ ] Commit: `feat(usage): CcUsageStore::prune to bound cc_usage_log growth`.

---

## Task 2 — `TranscriptIngestor::ingest_changed` (new ingest.rs)

- [ ] **Create `usage_telemetry/src/ingest.rs`** with the struct, error, and a `#[cfg(test)]` module. Write the FAILING test first (the struct/method will not exist yet). File skeleton:

```rust
//! TranscriptIngestor — the production adapter that fills `cc_usage_log` from
//! Claude Code transcript JSONL on disk. Pure file IO + parse + store; no Tauri
//! types. The scan root is injected (the root passes `~/.claude/projects`; tests
//! pass a temp dir). Approach A: walk one level deep, mtime-gate, full re-parse;
//! `UNIQUE(message_id)` makes re-parse idempotent so no byte-offset bookkeeping.

use crate::cc_log::{CcUsageError, CcUsageStore};
use crate::transcript::parse_transcript;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum IngestError {
    #[error(transparent)]
    Cc(#[from] CcUsageError),
}

pub struct TranscriptIngestor {
    root: PathBuf,
    cc: Arc<CcUsageStore>,
}

impl TranscriptIngestor {
    pub fn new(root: PathBuf, cc: Arc<CcUsageStore>) -> Self {
        Self { root, cc }
    }

    /// mtime of a path in epoch seconds, or None if unavailable.
    fn mtime_secs(path: &std::path::Path) -> Option<i64> {
        let m = std::fs::metadata(path).ok()?;
        let mt = m.modified().ok()?;
        Some(mt.duration_since(UNIX_EPOCH).ok()?.as_secs() as i64)
    }

    /// Walk `root/<project-dir>/*.jsonl` one level deep, ingesting files whose
    /// mtime (epoch secs) is `> modified_since`. IO errors on individual files
    /// are skipped + logged. Returns total newly-inserted across all files.
    /// A missing root yields 0 (fresh machine: no `~/.claude/projects`).
    pub async fn ingest_changed(&self, modified_since: i64) -> Result<u64, IngestError> {
        self.ingest_gated(|mt| mt > modified_since).await
    }

    /// Shared walk; `gate(mtime_secs)` decides whether to parse a file.
    async fn ingest_gated(
        &self,
        gate: impl Fn(i64) -> bool,
    ) -> Result<u64, IngestError> {
        let mut total = 0u64;
        let project_dirs = match std::fs::read_dir(&self.root) {
            Ok(rd) => rd,
            Err(_) => return Ok(0), // root absent / unreadable: no-op
        };
        for proj in project_dirs.flatten() {
            let proj_path = proj.path();
            if !proj_path.is_dir() {
                continue;
            }
            let files = match std::fs::read_dir(&proj_path) {
                Ok(rd) => rd,
                Err(e) => {
                    eprintln!("usage: skip project dir {proj_path:?}: {e}");
                    continue;
                }
            };
            for f in files.flatten() {
                let path = f.path();
                if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                    continue;
                }
                let mt = match Self::mtime_secs(&path) {
                    Some(mt) => mt,
                    None => continue,
                };
                if !gate(mt) {
                    continue;
                }
                let blob = match std::fs::read_to_string(&path) {
                    Ok(b) => b,
                    Err(e) => {
                        eprintln!("usage: skip transcript {path:?}: {e}");
                        continue;
                    }
                };
                let records = parse_transcript(&blob);
                total += self.cc.ingest(&records).await?;
            }
        }
        Ok(total)
    }
}
```

  Note: `SystemTime`/`UNIX_EPOCH` are imported for `mtime_secs`; the unused-`SystemTime` lint will not fire because `mtime_secs` uses `UNIX_EPOCH` and `m.modified()` returns a `SystemTime`. If clippy flags an unused import, drop `SystemTime` from the `use`.

- [ ] **Failing test** (append to `ingest.rs`):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::cc_log::CcUsageStore;
    use sqlx::sqlite::SqlitePoolOptions;
    use sqlx::SqlitePool;
    use std::fs::{self, File};
    use std::io::Write;
    use std::time::Duration;

    async fn fresh_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        sqlx::query(include_str!("../../app/migrations/005_usage.sql")).execute(&pool).await.unwrap();
        pool
    }

    /// Unique temp dir under the OS temp root (never the real ~/.claude).
    fn temp_root(tag: &str) -> PathBuf {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("cc-ingest-{tag}-{nanos}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_jsonl(root: &std::path::Path, project: &str, name: &str, body: &str) -> PathBuf {
        let pd = root.join(project);
        fs::create_dir_all(&pd).unwrap();
        let p = pd.join(name);
        let mut f = File::create(&p).unwrap();
        f.write_all(body.as_bytes()).unwrap();
        p
    }

    const A: &str = r#"{"timestamp":"2026-06-22T11:14:00Z","message":{"id":"msg_aaa","model":"claude-opus-4-7","usage":{"input_tokens":1200,"output_tokens":300,"cache_creation_input_tokens":40,"cache_read_input_tokens":10}}}"#;
    const B: &str = r#"{"timestamp":"2026-06-22T11:20:00Z","message":{"id":"msg_bbb","usage":{"input_tokens":500,"output_tokens":120}}}"#;
    const NONUSAGE: &str = r#"{"type":"user","message":{"id":"x"}}"#;
    const PARTIAL: &str = r#"{"timestamp":"2026-06-22T11:25:00Z","message":{"id":"msg_ccc","usa"#; // truncated mid-write

    #[tokio::test]
    async fn ingest_changed_ingests_all_valid_then_dedupes() {
        let root = temp_root("all");
        // project p1: A + a non-usage line + a partial trailing line
        write_jsonl(&root, "p1", "s1.jsonl", &format!("{A}\n{NONUSAGE}\n{PARTIAL}"));
        // project p2: B
        write_jsonl(&root, "p2", "s2.jsonl", B);
        let cc = Arc::new(CcUsageStore::new(fresh_pool().await));
        let ing = TranscriptIngestor::new(root, cc.clone());

        let n = ing.ingest_changed(0).await.unwrap();
        assert_eq!(n, 2); // msg_aaa + msg_bbb; non-usage + partial skipped
        assert_eq!(cc.window_tokens(0).await.unwrap(), 1500 + 620); // 1500 (A) + 620 (B)

        // second pass with the same files: dedup => 0 new
        assert_eq!(ing.ingest_changed(0).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn ingest_changed_skips_files_at_or_below_modified_since() {
        let root = temp_root("gate");
        let p = write_jsonl(&root, "p1", "s.jsonl", A);
        // set mtime to a known epoch second (1_000_000)
        let when = UNIX_EPOCH + Duration::from_secs(1_000_000);
        File::options().write(true).open(&p).unwrap().set_modified(when).unwrap();
        let cc = Arc::new(CcUsageStore::new(fresh_pool().await));
        let ing = TranscriptIngestor::new(root, cc.clone());

        // gate is strict >: a file at exactly modified_since is skipped
        assert_eq!(ing.ingest_changed(1_000_000).await.unwrap(), 0);
        assert_eq!(cc.window_tokens(0).await.unwrap(), 0);
        // a lower watermark picks it up
        assert_eq!(ing.ingest_changed(999_999).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn ingest_changed_missing_root_is_noop() {
        let cc = Arc::new(CcUsageStore::new(fresh_pool().await));
        let ing = TranscriptIngestor::new(PathBuf::from("/no/such/dir/at/all"), cc);
        assert_eq!(ing.ingest_changed(0).await.unwrap(), 0);
    }
}
```

- [ ] **Wire the module** — in `usage_telemetry/src/lib.rs`, after `pub mod cc_log;` (line 11) add `pub mod ingest;`.
- [ ] Run `cargo test -p usage_telemetry ingest_changed` — confirm FAIL (compile), then after the impl is in place, GREEN. (The impl above is already written; if you wrote tests strictly first, temporarily stub the methods with `unimplemented!()` to watch the test fail, then fill in.)
- [ ] Commit: `feat(usage): TranscriptIngestor::ingest_changed walks ~/.claude/projects, mtime-gated`.

---

## Task 3 — `TranscriptIngestor::backfill_window` (ingest.rs)

- [ ] **Failing test first** (append to `ingest.rs` tests):

```rust
    #[tokio::test]
    async fn backfill_window_includes_recent_excludes_old_by_mtime() {
        let root = temp_root("backfill");
        let recent = write_jsonl(&root, "p1", "recent.jsonl", A);
        let old = write_jsonl(&root, "p2", "old.jsonl", B);
        let now = 2_000_000i64;
        let window_secs = 18_000i64; // 5h
        // recent: mtime just inside the window
        File::options().write(true).open(&recent).unwrap()
            .set_modified(UNIX_EPOCH + Duration::from_secs((now - 100) as u64)).unwrap();
        // old: mtime well before now - window - 3600
        File::options().write(true).open(&old).unwrap()
            .set_modified(UNIX_EPOCH + Duration::from_secs((now - window_secs - 7200) as u64)).unwrap();
        let cc = Arc::new(CcUsageStore::new(fresh_pool().await));
        let ing = TranscriptIngestor::new(root, cc.clone());

        let n = ing.backfill_window(window_secs, now).await.unwrap();
        assert_eq!(n, 1); // only the recent file (msg_aaa)
        assert_eq!(cc.window_tokens(0).await.unwrap(), 1500);
    }
```

- [ ] **Implement** — add to `impl TranscriptIngestor` (after `ingest_changed`):

```rust
    /// Boot backfill: ingest files modified within the rolling window plus a 1h
    /// buffer (`mtime > now - window_secs - 3600`), so the meter is correct on
    /// launch instead of 0 until the first new transcript line.
    pub async fn backfill_window(&self, window_secs: i64, now: i64) -> Result<u64, IngestError> {
        let cutoff = now - window_secs - 3600;
        self.ingest_gated(|mt| mt > cutoff).await
    }
```

- [ ] Run `cargo test -p usage_telemetry backfill_window` — GREEN.
- [ ] Run the full crate suite: `cargo test -p usage_telemetry` — GREEN.
- [ ] Commit: `feat(usage): TranscriptIngestor::backfill_window for boot priming`.

---

## Task 4 — Composition-root wiring: scan-root resolver + boot backfill (app/src/lib.rs)

This task is structural and not unit-testable in isolation (the sweep runs inside `setup` on a live `AppHandle`). Verify via `cargo build` + the existing app test suite + clippy. Keep the change minimal and mirror the surrounding style.

- [ ] **Add a scan-root resolver.** Near the other free functions in `app/src/lib.rs` (e.g. beside `now_unix` at `:1103` or the skill-root helpers around `:811`), add:

```rust
/// Resolve `~/.claude/projects` (the Claude Code transcript tree this app
/// ingests for the window meter). Mirrors the existing HOME convention; on a
/// machine with no HOME it returns `.claude/projects` (relative) which simply
/// yields an empty walk.
fn cc_claude_projects_dir() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    std::path::PathBuf::from(home).join(".claude").join("projects")
}
```

- [ ] **Build the ingestor** right after `cc_store`/`worker_usage` are constructed (`app/src/lib.rs:1330`), so it is in scope for both the boot backfill and the sweep:

```rust
                let ingestor = std::sync::Arc::new(usage_telemetry::ingest::TranscriptIngestor::new(
                    cc_claude_projects_dir(),
                    cc_store.clone(),
                ));
```

  (Confirm whether `Arc` is already imported as `Arc` at the top of `lib.rs` — line 1329 uses bare `Arc::new`, so use `Arc::new` here too rather than the fully-qualified form.)
- [ ] **Boot backfill** — place AFTER boot activation (after `app/src/lib.rs:1450`) and BEFORE the sweep `spawn` block at `:1455`:

```rust
                {
                    let cfg = usage_telemetry::api::load_config(&pool).await;
                    let n = ingestor.backfill_window(cfg.window_secs, now_unix()).await.unwrap_or(0);
                    if n > 0 {
                        let _ = handle.emit(crate::events::USAGE_CHANGED, ());
                    }
                }
```

  (Per the spec, emit `USAGE_CHANGED` after backfill so the meter is correct on launch. Emitting unconditionally is acceptable per the spec text; gating on `n > 0` avoids a needless refetch and matches Decision 4. Match `handle.emit` usage already present in the sweep.)
- [ ] Run `cargo build -p app` — confirm it compiles. Do NOT run the app.
- [ ] Commit: `feat(usage): resolve ~/.claude/projects scan root + boot backfill`.

---

## Task 5 — Restructure the 15s sweep: always ingest + prune + conditional emit; gate only the brake decision (app/src/lib.rs)

- [ ] **Capture the ingestor into the sweep task.** In the `{ ... }` block at `app/src/lib.rs:1455`, add `let ingestor = ingestor.clone();` alongside the existing `let cc = cc_store.clone();` etc. clones (so the spawned `async move` owns its own `Arc`).
- [ ] **Rewrite the loop body** (`app/src/lib.rs:1465-1478`). Replace the early `if !cfg.auto_meter_enabled { continue; }` placement so ingestion/prune/emit run unconditionally and only the brake decision is gated. New body:

```rust
                        let mut last_scan = now_unix();
                        loop {
                            tokio::time::sleep(std::time::Duration::from_secs(15)).await;
                            let cfg = load_config(&pool).await;
                            let now = now_unix();

                            // Always ingest changed transcripts (the meter must stay
                            // fresh even with the auto-brake disabled).
                            let new = ingestor.ingest_changed(last_scan).await.unwrap_or(0);
                            last_scan = now;
                            // Bound table growth: keep two full windows of margin.
                            let _ = cc.prune(now - 2 * cfg.window_secs).await;
                            if new > 0 {
                                let _ = handle.emit(crate::events::USAGE_CHANGED, ());
                            }

                            // Only the brake DECISION is gated on the enable flag.
                            if !cfg.auto_meter_enabled {
                                continue;
                            }
                            let auto_on = brake.state().reason.as_deref() == Some(AUTO_METER_REASON);
                            if let Ok(snap) = compute_snapshot(&cc, &worker, &cfg, brake.is_on(), now).await {
                                match auto_brake_decision(&snap, &cfg, auto_on) {
                                    BrakeDecision::SetOn(reason) => { brake.set_on(reason); let _ = handle.emit(crate::events::USAGE_CHANGED, ()); }
                                    BrakeDecision::Release => { brake.set_off(); let _ = handle.emit(crate::events::USAGE_CHANGED, ()); }
                                    BrakeDecision::NoChange => {}
                                }
                            }
```

  Notes:
  - `last_scan` starts at `now_unix()` at task entry, BEFORE the first `sleep(15s)`, so the first sweep only picks up files modified during/after boot (the boot backfill already covered the window). This matches the spec's high-water tracking.
  - `cc.prune(...)` uses the `prune` from Task 1. `cc` here is the `cc_store` clone (an `Arc<CcUsageStore>`); `Arc` derefs so `cc.prune(...)` works.
  - `now` is reused for the snapshot to keep the sweep's clock consistent within a tick.
  - The `use usage_telemetry::...` imports at the top of the spawned block (`app/src/lib.rs:1462-1464`) already bring `load_config`, `BrakeDecision`, `AUTO_METER_REASON`, `auto_brake_decision`, `compute_snapshot` into scope — no new `use` needed (the ingestor is called via the captured `Arc`, fully owned).
- [ ] Run `cargo build -p app` — confirm GREEN.
- [ ] Commit: `feat(usage): sweep always ingests+prunes+emits; gate only the brake decision`.

---

## Task 6 — Verification gates (final)

- [ ] `cargo test` (workspace) — all green, including the new `usage_telemetry` tests and existing app tests.
- [ ] `cargo clippy --all-targets -- -D warnings` — clean. Watch for: unused `SystemTime` import in `ingest.rs` (drop if flagged); `needless_return`; `manual_map` on the `mtime_secs`/read_dir match arms (rewrite as `?`/`if let` if clippy prefers).
- [ ] `cargo build` (workspace) — succeeds.
- [ ] Confirm no modifications leaked into the DO-NOT-MODIFY files: `transcript.rs`, `snapshot.rs`. (`cc_log.rs` is modified — `prune` only; `lib.rs` of the crate gains one `pub mod` line.)
- [ ] Final commit if any lint fixups were applied: `chore(usage): clippy fixups for transcript ingestion`.

---

## Data flow (for reviewer orientation)

transcripts (disk) → `ingest_changed` (mtime gate → `parse_transcript`) → `cc_usage_log` (dedup on `UNIQUE(message_id)`) → `compute_snapshot` reads `window_tokens(since)` → `UsageSnapshot.window_total`/burn/% → `USAGE_CHANGED` emitted → `useUsage` refetch → meter headline updates.

## Out of scope (per spec)

- Incremental byte-offset ingestion and a real-time FSEvents watcher (approaches B/C).
- Scoping the window to the app's own runs (operator chose all-activity).
- Attributing transcript tokens to teams (that stays `worker_usage_log`).
- Windows path specifics beyond `$HOME`-based resolution.
