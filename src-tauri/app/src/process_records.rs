//! LiveProcessStore — durable record of spawned `claude` process groups so an
//! UNCLEAN app death can be reaped on the next boot, BEFORE recovery re-queues
//! the task. A composition-root concern (the runtime crate stays unaware),
//! mirroring the persisted-brake pattern. Unix-only reap; the record itself is
//! cross-platform.

use sqlx::SqlitePool;

pub struct LiveProcessStore {
    pool: SqlitePool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveProcess {
    pub pgid: i64,
    pub run_id: Option<String>,
    pub task_id: Option<String>,
    pub started_ts: i64,
}

impl LiveProcessStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn insert(
        &self,
        pgid: i64,
        run_id: Option<&str>,
        task_id: Option<&str>,
        started_ts: i64,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT OR REPLACE INTO live_processes (pgid, run_id, task_id, started_ts) VALUES (?,?,?,?)",
        )
        .bind(pgid)
        .bind(run_id)
        .bind(task_id)
        .bind(started_ts)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete(&self, pgid: i64) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM live_processes WHERE pgid = ?")
            .bind(pgid)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn list(&self) -> Result<Vec<LiveProcess>, sqlx::Error> {
        let rows: Vec<(i64, Option<String>, Option<String>, i64)> =
            sqlx::query_as("SELECT pgid, run_id, task_id, started_ts FROM live_processes")
                .fetch_all(&self.pool)
                .await?;
        Ok(rows
            .into_iter()
            .map(|(pgid, run_id, task_id, started_ts)| LiveProcess {
                pgid,
                run_id,
                task_id,
                started_ts,
            })
            .collect())
    }

    pub async fn clear_all(&self) -> Result<u64, sqlx::Error> {
        Ok(sqlx::query("DELETE FROM live_processes")
            .execute(&self.pool)
            .await?
            .rows_affected())
    }
}

/// A coarse process-start fingerprint for the PID-reuse guard: the kernel's
/// reported start time for `pid` (via `ps -o lstart=`). None if the pid is gone.
/// Equality of this value across the spawn-record and the boot-reap means the
/// pid was not recycled. Shells out to `ps`; unix-only.
#[cfg(unix)]
pub fn process_start_ts(pid: i32) -> Option<i64> {
    // `ps -o lstart=` is a human date; hash it to a stable i64 so it fits the
    // INTEGER column and compares exactly. A recycled pid yields a different
    // start date -> different hash -> guard rejects it.
    let out = std::process::Command::new("ps")
        .args(["-o", "lstart=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    Some(h.finish() as i64)
}

#[cfg(not(unix))]
pub fn process_start_ts(_pid: i32) -> Option<i64> {
    None
}

/// Boot reap (LH4): for each recorded prior-session pgid that is STILL ALIVE and
/// whose start-time fingerprint still matches (not a recycled pid), SIGTERM the
/// group, grace, SIGKILL; then clear ALL records (a clean exit left none anyway).
/// MUST run BEFORE release_orphaned_running / reconcile_occupancy so the re-run
/// has no surviving competitor. Best-effort + logged; never blocks boot fatally.
pub async fn reap_orphans(store: &LiveProcessStore) {
    let records = match store.list().await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("app: reap_orphans list failed: {e}");
            return;
        }
    };
    #[cfg(unix)]
    for rec in &records {
        let pgid = rec.pgid as i32;
        // Alive? probe with signal 0.
        if unsafe { libc::kill(-pgid, 0) } != 0 {
            continue;
        }
        // PID-reuse guard: the live start-time must match the recorded one.
        if process_start_ts(pgid) != Some(rec.started_ts) {
            eprintln!(
                "app: reap_orphans skipping pgid {pgid} (start-time mismatch / recycled)"
            );
            continue;
        }
        unsafe {
            libc::kill(-pgid, libc::SIGTERM);
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(2500);
        while unsafe { libc::kill(-pgid, 0) } == 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        if unsafe { libc::kill(-pgid, 0) } == 0 {
            unsafe {
                libc::kill(-pgid, libc::SIGKILL);
            }
        }
    }
    #[cfg(not(unix))]
    let _ = &records;
    if let Err(e) = store.clear_all().await {
        eprintln!("app: reap_orphans clear_all failed: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::str::FromStr;

    async fn fresh_pool() -> SqlitePool {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .foreign_keys(false);
        let pool = SqlitePoolOptions::new().connect_with(opts).await.unwrap();
        sqlx::query(include_str!("../migrations/013_lifecycle_hardening.sql"))
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    #[tokio::test]
    async fn insert_list_delete_round_trip() {
        let store = LiveProcessStore::new(fresh_pool().await);
        store.insert(1234, Some("R-1"), Some("T-1"), 500).await.unwrap();
        let all = store.list().await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].pgid, 1234);
        assert_eq!(all[0].started_ts, 500);
        store.delete(1234).await.unwrap();
        assert!(store.list().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn clear_all_empties_the_table() {
        let store = LiveProcessStore::new(fresh_pool().await);
        store.insert(1, None, None, 1).await.unwrap();
        store.insert(2, None, None, 2).await.unwrap();
        assert_eq!(store.clear_all().await.unwrap(), 2);
        assert!(store.list().await.unwrap().is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    #[allow(clippy::zombie_processes)] // reap_orphans signals the group; no direct wait
    async fn reap_orphans_kills_a_recorded_survivor_and_clears_records() {
        use std::os::unix::process::CommandExt;
        use std::process::Command;
        use std::time::{Duration, Instant};
        let store = LiveProcessStore::new(fresh_pool().await);
        let child = Command::new("sh")
            .arg("-c")
            .arg("sleep 30 & wait")
            .process_group(0)
            .spawn()
            .expect("spawn");
        let pgid = child.id() as i32;
        // Record it as a prior-session orphan. started_ts: read the live start-time
        // so the guard matches (in production the spawner records the same probe).
        let started = super::process_start_ts(pgid).unwrap_or(0);
        store
            .insert(pgid as i64, Some("R-1"), Some("T-1"), started)
            .await
            .unwrap();

        super::reap_orphans(&store).await;

        assert!(store.list().await.unwrap().is_empty(), "records cleared after reap");
        let alive = |p: i32| unsafe { libc::kill(-p, 0) == 0 };
        let deadline = Instant::now() + Duration::from_secs(3);
        while alive(pgid) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(!alive(pgid), "the orphan was reaped");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn reap_orphans_skips_a_pid_with_a_mismatched_start_time() {
        // A record whose started_ts does NOT match the live process is a recycled
        // pid: it must NOT be signalled. Use our OWN pid with a bogus start-time.
        let store = LiveProcessStore::new(fresh_pool().await);
        let me = std::process::id() as i64;
        store.insert(me, None, None, /*bogus*/ 1).await.unwrap();
        super::reap_orphans(&store).await; // must not kill ourselves
        assert!(store.list().await.unwrap().is_empty(), "records still cleared");
        // If we are still running to assert this, we were not killed. :)
    }
}
