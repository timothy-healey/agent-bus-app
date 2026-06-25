# Lifecycle exit/resume — research candidates

Decomposition candidates for the spec `docs/superpowers/specs/2026-06-25-lifecycle-exit-resume-design.md`,
produced by the app's own live research agent (runs `R-fab` + `R-51aa`) during the 2026-06-25 live test.

Preserved here before the DB/artifact wipe — see `docs/live-test-findings-2026-06-25.md` LF26 (the
agent wrote these under the app's run dir, `src-tauri/app/artifacts/`, instead of the project root).

These are ~19 distinct ideas across 24 files; several keys are near-duplicates of the same idea
because the generator ran two passes (the key-based ledger dedup doesn't catch semantic overlap — see LF25).

## Kill foundation (clean exit + Stop)
- `LF20-01-process-registry` — shared pgid registry; `kill_all` = SIGTERM → grace → SIGKILL on the group
- `LF20-02-killable-spawners` — route worker + chat runners through `with_spawner` + `.process_group(0)`
- `LF20-03-exit-kill-trigger` — `RunEvent::ExitRequested` → `kill_all`
- `LF20-04-stop-brake-kill-trigger` — `kill_all` at the three brake-on sites

## Resume / reconcile half
- `LF20-05-reconcile-occupancy-method` — `StoreRepo::reconcile_occupancy` rebuilds the occupancy leak killing creates
- `LF20-06-boot-reconcile-wiring` — invoke reconcile on the boot recovery path
- `LF20-07-resume-recovery-on-brake-off` — in-process recovery on Resume (no reboot)
- `LF20-08-lifecycle-integration-test` — assembled kill → recover → resume confidence test

## Hardening (beyond the spec)
- `LF20-09-exit-kill-shutdown-bounding` — don't hang quit / don't re-orphan SIGTERM-ignorers
- `LF20-10-brake-persistence-across-reboot` — don't silently un-Stop on next launch
- `LF20-11-auto-meter-no-kill-rerun-loop` / `LF20-12-auto-meter-kill-policy` — auto-meter brake must NOT kill-and-re-run (pay-kill-pay loop); only manual Stop kills *(near-duplicates)*
- `LF20-12-brake-reason-persistence-policy` — reason-aware persistence (auto brake shouldn't come back sticky)
- `LF20-12-kill-spawn-race-latch` / `LF20-15-spawn-register-kill-race` — kill latch closes the spawn-after-sweep escape window *(near-duplicates)*
- `LF20-13-stop-path-async-kill-offload` / `LF20-14-blocking-kill-in-async-stop-path` — offload the blocking grace off the Tokio executor *(near-duplicates)*
- `LF20-11-chat-path-kill-recovery` — seal dangling chat turns left by a killed chat invocation
- `LF20-13-chat-exempt-from-kill` — exempt the user's live chat from Stop / auto-meter kills
- `LF20-13-pgid-reuse-kill-safety` — never signal a reaped+recycled pgid
- `LF20-14-stop-kill-nonterminal-task-state` — a Stop-kill must leave the task re-runnable, not terminal `Failed`
- `LF20-14-worktree-hygiene-on-requeue` / `LF20-16-worktree-reset-on-resume` — reset the dirty worktree before re-run *(near-duplicates)*
- `LF20-15-crash-orphan-reaping-on-boot` — persist pgids + reap survivors after an unclean crash
