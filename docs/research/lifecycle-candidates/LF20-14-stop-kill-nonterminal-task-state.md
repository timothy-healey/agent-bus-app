# Candidate LF20-14 — A live-Stop kill must leave the task RE-RUNNABLE, not terminally `Failed`

## Delivery
Runtime lifecycle: clean exit + clean resume — spec `docs/superpowers/specs/2026-06-25-lifecycle-exit-resume-design.md`. The precondition the worker-side resume chain (LF20-05/06/07) silently assumes but no item guarantees: that a killed worker task is still recoverable when recovery runs.

## Location
- `src-tauri/app/src/pipeline_activator.rs:278`–`297` — the transformer worker loop's handling of `engine::transform_once`:
  - `:280`–`:286` treats `StepOutcome::Failed { .. }` as **settled** (a terminal disposition; emits `task-changed`/`usage-changed`).
  - `:293`–`:296` treats `Err(e)` as a logged non-event (`progressed = false`; the claim is *not* settled).
- The kill site this collides with: the **live** Stop path — `registry.kill_all()` wired by LF20-04 at the brake-on sites (`lib.rs:1067`/`:1070`, `:1473`, `:1527`) while the worker loops are **still alive** (unlike exit/LF20-03, where the whole process is going away).
- The runner output seam: `interpret_runner_output(stdout, stderr, success)` (LF20-02 spawner; `runners` crate) decides whether a SIGKILLed child becomes a terminal `Failed` verdict or a retained-claim `Err`.

## Why it is a candidate
On the **live Stop** path the worker loop does not die — it observes the killed `claude`'s failed output and runs it through `transform_once`. The entire worker-side resume thesis (LF20-07 re-queues killed **`running`** rows on brake-off; LF20-06 reconciles occupancy at boot) is correct **only if the killed task is still in `running` state** when recovery runs. But if the SIGKILLed invocation surfaces as `StepOutcome::Failed` (settled at `:283`), the engine transitions the task **out of `running` into a terminal failed state** *before* anyone hits Resume. LF20-07 then finds no `running` row to re-queue and the work is **permanently lost** — the opposite of "clean resume," on the very path (Stop) the delivery is about.

This is the **worker-path twin of LF20-11 (chat-path kill recovery)**: LF20-11 explicitly assumes "the worker run state machine is handled by LF20-05/06/07," but none of those items addresses the live worker loop **racing the kill** and terminally-failing the row. It is also distinct from LF20-07, which presupposes the `running` row exists rather than guaranteeing it.

The open question that decides whether this is a defect: does a killed worker invocation surface as **`Err`** (claim retained → `running` → recoverable, gap is benign) or as **`StepOutcome::Failed`** (terminal → unrecoverable)? That hinges on `interpret_runner_output`'s treatment of `success=false`+empty-stdout and the engine's `transform_once` mapping — both must be confirmed; this item owns making the answer "recoverable."

## Proposed change
Guarantee a kill is **interrupt, not failure**, so LF20-07 has something to recover:
1. Make a killed invocation **non-terminal** for the task: when `kill_all` is the cause (brake just turned on, or the spawner's `.wait()` shows signal-death), the worker step must **retain the claim** (leave the row `running`) or explicitly re-queue it — never record a terminal `Failed` verdict. Cheapest discriminator at the composition root: the worker loop checks `brake.state()` is on before treating a runner failure as a real verdict, and on a braked failure skips settlement (drop into the `Err`/`progressed=false` arm that retains the claim).
2. Alternatively/additionally, have the spawner (LF20-02) map a **signal-killed** child to a distinct `RunnerError` variant (e.g. `Interrupted`) that the engine never settles as `Failed`.
3. Confirm against `runtime::engine` + `runners::output::interpret_runner_output` whether (1) or (2) (or both) is needed, and that LF20-07's re-queue predicate matches the state a killed task is left in.

## Tests (no live `claude`)
- Behavioral: drive a fake spawner that returns a signal-killed/failed result **while the brake is on**; assert the task stays `running` (or is re-queued) and is **not** recorded as a terminal `Failed` verdict, so a subsequent LF20-07 brake-off recovery re-queues it.
- Contrast: the same failed output with the brake **off** (a genuine failure) settles as `Failed` as today — the discriminator does not swallow real failures.

## Dependencies / sequencing
- **Depends on** LF20-02 (the worker runner is now killable — the reason a live loop sees a killed result) and LF20-04 (the live Stop kill).
- **Guards** LF20-07's precondition (re-queue assumes `running`) and pairs with LF20-13 (the Stop-path async-offload — same live-Stop locus).
- **Mirrors** LF20-11 (chat-path) for the worker path.

## Out of scope
The kill primitive (LF20-01); boot-only crash recovery (already handled by `release_orphaned_running` when the whole process dies); the chat path (LF20-11); UI surfacing of an interrupted task (LF23).
