# Spec — Session resume for revise loops

*Design doc. Wayfinder map: "Map: runner capabilities from the current Claude CLI" (#3). Ticket: "Spec: session resume for revise loops" (#13). Research: "Research: --resume, --session-id and --fork-session under --print" (#12, `docs/research/print-session-resume.md` on `research/print-session-resume`). When a reviewer sends an item back to the team that wrote it, the writer's next attempt forks the writer's own earlier session instead of starting cold. It keeps its context and its prompt cache. Attempts also accumulate across the whole revise loop, so the cap of 3 actually holds. Terms are as defined in `DOMAIN.md` (Runtime, Runners).*

## Why this exists

- **A revise starts cold.** The writer re-reads the repo, the spec and its own earlier output, then works out from scratch what changed.
- **Resuming is much cheaper.** In the research, a resumed revise wrote 3.4k cache tokens against 20.3k for a fresh run (about 65% less input-equivalent cost), needed one fewer request and didn't re-read files.
- **The attempts cap never trips.** `transform_once` creates downstream children with `attempts = 1` (`engine.rs:441-461`), so every gated item arrives at attempts 1 and every revise child at attempts 2. A writer–reviewer pair can loop forever, and the join's revise-once guard never trips after a re-fork.

What the CLI does (2.1.289):
- `--resume <id> --fork-session` copies the history into a new session and leaves the original unchanged.
- `--session-id <uuid>` pre-assigns a session's id. A reused id fails fast.
- On resume, `--append-system-prompt` is ignored and the first run's system prompt is replayed, so reviser instructions must go in the user message.
- Different `--settings` do apply on resume, but they change the cached prefix and so miss the cache.
- Resume works from any cwd, but the history holds absolute paths.

## Decisions

1. **Every worker invocation gets a pre-assigned session id.**
   - The engine mints a UUID and passes `--session-id=<uuid>`.
   - It records the id on the invocation's audit row, together with a **config fingerprint**: a hash of everything that shapes the session prefix.
     - Included: the system prompt (team prompt plus output contract), the settings file content, the model, effort, permission mode, `--json-schema`, disallowed tools, plugin dirs, add-dirs and the cwd.
     - Excluded: the user message and the session flags.
   - Nothing is parsed from the stream.
2. **A revise resumes when the writer would do the same work again.**
   - A revise child resumes when all of these hold:
     - the team receiving the revise is the team that wrote the item's current artifact;
     - that team's last successful invocation for the item, matched by `(run_id, item_key, team_id)`, has a session id;
     - the child's config fingerprint equals that invocation's.
   - It then passes `--resume=<writer session> --fork-session --session-id=<new uuid>`.
   - Otherwise it starts fresh, as today.
3. **Starting fresh is forced when:**
   - the revise goes to a different team (`on_revise` pointing elsewhere, or a join sending an item back to the producer before the fork);
   - the team's prompt, scope, plugins, model or effort changed since the writer ran, or the cwd differs (all caught by the fingerprint);
   - the writer had no successful invocation for this item;
   - the resume fails (decision 5).
4. **The user message on resume is the same as on a fresh run:** the input block (template-prompts spec, decision 1) plus the revision bundle. The replayed system prompt is identical by construction (decision 2), so nothing is lost by it being frozen.
5. **Lost sessions fall back to a fresh run.**
   - Applies when a resumed invocation fails with a runner error before any result: a missing session file, a CLI error mentioning the session, or `NoResult`.
   - The engine retries once, fresh, in the same claim, without using an attempt, and audits the failed try as `error:resume-failed`.
   - Same pattern as LLM Chat's `chat_with_retry`.
6. **Attempts accumulate across the loop.**
   - Every child a transformer, gate or fork creates inherits its parent's `attempts`; only a revise increments it.
   - The cap of 3 then applies to the item's whole revise history: at the cap, a revise escalates to needs-human, as the structured-verdicts spec already says.
   - Operational failures keep bumping attempts on the same task, as today.
7. **Worktrees.** A revise child already reuses the writer's worktree, because worktrees are keyed by `(run_id, item_key)`. Resume adds no worktree rule beyond the cwd being part of the fingerprint.
8. **No retention policy.** Session files stay where the CLI keeps them (`~/.claude/projects/…`). Workers keep persisting sessions, so `--no-session-persistence` is never passed to them.

## Architecture

### Shared kernel (`agent_bus_core`)

```rust
pub struct SessionPlan {
    pub session_id: Uuid,
    pub resume_from: Option<Uuid>,  // when set, also --fork-session
}
```

### Runners (ACL) — the only code that knows the session flags

- `RunnerRequest` gains `session: SessionPlan`.
- The worker argv adds `--session-id=<id>`, and `--resume=<from> --fork-session` when resuming. All use the `=` form.
- `RunnerError` gains `ResumeFailed`, classified from a resumed run's stderr or from no-result.
- `config_fingerprint(&RunnerRequest) -> String` is a SHA-256 over the fields in decision 1. It lives here, because only the runner knows which flags shape the prefix.

### Runtime

- **Before invoking:**
  - The engine computes the fingerprint.
  - For a revise child (attempts > 1, with the revising team equal to the item's last artifact writer), it looks up that writer's last successful invocation for `(run_id, item_key, team_id)`.
  - It builds the `SessionPlan`.
- **On `ResumeFailed`:** it retries the same claim with a fresh `SessionPlan` (decision 5).
- **Attempts:** `Task::work_item` callers in `transform_once`, the gate commit and the fork step pass the parent's attempts; the revise paths keep `+1` (decision 6). The join's `already_revised` guard then works as intended.
- **Audit:** the row gets `session_id`, `resumed_from` and `config_fingerprint`.

### Composition root (app)

- **Migration** (the next free number): `ALTER TABLE invocation_audit ADD COLUMN session_id TEXT; … resumed_from TEXT; … config_fingerprint TEXT;`.
- **Lookup:** a join from audit to tasks on `run_id` and `item_key`, filtered by `team_id` and a settled, non-error outcome, newest first.

### Frontend

- No new UI. The Run Inspector spec shows `resumed_from` per invocation.

## Data flow

1. The writer runs fresh with `--session-id=A`. The audit records A and fingerprint F.
2. The reviewer returns revise. The revise child goes to the writer team with attempts + 1.
3. The child's fingerprint is F, and the writer's session A exists, so the child runs `--resume=A --fork-session --session-id=B`. The user message is the input block plus the revision bundle.
4. If the resume fails, the engine retries fresh with `--session-id=C`, audited as `resume-failed` then as the fresh run.
5. A second revise forks B (the latest successful writer session), and so on until the attempts cap escalates.

## Error handling

- **Missing or corrupt session file:** decision 5, retry fresh once.
- **Fingerprint mismatch:** fresh, silently. Not an error.
- **Resumed run produces no `structured_output`:** an operational failure, as for any run (structured-verdicts spec). The next attempt is a revise child or a requeue, and goes through the same decision 2 check.
- **Cache expired** (resume long after the writer ran): resume still works with less saving. No rule is needed.

## Testing

- **Argv:** a fresh run gives `--session-id=<id>` only; a resume gives `--resume=<a> --fork-session --session-id=<b>`; the `=` form is used everywhere.
- **Fingerprint:** stable across equal requests; changes with each listed field; unaffected by the user message.
- **Engine:**
  - a revise to the same team with an equal fingerprint resumes;
  - a different team, a changed prompt or model, or no prior session each start fresh;
  - `ResumeFailed` retries fresh without using an attempt.
- **Attempts:** a writer–reviewer loop escalates at attempt 3; gate and fork children inherit attempts; the join guard trips after one revise.
- **Live check (manual):** a writer–reviewer pair on a scratch repo where the reviewer always revises once. The second writer run is resumed (`resumed_from` set, much less cache creation), and a third revise escalates.

## Out of scope

- A repair turn for missing structured output via resume (structured-verdicts spec). It can be revisited using this machinery.
- Resuming across different teams, or across a pipeline edit.
- Session file cleanup or retention.
- Resume for operational-failure requeues. A requeue reruns the same task fresh.
