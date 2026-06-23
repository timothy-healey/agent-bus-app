# S3 — Sandbox hardening (`sandbox-exec`) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Generate a macOS `sandbox-exec` (SBPL) profile from a team's **Scope** and, behind a default-OFF experimental flag, wrap the `claude-cli` subprocess argv in `sandbox-exec -p <profile>` to OS-confine the worker.

**Architecture:** The Scope→profile mapping is a **pure function** in the Runners ACL Scope policy module (`runners/src/scope.rs`), alongside the existing `settings.json` generation — it is the same Scope-policy bounded responsibility, just a second projection of the same Scope. The SBPL/`sandbox-exec` idiom stays **sealed inside the ACL**: it never leaks past the `Runner` trait. The `ClaudeCliRunner` gains an optional sandbox-wrap step applied to argv at spawn time; the pool decides whether to populate the profile based on a config flag that defaults OFF. Live `sandbox-exec` confinement is **NOT** invoked or verified in tests — only profile-generation and argv-wrapping are tested.

**Tech Stack:** Rust (cargo workspace under `src-tauri/`), `pipeline::model::Scope`, `workspace::paths::{resolve, PathVars}`, the existing `SpawnFn` seam in `runners/src/claude_cli.rs`.

---

## Decisions

Every fork auto-decided with the recommended option (operator AFK).

- **D1 — Profile-gen lives in `runners/src/scope.rs`.** The Scope→`settings.json` projection already lives there; the Scope→SBPL projection is the same Scope-policy responsibility (a second on-disk artifact derived from the same `reads`/`writes`/`tools`). No new crate, no new bounded context. (Confirmed against the existing `build_settings` shape.)
- **D2 — The SBPL idiom never crosses the `Runner` trait.** The profile string is computed inside the Runners ACL and consumed inside `ClaudeCliRunner`. We carry it on `InvocationRequest` as an `Option<String>` field named `sandbox_profile` — `InvocationRequest` is already a Runners-ACL-owned struct (it holds `settings_path`, `add_dirs` — CLI-shaped data), so adding a CLI-shaped optional profile to it does NOT leak past the ACL. Runtime/pool only learns "sandbox: on/off" (a bool config), never any SBPL text. The AnthropicApiRunner ignores the field (sandbox-exec is a subprocess-only concept) — honest: API runner has no subprocess to confine.
- **D3 — Wrapping is argv-prefix, applied in `ClaudeCliRunner::run_once`.** When `req.sandbox_profile` is `Some(p)`, the spawned argv becomes `["sandbox-exec", "-p", p, CLAUDE_BIN, ...build_args(req)]` instead of `[CLAUDE_BIN, ...]`. The default (production) `SpawnFn` already prepends `CLAUDE_BIN`; we refactor so the program name is part of the argv the `SpawnFn` receives, making the wrap a pure argv transform that the fake spawner asserts. This keeps live confinement behind the existing seam.
- **D4 — Flag default OFF, lives with existing config.** A new field on the pipeline-level config the pool already threads (`PoolContext`) — `sandbox: bool`, default `false`. When false (default), `sandbox_profile` is `None` and behavior is byte-for-byte unchanged. No Settings UI toggle in this item (experimental; keep minimal — a `TODO(s3)` notes the optional future toggle).
- **D5 — SBPL content: deny-by-default, allow scope paths + system essentials.** `(version 1) (deny default)`, allow `process-exec`/`process-fork` (claude needs to run), allow `file-read*` on system essentials (`/usr/lib`, `/usr/bin`, `/System`, `/private/var`, `/dev`, dyld cache) + the resolved read/write dirs, allow `file-write*` only on the resolved write dirs + tmp, and a network policy keyed on tools: deny network unless the team's tools include a network-capable tool (`WebFetch`/`WebSearch`), else `(allow network*)`. Paths are emitted as `(subpath "...")` with quote-escaping.
- **D6 — Honesty / scope of verifiability (LOUD).** `sandbox-exec` is **Apple-deprecated** (since macOS 10.14, still functional through current macOS), **macOS-only**, and this feature is **opt-in + experimental**. The **only** verifiable parts here are (a) profile-string generation and (b) argv-wrapping — both pure and unit-tested via the fake spawner. **Actually confining a process is NOT verified** in this environment (no live `claude`, security-sensitive, cannot be proven headlessly). We do **NOT** claim a proven security boundary. This caveat is repeated in DOMAIN/spec comments, the backlog entry, and the report.

