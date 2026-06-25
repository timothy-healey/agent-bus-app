//! ProcessRegistry — the composition-root holder of live child process-GROUP
//! ids. The killable spawner registers a pgid on spawn and deregisters it on
//! completion; `kill_all` signals every registered group (SIGTERM, grace,
//! SIGKILL). Deliberately lives in the `app` crate: no Runner/ChatRunner ACL
//! trait ever gains process types — process control is a root concern.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// What the spawner must do after attempting to register a freshly-spawned child.
pub enum RegisterDecision {
    /// The pgid was recorded; proceed normally (deregister on completion).
    Registered,
    /// A kill is in progress (latch set): the spawner must immediately kill the
    /// child it just spawned and NOT track it. Closes the spawn-after-snapshot
    /// and register-after-spawn TOCTOU windows.
    KillImmediately,
}

/// The lock-guarded interior of the registry: the live process-group map plus
/// the kill latch. One critical section orders the latch read/write with the
/// map so a register cannot race past an in-progress kill (LH1).
#[derive(Default)]
struct Inner {
    /// pgid -> owned child handle (LH2: held until reaped, so kill_all never
    /// signals a recycled pgid). `None` once the reaper took it out to `.wait()`,
    /// or when registered via the bare `register`/`register_pgid` helpers (tests
    /// / back-compat) where no `Child` is owned.
    groups: HashMap<i32, Arc<Mutex<Option<std::process::Child>>>>,
    /// Latch: while set, no new child may register (it self-kills instead).
    killing: bool,
}

/// Shared registry of live child process-group ids (pgid == leader pid because
/// the spawner sets `.process_group(0)`). Cloned behind an `Arc` at the root.
#[derive(Default)]
pub struct ProcessRegistry {
    /// The live process-group map + kill latch, behind one lock.
    inner: Mutex<Inner>,
}

