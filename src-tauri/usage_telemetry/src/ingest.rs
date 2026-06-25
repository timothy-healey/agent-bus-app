//! TranscriptIngestor — the production adapter that fills `cc_usage_log` from
//! Claude Code transcript JSONL on disk. Pure file IO + parse + store; no Tauri
//! types. The scan root is injected (the root passes `~/.claude/projects`; tests
//! pass a temp dir). Approach A: walk one level deep, mtime-gate, full re-parse;
//! `UNIQUE(message_id)` makes re-parse idempotent so no byte-offset bookkeeping.

use crate::cc_log::{CcUsageError, CcUsageStore};
use crate::transcript::parse_transcript;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::UNIX_EPOCH;
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
    async fn ingest_gated(&self, gate: impl Fn(i64) -> bool) -> Result<u64, IngestError> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cc_log::CcUsageStore;
    use sqlx::sqlite::SqlitePoolOptions;
    use sqlx::SqlitePool;
    use std::fs::{self, File};
    use std::io::Write;
    use std::time::Duration;
    use std::time::{SystemTime, UNIX_EPOCH};

    async fn fresh_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(include_str!("../../app/migrations/005_usage.sql"))
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    /// Unique temp dir under the OS temp root (never the real ~/.claude).
    fn temp_root(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
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
        File::options()
            .write(true)
            .open(&p)
            .unwrap()
            .set_modified(when)
            .unwrap();
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
