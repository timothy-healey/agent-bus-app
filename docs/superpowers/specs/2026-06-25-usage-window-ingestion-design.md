# Spec — Usage window ingestion (cc_usage_log)

*Design doc. Brainstormed 2026-06-25. Fixes **LF28**: the usage meter's headline (window %, burn, tokens-this-window) reads `cc_usage_log`, which is never populated, so it shows 0% even while workers run. The per-team tooltip reads the separate `worker_usage_log` (which is populated), hence the split the operator sees. This wires the missing ingestion so the headline reflects the real shared Claude subscription window.*

## Why this exists

`compute_snapshot` (`usage_telemetry/src/snapshot.rs:84`) computes the window total from `cc.window_tokens(since)` + `worker.api_window_tokens(since)` (the latter "zero in v1"). `cc_usage_log` is meant to be fed by ingesting Claude Code transcripts (`transcript::parse_transcript` + `CcUsageStore::ingest`), but **`.ingest()` is called only in tests** — there is no watcher/poller, no `read_dir`, no `~/.claude/projects` reference in production. So `cc_usage_log` is permanently empty (live DB: 0 rows), the headline is 0%, and the auto-meter brake (which reads `window_pct`) can never trip. The per-team breakdown comes from `worker_usage_log` (populated by the `UsageSink` per invocation), which is why the tooltip shows real tokens while the headline shows none.

**Decision (from the brainstorm):** the window total should count **all Claude Code activity** — the app's workers *and* the operator's own interactive sessions/chat — because the rolling window the brake protects is the shared subscription rate-limit, not just the app's spend. So ingestion scans the whole `~/.claude/projects/` tree. `worker_usage_log` stays the separate per-team attribution source (no double-count risk: the headline reads only `cc_usage_log`).

The transcript format is already compatible: a real assistant line carries top-level `timestamp` (RFC3339) + `message.id` + `message.model` + `message.usage.{input_tokens, output_tokens, cache_creation_input_tokens, cache_read_input_tokens}`, exactly what `parse_transcript_line` reads. No parser change needed.

## Decisions

