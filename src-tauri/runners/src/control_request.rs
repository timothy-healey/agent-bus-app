//! One SDK control request to the `claude` CLI, answered without calling a
//! model. The shared spawn behind the `get_usage` and `initialize` queries.
//!
//! `claude` runs with `--setting-sources=` and `--strict-mcp-config` from the
//! system temp dir, so it never loads the operator's settings, hooks or MCP
//! servers. Valued flags use the `=` form so an empty value cannot swallow the
//! next argument. The child leads its own process group; the whole group is
//! killed once the `control_response` line arrives, or at the deadline.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

pub const CONTROL_ARGS: &[&str] = &[
    "-p",
    "--input-format",
    "stream-json",
    "--output-format",
    "stream-json",
    "--verbose",
    "--no-session-persistence",
    "--setting-sources=",
    "--strict-mcp-config",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlError {
    Spawn(String),
    TimedOut,
}

/// SIGKILL `claude` and everything it started (MCP servers, hook shells): on
/// unix the child leads its own process group, so the whole group is signalled.
pub(crate) fn kill_group(child: &mut std::process::Child) {
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    #[cfg(not(unix))]
    {
        let _ = child.kill();
    }
}

/// Reap the child, killing its group if it is still running at the deadline.
fn bounded_reap(child: &mut std::process::Child, deadline: Instant) {
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) if Instant::now() >= deadline => {
                kill_group(child);
                let _ = child.wait();
                return;
            }
            Ok(None) => thread::sleep(Duration::from_millis(30)),
            Err(_) => return,
        }
    }
}

fn is_control_response(line: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(line)
        .map(|v| v["type"] == "control_response")
        .unwrap_or(false)
}

/// Send `request_line` to `claude` and return its stdout up to and including
/// the first `control_response` line (or everything up to EOF if none comes).
pub fn run(bin: &str, request_line: &str, timeout: Duration) -> Result<String, ControlError> {
    let mut cmd = Command::new(bin);
    cmd.args(CONTROL_ARGS)
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0); // claude leads a fresh group; pgid == child pid
    }
    let mut child = cmd.spawn().map_err(|e| ControlError::Spawn(e.to_string()))?;

    // Read stdout on its own thread, line by line, so a full pipe never blocks
    // the child and the caller can stop at the response line.
    let (tx, rx) = mpsc::channel::<String>();
    if let Some(stdout) = child.stdout.take() {
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
    }

    // Send the request and close stdin to signal EOF.
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(request_line.as_bytes());
    }

    let deadline = Instant::now() + timeout;
    let mut out = String::new();
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(remaining) {
            Ok(line) => {
                let done = is_control_response(&line);
                out.push_str(&line);
                out.push('\n');
                if done {
                    kill_group(&mut child);
                    bounded_reap(&mut child, deadline);
                    return Ok(out);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                kill_group(&mut child);
                bounded_reap(&mut child, deadline);
                return Err(ControlError::TimedOut);
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                bounded_reap(&mut child, deadline);
                return Ok(out);
            }
        }
    }
}