impl ProcessRegistry {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
        }
    }

    /// Set the killing latch (called at the start of kill_all / kill_workers).
    pub fn begin_killing(&self) {
        self.inner.lock().unwrap().killing = true;
    }

    /// Clear the latch — re-enable normal spawning (wire on brake-off / resume).
    pub fn end_killing(&self) {
        self.inner.lock().unwrap().killing = false;
    }

    /// Register a freshly-spawned pgid under the lock. If the latch is set the
    /// spawner is told to self-kill its child instead of tracking it.
    pub fn register_pgid(&self, pgid: i32) -> RegisterDecision {
        let mut g = self.inner.lock().unwrap();
        if g.killing {
            return RegisterDecision::KillImmediately;
        }
        g.groups.insert(pgid, Arc::new(Mutex::new(None)));
        RegisterDecision::Registered
    }

    /// Register the owned Child under the lock (LH2). Returns the decision; on
    /// `Registered` the entry holds the Child so kill_all signals only live pids.
    pub(crate) fn register_child(
        &self,
        pgid: i32,
        child: std::process::Child,
    ) -> RegisterDecision {
        let mut g = self.inner.lock().unwrap();
        if g.killing {
            return RegisterDecision::KillImmediately;
        }
        g.groups.insert(pgid, Arc::new(Mutex::new(Some(child))));
        RegisterDecision::Registered
    }

    /// Take the owned Child out of the registry under the lock (the reaper calls
    /// this immediately before `.wait()`), so kill_all can never observe a pgid
    /// whose process was already reaped+recycled.
    pub(crate) fn take_child(&self, pgid: i32) -> Option<std::process::Child> {
        let mut g = self.inner.lock().unwrap();
        g.groups.remove(&pgid).and_then(|c| c.lock().unwrap().take())
    }

    /// Record a live child process-group id (the spawner calls this right after
    /// spawn). Idempotent — re-registering the same pgid is a no-op.
    pub fn register(&self, pgid: i32) {
        self.inner
            .lock()
            .unwrap()
            .groups
            .insert(pgid, Arc::new(Mutex::new(None)));
    }

    /// Drop a process-group id once its child has been waited on. A pgid that is
    /// not present is fine (best-effort).
    pub fn deregister(&self, pgid: i32) {
        self.inner.lock().unwrap().groups.remove(&pgid);
    }

    /// Count of currently-registered groups (test/inspection helper).
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().groups.len()
    }

    /// Whether the registry currently holds no live groups (test/inspection
    /// helper).
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.inner.lock().unwrap().groups.is_empty()
    }


    /// Best-effort graceful kill of every registered process GROUP: SIGTERM the
    /// group, poll up to a ~2.5s grace window for it to exit, then SIGKILL any
    /// survivor. Drains the registry. Idempotent — a gone group (ESRCH) is fine.
    /// Negative pid signals the whole group (the spawner set `.process_group(0)`
    /// so pgid == the child's pid), so `claude`'s own tool/subagent children die
    /// too. On non-unix this is a logged no-op (the documented Windows gap).
    pub fn kill_all(&self) {
        // Set the latch FIRST so any child racing past the brake gate self-kills
        // on register instead of escaping the snapshot. The latch stays set until
        // end_killing() (brake-off / resume) — kill_all never clears it.
        self.begin_killing();
        // Take every entry OUT under the lock: we now own each `Child`, so the
        // leader pid stays un-reaped (un-recyclable) until after we SIGKILL it —
        // closing the PID-reuse window (LH2). Drains the registry.
        let taken: Vec<(i32, Arc<Mutex<Option<std::process::Child>>>)> = {
            let mut g = self.inner.lock().unwrap();
            g.groups.drain().collect()
        };
        self.signal_and_reap(taken);
    }

    /// Shared kill primitive: SIGTERM each group, grace-poll ~2.5s, SIGKILL any
    /// survivor, then `.wait()` each owned `Child` to reap the zombie. Holding the
    /// owned `Child` until after SIGKILL means the leader pid cannot be recycled
    /// mid-sweep (LH2). Idempotent — a gone group (ESRCH) is fine.
    fn signal_and_reap(&self, taken: Vec<(i32, Arc<Mutex<Option<std::process::Child>>>)>) {
        if taken.is_empty() {
            return;
        }
        let pgids: Vec<i32> = taken.iter().map(|(p, _)| *p).collect();
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
        // Reap the owned children we took out so the leader pid is freed cleanly
        // (no zombie). A spawner thread racing us finds take_child -> None and
        // returns an interrupt error without double-waiting.
        for (_pgid, slot) in taken {
            if let Some(mut c) = slot.lock().unwrap().take() {
                let _ = c.wait();
            }
        }
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
        // Take the pipe handles off the child FIRST: the registry owns the
        // `Child` (LH2) so kill_all can signal only live, un-reaped pids, but the
        // drain below needs the stdout/stderr handles which we keep locally.
        use std::io::Read;
        let stderr_handle = child.stderr.take().map(|mut e| {
            std::thread::spawn(move || {
                let mut buf = Vec::new();
                let _ = e.read_to_end(&mut buf);
                buf
            })
        });
        let mut stdout_pipe = child.stdout.take();
        // Register the owned child under the lock. On a latched register the
        // child was moved in here, so reclaim it via take_child to self-kill.
        match registry.register_child(pgid, child) {
            RegisterDecision::Registered => {}
            RegisterDecision::KillImmediately => {
                // A Stop/exit kill is in progress; this claim raced past the
                // brake gate. Kill the child we just spawned (whole group) so it
                // cannot escape, then surface an interrupt-class failure.
                #[cfg(unix)]
                unsafe {
                    libc::kill(-pgid, libc::SIGKILL);
                }
                if let Some(mut c) = registry.take_child(pgid) {
                    let _ = c.wait();
                }
                return Err(RunnerError::Other("spawn aborted: kill in progress".into()));
            }
        }
        // Drain BOTH pipes CONCURRENTLY, then wait. A sequential stdout-then-
        // stderr drain deadlocks when the child writes more than one pipe buffer
        // (~64KB) to stderr before stdout reaches EOF (parent blocks on stdout,
        // child blocks writing stderr). Read stderr on a worker thread while this
        // thread reads stdout. (For the streaming path the engine still forwards
        // deltas via the stream parser over the returned stdout.)
        let mut stdout = Vec::new();
        if let Some(mut o) = stdout_pipe.take() {
            let _ = o.read_to_end(&mut stdout);
        }
        let stderr = stderr_handle
            .map(|h| h.join().unwrap_or_default())
            .unwrap_or_default();
        // Take the owned Child out of the registry under the lock BEFORE waiting,
        // so kill_all can never observe (and signal) an already-reaped pgid.
        let status = match registry.take_child(pgid) {
            Some(mut c) => c.wait().map_err(|e| RunnerError::Spawn(e.to_string())),
            // kill_all reaped + removed our entry already; the group is dead.
            None => Err(RunnerError::Other("spawn aborted: kill in progress".into())),
        };
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
        // Take the pipe handles off the child FIRST (the registry owns the Child).
        use std::io::Read;
        let stderr_handle = child.stderr.take().map(|mut e| {
            std::thread::spawn(move || {
                let mut buf = Vec::new();
                let _ = e.read_to_end(&mut buf);
                buf
            })
        });
        let mut stdout_pipe = child.stdout.take();
        // Register the owned child under the lock. On a latched register reclaim
        // it via take_child to self-kill (chat is exempt from a workers-only
        // latch — see LH5; for now an All-scope latch self-kills it).
        match registry.register_child(pgid, child) {
            RegisterDecision::Registered => {}
            RegisterDecision::KillImmediately => {
                #[cfg(unix)]
                unsafe {
                    libc::kill(-pgid, libc::SIGKILL);
                }
                if let Some(mut c) = registry.take_child(pgid) {
                    let _ = c.wait();
                }
                return Err(ChatError::Spawn("spawn aborted: kill in progress".into()));
            }
        }
        let mut stdout = Vec::new();
        if let Some(mut o) = stdout_pipe.take() {
            let _ = o.read_to_end(&mut stdout);
        }
        let stderr = stderr_handle
            .map(|h| h.join().unwrap_or_default())
            .unwrap_or_default();
        let status = match registry.take_child(pgid) {
            Some(mut c) => c.wait().map_err(|e| ChatError::Spawn(e.to_string())),
            None => Err(ChatError::Spawn("spawn aborted: kill in progress".into())),
        };
        let status = status?;
        llm_chat::claude_cli::interpret_chat_output(stdout, stderr, status.success())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_under_a_set_latch_returns_kill_immediately() {
        let reg = ProcessRegistry::new();
        reg.begin_killing(); // latch on
        // A spawner that registers while the latch is set is told to self-kill.
        assert!(matches!(reg.register_pgid(4321), RegisterDecision::KillImmediately));
        assert_eq!(reg.len(), 0, "a latched register must not retain the pgid");
    }

    #[test]
    fn register_clears_to_registered_after_latch_off() {
        let reg = ProcessRegistry::new();
        reg.begin_killing();
        reg.end_killing(); // latch off (brake-off / resume)
        assert!(matches!(reg.register_pgid(99), RegisterDecision::Registered));
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn kill_all_leaves_the_latch_set_so_stragglers_self_kill() {
        let reg = ProcessRegistry::new();
        reg.kill_all(); // empty sweep, but the latch must now be set
        assert!(
            matches!(reg.register_pgid(7), RegisterDecision::KillImmediately),
            "after kill_all the latch is held until end_killing()"
        );
    }

    #[cfg(unix)]
    #[test]
    fn spawner_self_kills_when_latch_is_set_and_does_not_register() {
        let reg = Arc::new(ProcessRegistry::new());
        reg.begin_killing(); // a Stop already happened; this claim raced past the gate
        // Spawn a child that would live 30s if it escaped.
        let out = (super::killable_spawn(&reg))(
            &["sh".into(), "-c".into(), "sleep 30".into()],
            None,
        );
        // The spawner must have killed its own child and surfaced a failure.
        assert!(out.is_err(), "a latched spawn must not succeed");
        assert!(reg.is_empty(), "the self-killed child is never registered");
    }

    #[cfg(unix)]
    #[test]
    fn reaping_takes_the_child_out_before_wait_so_kill_all_skips_it() {
        // A child that exits immediately; the spawner reaps it. After the spawner
        // returns, the registry holds NO live Child for that pgid, so a later
        // kill_all has nothing to signal (the pgid is unreachable, not recycled).
        let reg = Arc::new(ProcessRegistry::new());
        let out = (super::killable_spawn(&reg))(
            &["sh".into(), "-c".into(), "printf done".into()],
            None,
        );
        assert_eq!(out.unwrap(), "done");
        assert!(reg.is_empty(), "reaped child removed from the registry");
        // kill_all over the now-empty registry is a no-op and signals nothing.
        reg.kill_all();
        assert!(reg.is_empty());
    }

    #[cfg(unix)]
    #[test]
    #[allow(clippy::zombie_processes)] // kill_all reaps the owned Child
    fn register_racing_kill_all_never_lets_a_child_escape() {
        use std::os::unix::process::CommandExt;
        use std::process::Command;
        use std::time::{Duration, Instant};
        let reg = Arc::new(ProcessRegistry::new());
        // Spawn a long-lived child OURSELVES in its own group, then register it.
        let child = Command::new("sh")
            .arg("-c")
            .arg("sleep 30 & wait")
            .process_group(0)
            .spawn()
            .expect("spawn");
        let pgid = child.id() as i32;
        let dec = reg.register_child(pgid, child);
        assert!(matches!(dec, RegisterDecision::Registered));
        reg.kill_all();
        assert!(reg.is_empty(), "registry drained");
        let alive = |p: i32| unsafe { libc::kill(-p, 0) == 0 };
        let deadline = Instant::now() + Duration::from_secs(3);
        while alive(pgid) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(!alive(pgid), "the registered child must be dead after kill_all");
    }

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

    /// Run `f` on a thread and require it to finish within `timeout`, else fail —
    /// turns a true deadlock into a deterministic test failure rather than a
    /// stalled runner. Used by the concurrent-drain regression tests below.
    #[cfg(unix)]
    fn assert_completes_within<T: Send + 'static>(
        timeout: std::time::Duration,
        f: impl FnOnce() -> T + Send + 'static,
    ) -> T {
        use std::sync::mpsc;
        let (tx, rx) = mpsc::channel();
        let handle = std::thread::spawn(move || {
            let _ = tx.send(f());
        });
        match rx.recv_timeout(timeout) {
            Ok(v) => {
                handle.join().expect("worker thread panicked");
                v
            }
            Err(_) => panic!(
                "spawner deadlocked: did not complete within {timeout:?} \
                 (child wrote a large STDERR payload before STDOUT EOF)"
            ),
        }
    }

    /// Regression for the pipe-buffer deadlock: a child that writes >128KB to
    /// STDERR before STDOUT reaches EOF blocks the sequential stdout-then-stderr
    /// drain forever (the child blocks writing stderr past the ~64KB pipe buffer
    /// while the parent blocks reading stdout). The concurrent drain must finish
    /// and still capture STDOUT.
    #[cfg(unix)]
    #[test]
    fn killable_spawn_does_not_deadlock_on_large_stderr() {
        let reg = Arc::new(ProcessRegistry::new());
        let out = assert_completes_within(std::time::Duration::from_secs(20), move || {
            (super::killable_spawn(&reg))(
                &[
                    "sh".into(),
                    "-c".into(),
                    // ~200KB to stderr, a small known payload to stdout last.
                    "yes x | head -c 200000 1>&2; printf hello".into(),
                ],
                None,
            )
        });
        assert_eq!(out.unwrap(), "hello");
    }

    /// Same deadlock regression for the chat spawner. The chat spawner forces the
    /// `claude` binary as argv[0], so drive it through a fake binary on PATH.
    #[cfg(unix)]
    #[test]
    fn killable_chat_spawn_does_not_deadlock_on_large_stderr() {
        // Stage a fake `claude` that floods stderr then prints a stdout payload.
        let dir = std::env::temp_dir().join(format!("abtest-claude-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("claude");
        std::fs::write(
            &bin,
            "#!/bin/sh\nyes x | head -c 200000 1>&2\nprintf '{\"result\":\"hello\"}'\n",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        // CLAUDE_BIN is resolved via PATH lookup; prepend our staging dir.
        let old_path = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("{}:{}", dir.display(), old_path));

        let reg = Arc::new(ProcessRegistry::new());
        let res = assert_completes_within(std::time::Duration::from_secs(20), move || {
            (super::killable_chat_spawn(&reg))(&["-p".into(), "hi".into()], None)
        });

        std::env::set_var("PATH", old_path);
        let _ = std::fs::remove_dir_all(&dir);
        // We only assert no-deadlock + a captured success here; the exact chat
        // payload parsing is covered by llm_chat's own tests.
        assert!(res.is_ok(), "chat spawn should succeed, got {res:?}");
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
        #[allow(clippy::zombie_processes)] // kill_all reaps the group; no direct wait
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
        #[allow(clippy::zombie_processes)] // kill_all reaps the group; no direct wait
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