---

## File Structure

- **Modify** `src-tauri/runners/src/scope.rs` — add the pure `sandbox_profile(scope, vars) -> Result<String, ScopeError>` function + SBPL escaping helper + unit tests. This is the load-bearing, fully-testable deliverable.
- **Modify** `src-tauri/runners/src/output.rs` — add `sandbox_profile: Option<String>` to `InvocationRequest` (CLI-shaped, ACL-owned).
- **Modify** `src-tauri/runners/src/command.rs` — no behavioral change to `build_args`; add a thin `sandbox_wrap(profile, argv) -> argv` pure helper here (argv transform is command-construction's responsibility) + tests.
- **Modify** `src-tauri/runners/src/claude_cli.rs` — production `SpawnFn` now receives a full argv *including* the program; `run_once` builds `[CLAUDE_BIN] + build_args`, optionally `sandbox_wrap`s it, then spawns. Update tests.
- **Modify** `src-tauri/runners/src/fake.rs` and any other `InvocationRequest { ... }` construction sites (`runtime/src/pool.rs`, `runners/src/anthropic_api.rs` tests, `runners/src/command.rs` tests, `runners/src/claude_cli.rs` tests) — add `sandbox_profile: None`.
- **Modify** `src-tauri/runtime/src/pool.rs` — add `sandbox: bool` to `PoolContext` (default-off threaded from the root); when true, compute the profile via `scope::sandbox_profile` and set `req.sandbox_profile`. Add a test proving default-off yields `None` and on yields `Some`.
- **Modify** `src-tauri/app/src/lib.rs` — thread the flag (hardcoded `false` for now; `TODO(s3)` for config/Settings wiring) into `PoolContext`.
- **Modify** `src-tauri/runners/src/lib.rs` — extend the module-doc to note the sandbox projection + the experimental/deprecated/macOS-only/structural-only caveat (DOMAIN-as-doc-comment, the repo's convention).

---

### Task 1: Pure SBPL profile generation in the Scope policy

**Files:**
- Modify: `src-tauri/runners/src/scope.rs`
- Test: `src-tauri/runners/src/scope.rs` (`#[cfg(test)] mod tests`)

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `scope.rs`:

```rust
#[test]
fn sandbox_profile_denies_by_default_and_allows_scope_paths() {
    let vars = PathVars::new("/proj").with_target_repo("/repo");
    let p = sandbox_profile(&demo_scope(), &vars).unwrap();
    // deny-by-default header
    assert!(p.starts_with("(version 1)"));
    assert!(p.contains("(deny default)"));
    // process exec/fork allowed (claude must run)
    assert!(p.contains("(allow process-exec)"));
    assert!(p.contains("(allow process-fork)"));
    // read access to the resolved read dirs
    assert!(p.contains("(allow file-read*"));
    assert!(p.contains("(subpath \"/repo\")"));
    assert!(p.contains("(subpath \"/proj/artifacts/analyses\")"));
    // write access only to the resolved write dirs
    assert!(p.contains("(allow file-write*"));
    // system essentials are readable
    assert!(p.contains("(subpath \"/usr/lib\")"));
    assert!(p.contains("(subpath \"/System\")"));
}

#[test]
fn sandbox_profile_denies_network_unless_a_network_tool_is_present() {
    let vars = PathVars::new("/proj").with_target_repo("/repo");
    // demo_scope() has Read/Write/Bash(git commit) — no network tool
    let p = sandbox_profile(&demo_scope(), &vars).unwrap();
    assert!(p.contains("(deny network*)"));
    assert!(!p.contains("(allow network*)"));

    let mut net = demo_scope();
    net.tools.push("WebFetch".into());
    let p2 = sandbox_profile(&net, &vars).unwrap();
    assert!(p2.contains("(allow network*)"));
    assert!(!p2.contains("(deny network*)"));
}

#[test]
fn sandbox_profile_escapes_quotes_and_backslashes_in_paths() {
    let mut scope = Scope { reads: vec!["${project}/a\"b".into()], writes: vec![], tools: vec![] };
    scope.writes.clear();
    let vars = PathVars::new("/proj");
    let p = sandbox_profile(&scope, &vars).unwrap();
    // an embedded quote must be backslash-escaped inside the SBPL string literal
    assert!(p.contains("/proj/a\\\"b"));
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src-tauri && cargo test -p runners scope::tests::sandbox 2>&1 | tail -20`
Expected: FAIL — `cannot find function sandbox_profile in this scope`.

- [ ] **Step 3: Implement the pure profile generator**

Add to `scope.rs` (after `build_settings`):

```rust
/// Tool patterns that imply the worker needs outbound network access. When none
/// of the team's tools match, the generated sandbox profile denies network.
const NETWORK_TOOLS: &[&str] = &["WebFetch", "WebSearch"];

/// System paths a confined `claude` subprocess must still read to start and run
/// (dynamic linker, shared libs, system frameworks, devices). Deliberately
/// read-only.
const SYSTEM_READ_SUBPATHS: &[&str] = &[
    "/usr/lib",
    "/usr/bin",
    "/usr/share",
    "/System",
    "/Library",
    "/private/var",
    "/dev",
    "/etc",
    "/bin",
    "/sbin",
];

/// Escape a path for inclusion in an SBPL double-quoted string literal.
fn sbpl_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// **EXPERIMENTAL · macOS-only · STRUCTURAL-ONLY.** Generate a macOS
/// `sandbox-exec` (SBPL) profile string from a team Scope.
///
/// This is a *pure* projection of the same Scope that `build_settings` projects
/// to `settings.json` — a deny-by-default profile that allows reads/writes only
/// on the team's resolved scope paths (plus system essentials needed to run
/// `claude`), and grants network only when the team holds a network-capable tool.
///
/// IMPORTANT HONESTY CAVEAT: `sandbox-exec` is Apple-DEPRECATED (still functional
/// on current macOS) and this confinement is **unverified** in CI — only this
/// string generation is unit-tested. Wrapping a real subprocess in
/// `sandbox-exec -p <this>` is opt-in/experimental and is NOT a proven security
/// boundary. The SBPL idiom is sealed inside the Runners ACL.
pub fn sandbox_profile(scope: &Scope, vars: &PathVars) -> Result<String, ScopeError> {
    let mut reads: Vec<String> = Vec::new();
    for pat in scope.reads.iter() {
        reads.push(resolve(pat, vars)?);
    }
    let mut writes: Vec<String> = Vec::new();
    for pat in scope.writes.iter() {
        writes.push(resolve(pat, vars)?);
    }
    // Writable dirs are also readable.
    reads.extend(writes.iter().cloned());
    reads.sort();
    reads.dedup();
    writes.sort();
    writes.dedup();

    let net_allowed = scope
        .tools
        .iter()
        .any(|t| NETWORK_TOOLS.iter().any(|n| t.starts_with(n)));

    let mut p = String::new();
    p.push_str("(version 1)\n");
    p.push_str(";; EXPERIMENTAL macOS sandbox-exec profile — generated from team Scope.\n");
    p.push_str(";; sandbox-exec is Apple-deprecated; this is opt-in and NOT a proven boundary.\n");
    p.push_str("(deny default)\n");
    p.push_str("(allow process-exec)\n");
    p.push_str("(allow process-fork)\n");
    p.push_str("(allow sysctl-read)\n");
    p.push_str("(allow mach-lookup)\n");

    // System essentials: read-only.
    p.push_str("(allow file-read*\n");
    for sp in SYSTEM_READ_SUBPATHS {
        p.push_str(&format!("  (subpath \"{}\")\n", sbpl_escape(sp)));
    }
    // Scope read dirs (includes write dirs, which are also readable).
    for d in &reads {
        p.push_str(&format!("  (subpath \"{}\")\n", sbpl_escape(d)));
    }
    p.push_str(")\n");

    // Scope write dirs + tmp.
    p.push_str("(allow file-write*\n");
    p.push_str("  (subpath \"/private/tmp\")\n");
    p.push_str("  (subpath \"/private/var/folders\")\n");
    for d in &writes {
        p.push_str(&format!("  (subpath \"{}\")\n", sbpl_escape(d)));
    }
    p.push_str(")\n");

    if net_allowed {
        p.push_str("(allow network*)\n");
    } else {
        p.push_str("(deny network*)\n");
    }

    Ok(p)
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test -p runners scope::tests::sandbox 2>&1 | tail -20`
Expected: PASS (3 sandbox tests).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/runners/src/scope.rs
git commit -m "feat(s3): pure Scope->sandbox-exec SBPL profile generation

EXPERIMENTAL macOS-only; sandbox-exec is Apple-deprecated. Pure, unit-tested
projection of the same Scope that build_settings projects to settings.json.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: Pure argv-wrapping helper in command construction

**Files:**
- Modify: `src-tauri/runners/src/command.rs`
- Test: `src-tauri/runners/src/command.rs` (`tests` module)

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `command.rs`:

```rust
#[test]
fn sandbox_wrap_prefixes_sandbox_exec_and_preserves_argv() {
    let inner = vec!["claude".to_string(), "--print".to_string(), "hi".to_string()];
    let wrapped = sandbox_wrap("(version 1)(deny default)", &inner);
    assert_eq!(wrapped[0], "sandbox-exec");
    assert_eq!(wrapped[1], "-p");
    assert_eq!(wrapped[2], "(version 1)(deny default)");
    // the original argv follows verbatim
    assert_eq!(&wrapped[3..], &inner[..]);
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd src-tauri && cargo test -p runners command::tests::sandbox_wrap 2>&1 | tail -20`
Expected: FAIL — `cannot find function sandbox_wrap`.

- [ ] **Step 3: Implement the helper**

Add to `command.rs` (after `build_args`):

```rust
/// The macOS sandbox launcher binary. Apple-deprecated but functional.
pub const SANDBOX_BIN: &str = "sandbox-exec";

/// **EXPERIMENTAL · macOS-only.** Wrap a full argv (program + args) in
/// `sandbox-exec -p <profile>` so the program runs confined by the given SBPL
/// profile. Pure argv transform — does NOT spawn anything. Live confinement is
/// unverified; this only constructs the command line.
pub fn sandbox_wrap(profile: &str, argv: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(argv.len() + 3);
    out.push(SANDBOX_BIN.to_string());
    out.push("-p".to_string());
    out.push(profile.to_string());
    out.extend(argv.iter().cloned());
    out
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd src-tauri && cargo test -p runners command::tests::sandbox_wrap 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/runners/src/command.rs
git commit -m "feat(s3): pure sandbox_wrap argv transform (sandbox-exec -p)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: Carry the profile on InvocationRequest (ACL-owned, CLI-shaped)

**Files:**
- Modify: `src-tauri/runners/src/output.rs`
- Modify: all `InvocationRequest { ... }` literals (compile-driven): `src-tauri/runners/src/command.rs` (test), `src-tauri/runners/src/claude_cli.rs` (test), `src-tauri/runners/src/output.rs` (test), `src-tauri/runners/src/fake.rs`, `src-tauri/runners/src/anthropic_api.rs` (test), `src-tauri/runtime/src/pool.rs` (real + tests)

- [ ] **Step 1: Add the field**

In `output.rs`, add to `InvocationRequest` (after `add_dirs`):

```rust
    /// **EXPERIMENTAL (S3) · macOS-only.** When `Some`, the CLI runner wraps the
    /// `claude` subprocess in `sandbox-exec -p <profile>`. CLI-shaped data owned
    /// by the Runners ACL — the SBPL idiom never crosses the `Runner` trait
    /// outward (Runtime only chooses on/off, never sees this string). `None`
    /// (the default) = unchanged behavior. The AnthropicApiRunner ignores it
    /// (no subprocess to confine).
    pub sandbox_profile: Option<String>,
```

- [ ] **Step 2: Run the build to find every construction site**

Run: `cd src-tauri && cargo check --workspace 2>&1 | grep "missing field" | sort -u`
Expected: errors listing each `InvocationRequest { ... }` literal missing `sandbox_profile`.

- [ ] **Step 3: Add `sandbox_profile: None` to every literal**

For each site reported in Step 2, add `sandbox_profile: None,` to the struct literal (these are all test fixtures + the pool's real request, which Task 5 will populate conditionally). Leave `fake.rs` and all test `req()` helpers at `None`.

- [ ] **Step 4: Run check + full tests to verify green**

Run: `cd src-tauri && cargo check --workspace 2>&1 | tail -5 && cargo test -p runners 2>&1 | tail -5`
Expected: check clean; runners tests PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/runners/src/output.rs src-tauri/runners/src/fake.rs src-tauri/runners/src/anthropic_api.rs src-tauri/runners/src/command.rs src-tauri/runners/src/claude_cli.rs src-tauri/runtime/src/pool.rs
git commit -m "feat(s3): add InvocationRequest.sandbox_profile (ACL-owned, default None)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: Wire the wrap into ClaudeCliRunner via the SpawnFn seam

**Files:**
- Modify: `src-tauri/runners/src/claude_cli.rs`

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `claude_cli.rs`:

```rust
#[tokio::test]
async fn spawn_argv_starts_with_claude_bin_when_no_sandbox() {
    let canned = r#"{"type":"result","is_error":false,"result":"VERDICT: approve","usage":{"input_tokens":1,"output_tokens":1}}"#;
    let runner = ClaudeCliRunner::with_spawner(Box::new(move |args| {
        // program name is now the first argv element handed to the SpawnFn
        assert_eq!(args[0], crate::command::CLAUDE_BIN);
        assert!(args.iter().any(|a| a == "--print"));
        assert!(!args.iter().any(|a| a == "sandbox-exec"));
        Ok(canned.to_string())
    }));
    runner.invoke(&req()).await.unwrap();
}

#[tokio::test]
async fn spawn_argv_is_sandbox_wrapped_when_profile_present() {
    let canned = r#"{"type":"result","is_error":false,"result":"VERDICT: approve","usage":{"input_tokens":1,"output_tokens":1}}"#;
    let runner = ClaudeCliRunner::with_spawner(Box::new(move |args| {
        assert_eq!(args[0], "sandbox-exec");
        assert_eq!(args[1], "-p");
        assert_eq!(args[2], "(version 1)(deny default)");
        assert_eq!(args[3], crate::command::CLAUDE_BIN);
        assert!(args.iter().any(|a| a == "--print"));
        Ok(canned.to_string())
    }));
    let mut r = req();
    r.sandbox_profile = Some("(version 1)(deny default)".into());
    runner.invoke(&r).await.unwrap();
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src-tauri && cargo test -p runners claude_cli::tests::spawn_argv 2>&1 | tail -20`
Expected: FAIL — the production/test SpawnFn currently receives argv WITHOUT the program name, so `args[0] == CLAUDE_BIN` fails.

- [ ] **Step 3: Refactor so the SpawnFn receives the full argv including the program, and wrap when a profile is present**

Replace the production `SpawnFn` in `ClaudeCliRunner::new` (the closure now treats `args[0]` as the program):

```rust
    pub fn new() -> Self {
        Self {
            spawn: Box::new(|args: &[String]| {
                // args[0] is the program (CLAUDE_BIN, or sandbox-exec when wrapped).
                let (program, rest) = args
                    .split_first()
                    .ok_or_else(|| RunnerError::Spawn("empty argv".into()))?;
                let output = std::process::Command::new(program)
                    .args(rest)
                    .output()
                    .map_err(|e| RunnerError::Spawn(e.to_string()))?;
                Ok(String::from_utf8_lossy(&output.stdout).into_owned())
            }),
        }
    }
```

Replace `run_once` so it prepends the program and optionally wraps:

```rust
    fn run_once(
        &self,
        req: &InvocationRequest,
        forward: &mut dyn FnMut(&str),
    ) -> Result<RunnerOutput, RunnerError> {
        let mut argv = vec![CLAUDE_BIN.to_string()];
        argv.extend(build_args(req));
        // S3 (EXPERIMENTAL · macOS-only): when a sandbox profile is present, wrap
        // the whole argv in `sandbox-exec -p <profile>`. Default (None) = the
        // plain `claude` argv, unchanged. Live confinement is UNVERIFIED here.
        if let Some(profile) = &req.sandbox_profile {
            argv = crate::command::sandbox_wrap(profile, &argv);
        }
        let stdout = (self.spawn)(&argv)?;
        parse_stream_streaming(&stdout, &req.model, forward)
    }
```

Note: the existing test `invoke_parses_canned_stdout_without_a_real_binary` asserts `args.contains(&"--print")` — still true (the program is prepended, `--print` is still in the vec). No change needed there.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test -p runners claude_cli 2>&1 | tail -20`
Expected: PASS — all claude_cli tests including the two new spawn_argv tests.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/runners/src/claude_cli.rs
git commit -m "feat(s3): sandbox-wrap the claude argv behind the SpawnFn seam

When InvocationRequest.sandbox_profile is Some, the spawned argv is
sandbox-exec -p <profile> claude ... Default None = unchanged. Live
confinement is NOT exercised in tests (security-sensitive, unverifiable here).

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 5: Flag-gate profile generation in the pool (default OFF)

**Files:**
- Modify: `src-tauri/runtime/src/pool.rs`
- Modify: `src-tauri/app/src/lib.rs`

- [ ] **Step 1: Write the failing test**

In `pool.rs` tests, locate the `PoolContext` builder used by existing tests (e.g. around the `process_one_claim` tests) and add a test that asserts the request carries a profile when `sandbox` is on. Use a capturing fake runner (the test module already has runners that capture the `InvocationRequest`). Add:

```rust
#[tokio::test]
async fn sandbox_flag_off_leaves_profile_none_on_default() {
    // Build a ctx with sandbox = false (the default) and a capturing runner;
    // assert the InvocationRequest seen by the runner has sandbox_profile == None.
    // (Mirror the existing process_one_claim test setup; set ctx.sandbox = false.)
}

#[tokio::test]
async fn sandbox_flag_on_sets_a_profile() {
    // Same setup with ctx.sandbox = true; assert the captured request's
    // sandbox_profile is Some and contains "(deny default)".
}
```

Fill these in concretely by copying the nearest existing capturing-runner test in the file and adjusting the `sandbox` field + the assertion on the captured request. (The existing tests show the exact `PoolContext { ... }` literal and capturing-runner pattern to mirror.)

- [ ] **Step 2: Run to verify it fails**

Run: `cd src-tauri && cargo test -p runtime sandbox_flag 2>&1 | tail -20`
Expected: FAIL — `PoolContext` has no `sandbox` field.

- [ ] **Step 3: Add the flag + conditional profile generation**

In `pool.rs`, add to the `PoolContext` struct:

```rust
    /// **EXPERIMENTAL (S3) · macOS-only.** When true, generate a `sandbox-exec`
    /// SBPL profile from the team Scope and confine the CLI worker subprocess.
    /// Default false (unchanged behavior). Runtime only flips this bool — the
    /// SBPL idiom is generated and consumed entirely inside the Runners ACL.
    /// Live confinement is UNVERIFIED; this is opt-in/experimental.
    pub sandbox: bool,
```

In `process_one_claim`, after building the `InvocationRequest` (or while building it), set the field:

```rust
        sandbox_profile: if ctx.sandbox {
            Some(runners::scope::sandbox_profile(&team.scope, &vars)?)
        } else {
            None
        },
```

(`vars` is already in scope from the PREPARE step; `ScopeError` already converts into the pool's error via the existing `?` on `prepare`.)

Add `sandbox: false,` to every `PoolContext { ... }` literal in the existing tests (compile-driven; the on-test sets `true`).

- [ ] **Step 4: Wire the default-off flag at the composition root**

In `app/src/lib.rs`, find the `PoolContext { ... }` construction in `spawn_worker_loops` and add:

```rust
            // S3: OS sandbox confinement is EXPERIMENTAL, macOS-only, and OFF by
            // default. TODO(s3): surface a config/Settings toggle once the live
            // sandbox-exec boundary has been validated on macOS.
            sandbox: false,
```

- [ ] **Step 5: Run to verify green**

Run: `cd src-tauri && cargo test -p runtime sandbox_flag 2>&1 | tail -20 && cargo check --workspace 2>&1 | tail -5`
Expected: both new tests PASS; workspace check clean.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/runtime/src/pool.rs src-tauri/app/src/lib.rs
git commit -m "feat(s3): flag-gate sandbox profile generation in the pool (default OFF)

PoolContext.sandbox (default false) decides whether the pool projects the team
Scope to an SBPL profile and confines the CLI worker. Off => unchanged. Root
hardcodes false with a TODO(s3) for the future config/Settings toggle.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 6: DOMAIN/module-doc caveat + final verification

**Files:**
- Modify: `src-tauri/runners/src/lib.rs`

- [ ] **Step 1: Extend the module doc with the honest caveat**

Append to the `//!` block in `runners/src/lib.rs`:

```rust
//!
//! S3 (EXPERIMENTAL · macOS-only · STRUCTURAL-ONLY): the Scope policy also
//! projects a team Scope to a macOS `sandbox-exec` (SBPL) profile
//! (`scope::sandbox_profile`); when enabled (default OFF) the ClaudeCliRunner
//! wraps the subprocess argv in `sandbox-exec -p <profile>`. The SBPL idiom is
//! sealed inside this ACL — it never crosses the `Runner` trait. `sandbox-exec`
//! is Apple-DEPRECATED (still functional). ONLY profile-generation + argv-
//! wrapping are tested; the live OS confinement is NOT a proven security
//! boundary in this codebase.
```

- [ ] **Step 2: Full workspace verification**

Run, expecting all green:

```bash
cd src-tauri && cargo test --workspace 2>&1 | tail -20
cd src-tauri && cargo clippy --workspace 2>&1 | tail -20
cd src-tauri && cargo check --workspace 2>&1 | tail -5
```

Then frontend (from repo root, PATH may need `export PATH=/opt/homebrew/bin:$PATH`):

```bash
bun vitest run 2>&1 | tail -15
bun run build 2>&1 | tail -15
```

Expected: cargo test all pass; clippy clean (no warnings); check clean; vitest pass; build succeeds.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/runners/src/lib.rs
git commit -m "docs(s3): note the sandbox projection + experimental/deprecated caveat in the ACL doc

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Self-Review

**Spec coverage:**
- Profile generation (Scope → SBPL) — Task 1. ✓ (pure, fully unit-tested — the load-bearing deliverable)
- Optional `sandbox-exec -p <profile>` wrap via SpawnFn — Tasks 3+4. ✓
- Opt-in + default OFF flag — Task 5. ✓
- macOS-only / deprecated / experimental noted loudly in plan, DOMAIN/doc, will be in backlog + report — Decisions D6, Task 1/6 doc comments. ✓
- SBPL idiom sealed inside the ACL (no leak past `Runner`) — D2, Task 3. ✓
- Live confinement NOT claimed as a security boundary — D6, every doc comment. ✓

**Placeholder scan:** Task 5's test bodies are described-by-mirror rather than fully spelled out because the exact `PoolContext { ... }` literal and capturing-runner shape must be copied verbatim from the adjacent existing test (spelling them blind risks drifting from the real fixture). The implementer copies the nearest existing test and changes only the `sandbox` field + the captured-request assertion. This is a deliberate, bounded instruction, not a "TODO".

**Type consistency:** `sandbox_profile(scope, vars) -> Result<String, ScopeError>` (Task 1) is called in Task 5 with `(&team.scope, &vars)`. `sandbox_wrap(&str, &[String]) -> Vec<String>` (Task 2) is called in Task 4 with `(profile, &argv)`. `InvocationRequest.sandbox_profile: Option<String>` (Task 3) is read in Task 4 and written in Task 5. `PoolContext.sandbox: bool` (Task 5) read in `process_one_claim`. Consistent.
