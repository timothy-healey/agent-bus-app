# Candidate LF20-13 — Exempt interactive chat invocations from Stop/brake `kill_all`

## Delivery
Runtime lifecycle: clean exit + clean resume — spec `docs/superpowers/specs/2026-06-25-lifecycle-exit-resume-design.md` (§ Killable spawner; § Kill triggers — Stop). Scopes *which* registered processes the Stop trigger is allowed to kill.

## Location
- `src-tauri/app/src/pipeline_activator.rs:431` `chat_runner_for(...)` — LF20-02 routes the CLI chat branch through `ClaudeChatRunner::with_spawner(...)` (`:438`), registering the chat child in the **same** `ProcessRegistry` as worker invocations.
- `src-tauri/app/src/lib.rs:1539` — `conversational_control::api::send_message` (the interactive chat / terminal entrypoint registered in `invoke_handler`).
- The kill triggers that call `kill_all()`: exit (LF20-03, `lib.rs:1542`) and the three brake-on sites (LF20-04: `lib.rs:1070`, `:1473`, `:1527`).

## Why it is a candidate
LF20-02 deliberately routes **both** the worker `ClaudeCliRunner` **and** the chat `ClaudeChatRunner` through `with_spawner`, so both land in one shared registry. But `kill_all()` (LF20-01) kills **everything registered**. That means pressing **Stop** — or, far worse, an autonomous **auto-meter brake** trip (`lib.rs:1473`, see LF20-11) — will **kill the user's in-flight chat response mid-stream**. LF20's backlog headline is specifically "exit/brake doesn't stop in-flight **`claude` subprocesses**" in the context of **autonomous worker work** stalling the run; the user's live conversational turn is a different category — it is *foreground, user-driven, and not part of the run's bounded-buffer occupancy* (killing it leaks nothing, and recovering it makes no sense — there is no task row to re-queue). Silently terminating a streaming chat answer when the budget meter ticks is a surprising UX defect that no LF20-01..12 item addresses: every kill-trigger item assumes the only thing registered is worker work.

## Proposed change
Separate kill scope so Stop/brake kills **worker** invocations only, leaving chat alone (while exit may still kill both — quitting the app should tear everything down):
- Simplest: **two registries** (or one registry with a `Scope::{Worker, Chat}` tag per entry). The worker spawner (LF20-02 at `pipeline_activator.rs:404`/`:186`) registers as `Worker`; `chat_runner_for` (`:431`) registers as `Chat`.
- `kill_all()` gains a scope argument (or split into `kill_workers()` / `kill_all()`):
  - **Stop / brake-on** (LF20-04) → `kill_workers()` (chat survives).
  - **App exit** (LF20-03/09) → `kill_all()` including chat (clean shutdown of the whole process; nothing to preserve).
- Confirm the desired behavior for a **manual** Stop explicitly in the plan (does the user expect Stop to also cut their chat? Likely no — Stop is about the run). The **auto-meter** case is unambiguous: it must not kill chat (compounds with LF20-11).

## Tests (no live `claude`)
- Register one `Worker`-scoped and one `Chat`-scoped short-lived child; `kill_workers()`; assert only the worker child died and the chat child survives + stays registered.
- `kill_all()` (exit) kills both.

## Dependencies / sequencing
- **Depends on** LF20-01 (registry shape — this adds the scope tag) and LF20-02 (which currently registers chat into the shared registry).
- **Pairs with** LF20-11 (auto-meter must not kill) and LF20-03/04 (the triggers that gain scope).
- Decide the scope model in the plan **before** LF20-01 finalizes the registry signature, since this changes `register`/`kill_all` shape.

## Out of scope
Per-conversation selective kill; UI affordance for an interrupted chat; Windows kill.
