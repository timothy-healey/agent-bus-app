//! The target repo's visibility (public, private, internal), for the auto-mode
//! classifier's context. Read once per run with `gh repo view`; any failure
//! (no `gh`, no remote, not authenticated, timeout) gives `None`.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How long the `gh` lookup may take before it is abandoned.
pub const LOOKUP_TIMEOUT: Duration = Duration::from_secs(10);

/// Parse `gh repo view --json visibility -q .visibility` output.
pub fn parse_visibility(stdout: &str) -> Option<String> {
    let v = stdout.trim().to_ascii_lowercase();
    match v.as_str() {
        "public" | "private" | "internal" => Some(v),
        _ => None,
    }
}

/// Look up `repo`'s visibility with `gh`.
pub fn repo_visibility(repo: &Path) -> Option<String> {
    let mut child = Command::new("gh")
        .args(["repo", "view", "--json", "visibility", "-q", ".visibility"])
        .current_dir(repo)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + LOOKUP_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) => return None,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(_) => return None,
        }
    }
    let mut out = String::new();
    use std::io::Read;
    child.stdout.take()?.read_to_string(&mut out).ok()?;
    parse_visibility(&out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visibility_is_lowercased_and_unknown_values_are_none() {
        assert_eq!(parse_visibility("PUBLIC\n"), Some("public".into()));
        assert_eq!(parse_visibility("private"), Some("private".into()));
        assert_eq!(parse_visibility("INTERNAL"), Some("internal".into()));
        assert_eq!(parse_visibility(""), None);
        assert_eq!(parse_visibility("could not resolve"), None);
    }

    #[test]
    fn a_directory_that_is_not_a_repo_gives_none() {
        let dir = std::env::temp_dir().join(format!("abp-vis-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(repo_visibility(&dir), None);
    }
}
