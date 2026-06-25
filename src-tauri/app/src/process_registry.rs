//! ProcessRegistry — the composition-root holder of live child process-GROUP
//! ids. The killable spawner registers a pgid on spawn and deregisters it on
//! completion; `kill_all` signals every registered group (SIGTERM, grace,
//! SIGKILL). Deliberately lives in the `app` crate: no Runner/ChatRunner ACL
//! trait ever gains process types — process control is a root concern.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

/// Shared registry of live child process-group ids (pgid == leader pid because
/// the spawner sets `.process_group(0)`). Cloned behind an `Arc` at the root.
#[derive(Default)]
pub struct ProcessRegistry {
    /// The set of currently-live process-group ids.
    groups: Mutex<HashSet<i32>>,
}

impl ProcessRegistry {
    pub fn new() -> Self {
        Self {
            groups: Mutex::new(HashSet::new()),
        }
    }

    /// Record a live child process-group id (the spawner calls this right after
    /// spawn). Idempotent — re-registering the same pgid is a no-op.
    pub fn register(&self, pgid: i32) {
        self.groups.lock().unwrap().insert(pgid);
    }

    /// Drop a process-group id once its child has been waited on. A pgid that is
    /// not present is fine (best-effort).
    pub fn deregister(&self, pgid: i32) {
        self.groups.lock().unwrap().remove(&pgid);
    }

    /// Count of currently-registered groups (test/inspection helper).
    pub fn len(&self) -> usize {
        self.groups.lock().unwrap().len()
    }

    /// Whether the registry currently holds no live groups.
    pub fn is_empty(&self) -> bool {
        self.groups.lock().unwrap().is_empty()
    }

    /// Snapshot of the live pgids (used by `kill_all` so the kill loop does not
    /// hold the lock while sleeping).
    fn snapshot(&self) -> Vec<i32> {
        self.groups.lock().unwrap().iter().copied().collect()
    }

