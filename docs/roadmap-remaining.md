# Remaining Roadmap

Single consolidated list of everything still deferred as of 2026-06-25, after the v1.1 backlog
(`docs/v1.1-backlog.md`, essentially all `[x] done`) and the four chunks shipped 2026-06-25
(`plan-lifecycle`, `plan-live-worker-view`, `plan-usage-ingestion`, `plan-run-control`).

Status legend: `backlog` · `in progress` · `done`.

## Lifecycle hardening — `in progress`
Deferred from the lifecycle chunk as beyond-spec (analyses in `docs/research/lifecycle-candidates/`).
All build on chunk 1's `ProcessRegistry` / killable spawner / brake wiring.

- **LH1 · Kill/spawn race latch** — `backlog` — a worker that passes the brake gate can `.spawn()` *after* `kill_all`'s snapshot and escape Stop. Add a "killing" latch to `ProcessRegistry` so a spawn into a swept state self-kills. (candidates LF20-12-kill-spawn-race-latch, LF20-15-spawn-register-kill-race)
- **LH2 · PID-reuse guard** — `backlog` — between reap and deregister (and snapshot and signal), the OS could recycle a pgid; hold the `Child` / verify before signalling so `kill_all` never signals a stranger. (LF20-13-pgid-reuse-kill-safety)
- **LH3 · Async-offload the kill grace** — `backlog` — `kill_all` blocks ~2.5s synchronously; on the Stop path (Tokio) that stalls the executor. `spawn_blocking` it / add an async variant. (LF20-13-stop-path-async-kill-offload, LF20-14-blocking-kill-in-async-stop-path)
- **LH4 · Crash-orphan reaping on boot** — `backlog` — an unclean crash bypasses the exit kill; persist live pgids and reap survivors on boot *before* recovery re-runs the task (else two `claude` on one worktree). (LF20-15-crash-orphan-reaping-on-boot)
- **LH5 · Chat-exempt-from-kill** — `backlog` — Stop / auto-brake currently also kill the user's in-flight *chat* turn. Scope kills to worker invocations (tag the registry by Worker/Chat); exit still kills both. (LF20-13-chat-exempt-from-kill)
- **LH6 · Brake persistence + per-reason policy** — `backlog` — a manual Stop doesn't survive reboot (run silently auto-resumes); persist `(on, reason, ts)` and restore manual/reactive brakes authoritatively, while auto-meter brakes re-derive from fresh telemetry. (LF20-10-brake-persistence-across-reboot, LF20-12-brake-reason-persistence-policy)
- **LH7 · Auto-meter no-kill policy** — `backlog` — the budget auto-brake must NOT kill-and-re-run in-flight work (pay-kill-pay loop); make the kill reason-aware (manual kills, auto soft-brakes). (LF20-11-auto-meter-no-kill-rerun-loop, LF20-12-auto-meter-kill-policy)
- **LH8 · Killed-task stays re-runnable** — `backlog` — a live Stop-kill must leave the worker task non-terminal (retain claim / re-queue), not terminal `Failed`, or resume finds nothing to recover. (LF20-14-stop-kill-nonterminal-task-state)

## Worktree isolation — `in progress`
- **WT1 · Per-task git worktree creation for implementers** — `backlog` — per-task worktree *creation does not exist* (S2 honest note); workers run under `--add-dir` scopes and chunk 1 anchors them to the target repo. Before the pipeline reaches the `implement` stage, implementers need an isolated worktree (committed, never pushed) so they don't mutate the target repo directly. Extends chunk-1's `working_dir` resolution.
- **WT2 · Worktree reset/hygiene on resume** — `backlog` — a killed mid-write `claude` leaves a dirty worktree; reset it to a clean baseline before the re-queued task re-runs. Pairs with LH8 + S2 cleanup. (LF20-14-worktree-hygiene-on-requeue, LF20-16-worktree-reset-on-resume)

## Open findings
- **LF25** — `backlog` — `generator_ledger` records candidate keys *before* the work-item commits, so a key whose commit fails/backpressures is "found" but never stored → lost on resume. Fix: record after commit.

## Authoring-UI deferrals (backend done, UI pending)
- **AU1 · P2/P3 join authoring controls** — `backlog` — `cancel_on_reject` toggle + quorum (N-of-M) control in the wizard (runtime supports both).
- **AU2 · Nested-lane (P1) hierarchical render** — `backlog` — render nested fork lanes distinctly in `PipelineView`.

## Frontend impeccable-pass leftovers
- **FE1 · Inline-style → CSS-class migration** — `backlog` — ~26 one-off chrome components remain inline (only `ui/*` + high-traffic controls were converted).
- **FE2 · Tablist roving-tabindex / arrow-key nav** — `backlog` — `role=tab` present, arrow-key navigation not added.
- **FE3 · UsageMeter full tooltip rows** — `backlog` — brake-at, 10-min avg, per-team effort (needs data plumbing).

## Platform / test hardening
- **R4-API · Live-log + thinking for the anthropic-api runner** — `backlog` — the API runner uses the default `invoke_stream` (final result only), so API-path workers show no live log/thinking (pre-existing; R4 live-log was CLI-only). Surfaced by the live-worker-view candidates (LWV-A3).
- **S3-validate · Validate the sandbox-exec boundary** — `backlog` — S3 shipped experimental + default OFF + not a proven boundary; needs a live confinement test + a Settings toggle.
- **S4-CI · Packaged-app E2E on Linux/Windows CI** — `backlog` — WebDriver harness scaffolded (can't run on macOS); wire `e2e.yml` + first live run. (S5's Playwright frontend-in-browser E2E runs locally.)
- **E2E-live · Prove the live worker→`claude` run path end-to-end** — `backlog` — still structural-only; being exercised manually via live testing.

## Standing
- **Push to remote** — `backlog` (by standing instruction) — everything v1→today is committed + tagged locally only, never pushed.
