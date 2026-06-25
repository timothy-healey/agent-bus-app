# Candidate LF20-02 — Killable spawner closures (worker + chat runners)

## Delivery
Runtime lifecycle: clean exit + clean resume — spec `docs/superpowers/specs/2026-06-25-lifecycle-exit-resume-design.md` (§ Killable spawner — the seam).

## Location
- `src-tauri/app/src/lib.rs` — build the spawner closures and inject the registry.
- `src-tauri/app/src/pipeline_activator.rs:404` `runner_for(...)` and `:186` `runner_for_team(...)` — today build `ClaudeCliRunner::new()` (`:207`, `:411`); must instead build `ClaudeCliRunner::with_spawner(...)`.
- `src-tauri/app/src/pipeline_activator.rs:431` `chat_runner_for(...)` — the CLI branch builds `ClaudeChatRunner::new()` (`:438`); must build `ClaudeChatRunner::with_spawner(...)`.
- ACL seams already exist: `runners::claude_cli::ClaudeCliRunner::with_spawner` (`../runners/src/claude_cli.rs:73`) and `llm_chat::claude_cli::ClaudeChatRunner::with_spawner` (`../llm_chat/src/claude_cli.rs:74`).

## Why it is a candidate
The registry (LF20-01) is inert without a spawner that registers what it spawns. The spec routes both the worker `ClaudeCliRunner` and the chat `ClaudeChatRunner` through `with_spawner` so the child is spawned in its own process group and tracked. This is the behavioral heart of "kill in-flight work" — and it currently does not exist (the default path uses blocking `.output()` with no `Child`).

## Proposed change
A spawner closure capturing `registry.clone()` that:
1. Builds the `Command`, sets `.process_group(0)` (Unix `std::os::unix::process::CommandExt`) so the child leads a fresh group (pgid == child pid), pipes stdout/stderr.
2. `.spawn()` → `Child`; `registry.register(child.id())`.
3. Reads stdout/stderr to completion and `.wait()`s (replicating `.output()` capture; the streaming chat runner keeps forwarding deltas as today).
4. `registry.deregister(pgid)`; returns the same `interpret_runner_output(stdout, stderr, success)` the current closure returns.

`ClaudeCliRunner::new()`'s plain blocking `.output()` stays for tests/non-app use. The ACL trait surface is unchanged — only the root's spawner closure differs. Threading: the registry `Arc` must reach `PipelineActivator` (via `WorkerDeps` or a new field) so per-team runners are built killable, and reach `chat_runner_for` at the root.

## Tests (no live `claude`)
- A real `echo hello` (or `sh -c`) through the spawner returns the captured output AND registers-then-deregisters (registry empty afterward).

## Dependencies / sequencing
- **Depends on** LF20-01 (registry type + `register`/`deregister`).
- **Enables** LF20-03 / LF20-04 to actually have something to kill.
- Note the wiring cost: `runner_for`/`runner_for_team` are in `pipeline_activator.rs`; the registry must be added to `WorkerDeps` (or `PipelineActivator::new`) since runners are built per-team inside the activator, not at the root.
