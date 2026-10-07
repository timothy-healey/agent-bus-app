//! Live check of scope enforcement against the installed `claude` (manual,
//! paid). An implementer scope with Remote git, in a throwaway repo whose
//! `origin` is a LOCAL bare repo, may push its branch; a force push is denied
//! by rule and reported.
//!
//! Run with a scratch directory that may be wiped:
//! `AGENT_BUS_LIVE_DIR=/path/to/scratch cargo test -p runners --test live_scope -- --ignored --nocapture`

use agent_bus_core::{DenialSource, Effort, OutputKind, PermissionMode, ToolGrant};
use runners::claude_cli::ClaudeCliRunner;
use runners::output::{InvocationRequest, Runner, RunnerError};
use runners::scope::{prepare, WorkerScope};
use runners::stream_json::output_contract;
use std::path::{Path, PathBuf};
use std::process::Command;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git").args(args).current_dir(dir).output().expect("git runs");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A repo on branch `feature-x` with one commit ahead of `main`, whose only
/// remote is a bare repo on disk.
fn scratch_repo(root: &Path) -> (PathBuf, PathBuf) {
    let _ = std::fs::remove_dir_all(root);
    std::fs::create_dir_all(root).unwrap();
    let origin = root.join("origin.git");
    let repo = root.join("repo");
    git(root, &["init", "-q", "--bare", origin.to_str().unwrap()]);
    git(root, &["init", "-q", "-b", "main", repo.to_str().unwrap()]);
    git(&repo, &["config", "user.email", "probe@example.invalid"]);
    git(&repo, &["config", "user.name", "probe"]);
    std::fs::write(repo.join("README.md"), "hello\n").unwrap();
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    git(&repo, &["remote", "add", "origin", origin.to_str().unwrap()]);
    git(&repo, &["push", "-q", "origin", "main"]);
    git(&repo, &["checkout", "-q", "-b", "feature-x"]);
    std::fs::write(repo.join("README.md"), "hello\nchange\n").unwrap();
    git(&repo, &["commit", "-q", "-am", "feature change"]);
    (repo, origin)
}

fn request(root: &Path, repo: &Path, user_message: &str) -> InvocationRequest {
    let artifacts = root.join("artifacts/implementers");
    std::fs::create_dir_all(&artifacts).unwrap();
    let scope = WorkerScope {
        reads: vec![root.join("artifacts").to_string_lossy().into_owned()],
        writes: vec![repo.to_string_lossy().into_owned(), artifacts.to_string_lossy().into_owned()],
        grants: vec![ToolGrant::Bash, ToolGrant::Agent, ToolGrant::RemoteGit],
        repo_visibility: Some("private".into()),
    };
    let ss = prepare(root, "implementers", "T-live", 1, &scope).unwrap();
    InvocationRequest {
        task_id: "T-live".into(),
        team_id: "implementers".into(),
        model: "sonnet".into(),
        effort: Effort::Level("low".into()),
        system_prompt: format!(
            "You are an implementer working in this git repository.\n\n{}",
            output_contract(OutputKind::Producer, &artifacts.to_string_lossy(), &[])
        ),
        user_message: user_message.into(),
        settings_path: ss.settings_path.to_string_lossy().into_owned(),
        add_dirs: ss.add_dirs,
        permission_mode: PermissionMode::Auto,
        disallowed_tools: ss.disallowed_tools,
        plugin_dirs: vec![],
        working_dir: Some(repo.to_string_lossy().into_owned()),
        output_kind: OutputKind::Producer,
    }
}

fn live_dir() -> Option<PathBuf> {
    std::env::var_os("AGENT_BUS_LIVE_DIR").map(PathBuf::from)
}

#[tokio::test]
#[ignore = "runs the real claude CLI (paid); needs AGENT_BUS_LIVE_DIR"]
async fn an_implementer_with_remote_git_pushes_its_branch() {
    let Some(base) = live_dir() else { return };
    let root = base.join("push");
    let (repo, origin) = scratch_repo(&root);
    let req = request(
        &root,
        &repo,
        "Push the current branch feature-x to origin with: git push origin feature-x . Then report with no artifact.",
    );
    let out = ClaudeCliRunner::new().invoke(&req).await;
    println!("{out:?}");
    let branches = git(&origin, &["branch", "--list"]);
    assert!(branches.contains("feature-x"), "origin branches: {branches}");
    let out = out.expect("the run settles");
    assert!(out.permission_denials.is_empty(), "{:?}", out.permission_denials);
}

#[tokio::test]
#[ignore = "runs the real claude CLI (paid); needs AGENT_BUS_LIVE_DIR"]
async fn a_force_push_is_denied_by_rule_and_reported() {
    let Some(base) = live_dir() else { return };
    let root = base.join("force");
    let (repo, origin) = scratch_repo(&root);
    let req = request(
        &root,
        &repo,
        "Run exactly: git push --force origin feature-x . If it is refused, do not retry and do not push any other way; report with no artifact.",
    );
    let res = ClaudeCliRunner::new().invoke(&req).await;
    println!("{res:?}");
    let denials = match &res {
        Ok(o) => o.permission_denials.clone(),
        Err(e @ RunnerError::NoStructuredOutput { .. }) => e.denials().to_vec(),
        Err(e) => panic!("unexpected failure: {e:?}"),
    };
    let forced = denials
        .iter()
        .find(|d| d.tool_input["command"].as_str().is_some_and(|c| c.contains("--force")))
        .expect("the force push is reported as a denial");
    assert_eq!(forced.source, DenialSource::Rule);
    let branches = git(&origin, &["branch", "--list"]);
    assert!(!branches.contains("feature-x"), "nothing was pushed: {branches}");
}
