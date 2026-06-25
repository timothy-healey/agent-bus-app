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

/// Decide whether a recorded live-process group should be reaped on boot.
/// `recorded_ts` = the start-time fingerprint stored at spawn (`0` if `ps` failed
/// then). `probe` = `process_start_ts(leader pgid)` at boot (`None` = the leader
/// pid is gone / `ps` failed, even though the GROUP may still be alive because a
/// child outlives it). `group_alive` = `kill(-pgid, 0)` succeeded.
///
/// The PID-reuse guard must only protect against signalling a *stranger*: a
/// POSITIVE start-ts mismatch (a recycled leader, recorded != 0 and probe != it)
/// is the one case we skip. Otherwise — an exact match, a leader that has exited
/// while a group child survives (`None`), or a record whose `started_ts` was `0`
/// (ps failed at spawn) — we REAP the still-alive group. This closes the LH4
/// under-reap hole where `claude`'s subagent children outlive `claude` itself.
fn should_reap(recorded_ts: i64, probe: Option<i64>, group_alive: bool) -> bool {
    if !group_alive {
        return false; // nothing to kill (caller clears the row regardless)
    }
    match probe {
        // Recycled leader: a DIFFERENT live start-time -> don't signal a stranger.
        Some(ts) if recorded_ts != 0 && ts != recorded_ts => false,
        // Exact match, leader-gone-but-group-alive (None), or recorded 0 -> reap.
        _ => true,
    }
}

/// Boot reap (LH4): for each recorded prior-session pgid that is STILL ALIVE and
/// is not a recycled-leader stranger (see `should_reap`), SIGTERM the group,
/// grace, SIGKILL; then clear ALL records (a clean exit left none anyway).
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
        // Alive? probe the GROUP with signal 0 (a child can keep the group alive
        // after the leader exits).
        let group_alive = unsafe { libc::kill(-pgid, 0) } == 0;
        // PID-reuse guard: probe the LEADER's start-time. Only a POSITIVE mismatch
        // (recycled leader) skips; a gone leader (None) over a still-alive group,
        // or a recorded 0, still reaps. See `should_reap`.
        if !should_reap(rec.started_ts, process_start_ts(pgid), group_alive) {
            if group_alive {
                eprintln!(
                    "app: reap_orphans skipping pgid {pgid} (start-time mismatch / recycled leader)"
                );
            }
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

    #[test]
    fn should_reap_decision_matrix() {
        // Exact start-ts match on a live group -> reap.
        assert!(should_reap(500, Some(500), true), "exact match -> reap");
        // Positive mismatch (recycled leader) -> skip; never signal a stranger.
        assert!(
            !should_reap(500, Some(999), true),
            "recycled leader (positive mismatch) -> skip"
        );
        // Leader gone (None) but the GROUP is still alive — claude's subagent child
        // outlived claude. This is the LH4 case; it MUST reap.
        assert!(
            should_reap(500, None, true),
            "leader-gone-but-group-alive -> reap"
        );
        // Recorded started_ts == 0 (ps failed at spawn) on a live group -> reap;
        // the 0 sentinel must never make the guard skip a real survivor.
        assert!(
            should_reap(0, Some(123), true),
            "recorded 0 + live group -> reap"
        );
        assert!(
            should_reap(0, None, true),
            "recorded 0 + leader gone + live group -> reap"
        );
        // Group already dead -> nothing to kill (the caller still clears the row).
        assert!(!should_reap(500, Some(500), false), "dead group -> no reap");
        assert!(!should_reap(0, None, false), "dead group (any ts) -> no reap");
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