1. **Approach A — poll on the existing 15s auto-meter sweep, mtime-gated, full re-parse of changed files.** The sweep loop (`app/src/lib.rs:1465`) already runs every 15s; ingest at the top of each tick. List `~/.claude/projects/**/*.jsonl` modified since the last pass, re-parse each whole, and `ingest` — the table's `UNIQUE(message_id)` makes re-parsing idempotent (no byte-offset bookkeeping). Rejected: incremental byte-offset (B — premature optimization) and a real-time FSEvents watcher (C — new dep + thread for a meter that refreshes on a 15s cadence anyway).
2. **Always ingest, regardless of `auto_meter_enabled`.** The current sweep does `if !cfg.auto_meter_enabled { continue; }` at the top, which would skip ingestion. Ingestion (and the snapshot freshness it enables) must run even with the auto-brake disabled; only the brake *decision* is gated on `auto_meter_enabled`.
3. **Boot backfill.** At startup, before the sweep loop, ingest once over files modified within the rolling window (+ a buffer) so the meter is correct on launch instead of 0 until the first new transcript line.
4. **Emit `USAGE_CHANGED` when rows were inserted.** After a pass that ingested ≥1 new record, emit `USAGE_CHANGED` so `useUsage` refetches and the meter updates even when no `task-changed` is firing (idle / operator's own sessions). No emit on a zero-insert pass (no needless refetch).
5. **Bound table growth.** `cc_usage_log` accumulates forever otherwise; `window_tokens`/`oldest_in_window` already filter by `ts > since`, so rows older than the window are dead weight. Prune rows with `ts < now - 2 * window_secs` each pass (cheap DELETE; keeping two full windows leaves ample margin so `reset_in`/`oldest_in_window` stay correct).

## Architecture

### `TranscriptIngestor` (usage_telemetry crate, new `ingest.rs`)
A small unit owning the scan root + the `CcUsageStore`:
```
pub struct TranscriptIngestor { root: PathBuf, cc: Arc<CcUsageStore> }
```
- `async fn ingest_changed(&self, modified_since: i64) -> Result<u64, IngestError>` — walk `root` one level deep (`root/<project-dir>/*.jsonl`), for each `*.jsonl` whose mtime (epoch secs) `> modified_since`: read to string (skip on IO error, logged), `parse_transcript`, `cc.ingest(&records)`. Returns total newly-inserted across files.
- `async fn backfill_window(&self, window_secs: i64, now: i64) -> Result<u64, IngestError>` — same walk, but gate files by `mtime > now - window_secs - 3600` (a 1h buffer beyond the window; don't parse the operator's entire history). Used once at boot.
- Pure file IO + parse + store; no Tauri types. The scan root is injected (default `~/.claude/projects`, resolved at the composition root; tests pass a temp dir).

`parse_transcript` is already lenient (a malformed or partial trailing line in a transcript still being written → `None`, skipped), so a file mid-write is safe; the completed line is picked up on a later pass and `UNIQUE(message_id)` prevents a double-count.

### `CcUsageStore::prune` (cc_log.rs)
`async fn prune(&self, before_ts: i64) -> Result<u64, CcUsageError>` — `DELETE FROM cc_usage_log WHERE ts < ?`. Returns rows removed.

### Composition-root wiring (`app/src/lib.rs`)
- Build `let ingestor = Arc::new(TranscriptIngestor::new(cc_claude_projects_dir(), cc_store.clone()));` where `cc_claude_projects_dir()` resolves `~/.claude/projects` (home dir via `dirs`/`std::env`, or the Tauri path API).
- **Boot:** in `setup`, before spawning the sweep, `let _ = ingestor.backfill_window(cfg.window_secs, now_unix()).await;` then `let _ = handle.emit(USAGE_CHANGED, ());`.
- **Sweep (restructure the loop at `:1465`):**
  ```
  let mut last_scan = now_unix();
  loop {
      tokio::time::sleep(Duration::from_secs(15)).await;
      let cfg = load_config(&pool).await;
      let now = now_unix();
      let new = ingestor.ingest_changed(last_scan).await.unwrap_or(0);
      last_scan = now;
      let _ = cc_store.prune(now - 2 * cfg.window_secs).await;
      if new > 0 { let _ = handle.emit(crate::events::USAGE_CHANGED, ()); }
      if !cfg.auto_meter_enabled { continue; }            // brake decision only, gated
      // … existing compute_snapshot + auto_brake_decision unchanged …
  }
  ```
  The ingestor `Arc` is cloned into the spawned task alongside the existing captures.

### Data flow
transcripts (disk) → `ingest_changed` (parse) → `cc_usage_log` (dedup) → `compute_snapshot` reads `window_tokens(since)` → `UsageSnapshot.window_total`/burn/% → `USAGE_CHANGED` → `useUsage` refetch → meter.

## Components / files
- New: `usage_telemetry/src/ingest.rs` (`TranscriptIngestor`); exported from `usage_telemetry/src/lib.rs`.
- Modified: `usage_telemetry/src/cc_log.rs` (`prune`); `app/src/lib.rs` (resolve scan root; boot backfill; restructure sweep to always-ingest + prune + conditional emit; gate only the brake decision on `auto_meter_enabled`).
- Unchanged: `snapshot.rs` (math already correct once `cc_usage_log` fills); `transcript.rs` (parser already format-compatible).

## Error handling
- Missing scan root (`~/.claude/projects` absent, fresh machine): walk yields nothing → ingest is a no-op, meter stays 0 until CC is used. Logged once, not an error.
- Unreadable file / permission denied: skip that file + debug log, continue the walk.
- Malformed / partial trailing line: `parse_transcript` skips it (already lenient).
- A pass that errors entirely: `unwrap_or(0)` — the sweep continues; next pass retries.

## Testing (no live `claude`, no real `~/.claude`)
- **ingest_changed (temp dir):** two project subdirs with fixture `*.jsonl` (usage lines + non-usage + a partial trailing line); `ingest_changed(0)` ingests all valid records; `window_tokens` reflects them. A second `ingest_changed(0)` ingests 0 new (dedup).
- **mtime gate:** a file with mtime ≤ `modified_since` is skipped; a freshly-touched file is picked up.
- **backfill_window:** files modified within the window are ingested; older files skipped.
- **prune:** rows with `ts < before_ts` removed; in-window rows kept.
- **Wiring (structural):** the sweep ingests before the `auto_meter_enabled` guard (ingestion runs with auto-meter off); boot calls `backfill_window`; `USAGE_CHANGED` emitted only when new rows were inserted.

## Out of scope / non-goals
- Incremental byte-offset ingestion and a real-time FSEvents watcher (approaches B/C — revisit only if a sweep ever shows a real perf cost).
- Scoping the window to the app's own runs (the operator chose all-activity).
- Attributing transcript tokens to teams (that's `worker_usage_log`'s job; `cc_usage_log` is the undifferentiated window total).
- Windows path specifics beyond what `~/.claude/projects` resolution already gives.

## Relationship to other items
- Fixes **LF28** (headline meter stuck at 0%).
- Independent of the lifecycle and live-worker-view specs (no shared seam) — can build in any order relative to them.
- Note: the LF26 cwd fix (lifecycle spec) moves where *worker* transcripts are written under `~/.claude/projects` (different cwd-hash dir), but since this ingester scans the whole tree, it is unaffected.