    /// Best-effort graceful kill of every registered process GROUP: SIGTERM the
    /// group, poll up to a ~2.5s grace window for it to exit, then SIGKILL any
    /// survivor. Drains the registry. Idempotent — a gone group (ESRCH) is fine.
    /// Negative pid signals the whole group (the spawner set `.process_group(0)`
    /// so pgid == the child's pid), so `claude`'s own tool/subagent children die
    /// too. On non-unix this is a logged no-op (the documented Windows gap).
    pub fn kill_all(&self) {
        let pgids = self.snapshot();
        if pgids.is_empty() {
            self.groups.lock().unwrap().clear();
            return;
        }
        #[cfg(unix)]
        {
            use std::time::{Duration, Instant};
            // 1. SIGTERM every group.
            for &pgid in &pgids {
                unsafe {
                    libc::kill(-pgid, libc::SIGTERM);
                }
            }
            // 2. Grace poll (~2.5s) for natural exit.
            let deadline = Instant::now() + Duration::from_millis(2500);
            loop {
                let alive: Vec<i32> = pgids
                    .iter()
                    .copied()
                    .filter(|&pgid| unsafe { libc::kill(-pgid, 0) } == 0)
                    .collect();
                if alive.is_empty() || Instant::now() >= deadline {
                    // 3. SIGKILL any survivor.
                    for &pgid in &alive {
                        unsafe {
                            libc::kill(-pgid, libc::SIGKILL);
                        }
                    }
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        #[cfg(not(unix))]
        {
            eprintln!(
                "ProcessRegistry::kill_all: non-unix build cannot kill {} process group(s) \
                 (documented Windows gap)",
                pgids.len()
            );
        }
        // Drain regardless of platform/outcome — these handles are spent.
        self.groups.lock().unwrap().clear();
    }
}

/// Build the production worker spawner closure: spawns `claude` in its own
/// process group, registers the pgid, captures output to completion, waits,
/// deregisters, and maps the result via `interpret_runner_output`. The group +
/// registry are what make `kill_all` reach `claude`'s own children (LF20).
pub(crate) fn killable_spawn(registry: &Arc<ProcessRegistry>) -> runners::claude_cli::SpawnFn {
    let registry = registry.clone();
    Box::new(move |args: &[String], cwd: Option<&str>| {
        use runners::output::RunnerError;
        use std::process::Stdio;
        let (program, rest) = args
            .split_first()
            .ok_or_else(|| RunnerError::Spawn("empty argv".into()))?;
        let mut cmd = std::process::Command::new(program);
        cmd.args(rest).stdout(Stdio::piped()).stderr(Stdio::piped());
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0); // child leads a fresh group; pgid == child pid
        }
        let mut child = cmd.spawn().map_err(|e| RunnerError::Spawn(e.to_string()))?;
        let pgid = child.id() as i32;
        registry.register(pgid);
        // Capture to completion, then wait. (Reads the piped handles; for the
        // streaming path the engine still forwards deltas via the stream parser
        // over the returned stdout — same as the .output() path.)
        use std::io::Read;
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        if let Some(mut o) = child.stdout.take() {
            let _ = o.read_to_end(&mut stdout);
        }
        if let Some(mut e) = child.stderr.take() {
            let _ = e.read_to_end(&mut stderr);
        }
        let status = child.wait().map_err(|e| RunnerError::Spawn(e.to_string()));
        registry.deregister(pgid);
        let status = status?;
        runners::claude_cli::interpret_runner_output(stdout, stderr, status.success())
    })
}

/// Build a worker `ClaudeCliRunner` wired to the killable spawner.
pub(crate) fn build_killable_worker_runner(
    registry: Arc<ProcessRegistry>,
) -> Arc<dyn runners::output::Runner> {
    Arc::new(runners::claude_cli::ClaudeCliRunner::with_spawner(
        killable_spawn(&registry),
    ))
}

/// Same for the chat runner (capture-to-completion via the chat SpawnFn).
pub(crate) fn killable_chat_spawn(registry: &Arc<ProcessRegistry>) -> llm_chat::claude_cli::SpawnFn {
    let registry = registry.clone();
    Box::new(move |args: &[String], cwd: Option<&str>| {
        use llm_chat::chat::ChatError;
        use std::process::Stdio;
        let mut cmd = std::process::Command::new(runners::command::CLAUDE_BIN);
        cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        let mut child = cmd.spawn().map_err(|e| ChatError::Spawn(e.to_string()))?;
        let pgid = child.id() as i32;
        registry.register(pgid);
        use std::io::Read;
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        if let Some(mut o) = child.stdout.take() {
            let _ = o.read_to_end(&mut stdout);
        }
        if let Some(mut e) = child.stderr.take() {
            let _ = e.read_to_end(&mut stderr);
        }
        let status = child.wait().map_err(|e| ChatError::Spawn(e.to_string()));
        registry.deregister(pgid);
        let status = status?;
        llm_chat::claude_cli::interpret_chat_output(stdout, stderr, status.success())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_then_deregister_leaves_the_set_empty() {
        let reg = ProcessRegistry::new();
        reg.register(1234);
        reg.register(5678);
        assert_eq!(reg.len(), 2);
        reg.deregister(1234);
        assert_eq!(reg.len(), 1);
        reg.deregister(5678);
        assert_eq!(reg.len(), 0);
    }

    #[test]
    fn deregister_unknown_pgid_is_a_noop() {
        let reg = ProcessRegistry::new();
        reg.deregister(9999);
        assert_eq!(reg.len(), 0);
    }

    #[test]
    fn register_is_idempotent_on_the_same_pgid() {
        let reg = ProcessRegistry::new();
        reg.register(42);
        reg.register(42);
        assert_eq!(reg.len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn killable_spawner_runs_a_child_captures_output_and_drains_registry() {
        let reg = Arc::new(ProcessRegistry::new());
        let out = (super::killable_spawn(&reg))(
            &["sh".into(), "-c".into(), "printf hello".into()],
            None,
        );
        assert_eq!(out.unwrap(), "hello");
        assert!(reg.is_empty(), "registry drained after the child is waited on");
    }

    #[cfg(unix)]
    mod unix_kill {
        use super::super::*;
        use std::os::unix::process::CommandExt;
        use std::process::Command;
        use std::time::{Duration, Instant};

        /// Is the process group still alive? `kill(-pgid, 0)` probes without
        /// signalling: Ok(0) => alive, Err(ESRCH) => gone.
        fn group_alive(pgid: i32) -> bool {
            unsafe { libc::kill(-pgid, 0) == 0 }
        }

        #[test]
        fn kill_all_terminates_a_well_behaved_group_and_empties_the_registry() {
            // `sh -c 'sleep 30 & wait'` => the shell leads the group, a child
            // sleep is in the same group, so killing the GROUP must take both.
            let child = Command::new("sh")
                .arg("-c")
                .arg("sleep 30 & wait")
                .process_group(0)
                .spawn()
                .expect("spawn");
            let pgid = child.id() as i32;
            let reg = ProcessRegistry::new();
            reg.register(pgid);

            reg.kill_all();

            assert!(reg.is_empty(), "registry drained after kill_all");
            // Give the OS a beat to reap, then assert the group is gone.
            let deadline = Instant::now() + Duration::from_secs(3);
            while group_alive(pgid) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(50));
            }
            assert!(!group_alive(pgid), "group must be dead after kill_all");
        }

        #[test]
        fn kill_all_escalates_to_sigkill_for_a_sigterm_ignoring_group() {
            // `trap '' TERM` makes the shell ignore SIGTERM; only SIGKILL ends it.
            let child = Command::new("sh")
                .arg("-c")
                .arg("trap '' TERM; sleep 30")
                .process_group(0)
                .spawn()
                .expect("spawn");
            let pgid = child.id() as i32;
            let reg = ProcessRegistry::new();
            reg.register(pgid);

            let start = Instant::now();
            reg.kill_all();
            // kill_all blocks for ~the grace window then SIGKILLs.
            assert!(start.elapsed() < Duration::from_secs(6), "grace bounded");

            let deadline = Instant::now() + Duration::from_secs(3);
            while group_alive(pgid) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(50));
            }
            assert!(!group_alive(pgid), "SIGTERM-ignoring group dies via SIGKILL");
        }
    }
}
