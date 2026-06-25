# Candidate LF20-11 — Chat-path kill recovery (killed in-flight chat invocation leaves the dialogue inconsistent)

## Delivery
Runtime lifecycle: clean exit + clean resume — spec `docs/superpowers/specs/2026-06-25-lifecycle-exit-resume-design.md`. The half of the kill surface the worker/run-focused resume work (LF20-05/06/07) does **not** touch: the terminal's free-form chat dialogue.

## Location
- `src-tauri/app/src/lib.rs:1376`–`1380` — the chat runner: `chat_runner_for(...)` → `Arc<dyn llm_chat::chat::ChatRunner>`, which LF20-02 routes through `ClaudeChatRunner::with_spawner` so its in-flight `claude` child is **registered and killable**.
- `src-tauri/app/src/lib.rs:1388`–`1398` — `AgenticChatEngine` streams deltas via `make_conversation_delta_sink(handle)`; brake-aware within-turn loop.
- `src-tauri/app/src/lib.rs:1402` — `ConversationStore::new(pool)`; conversations persisted per `project_id` (the stable `dialogue_id`), saved on turn completion.
- (The `Conversation`/turn model and save semantics live in `../llm_chat` and `../conversational_control`, outside the app crate — read-only analysis here.)

## Why it is a candidate
LF20-02 makes the **chat** runner killable alongside the worker runner — so Stop (LF20-04) and exit (LF20-03) now also abort an in-flight *chat* turn mid-stream. But every recovery item (LF20-05 `reconcile_occupancy`, LF20-06 boot wiring, LF20-07 brake-off recovery) operates **only** on the `tasks` + `stores` run state machine. The chat dialogue has **no task row and no store occupancy** — it is an entirely separate persistence path (`ConversationStore`). So when a chat invocation is killed:
- The streaming assistant turn is cut mid-delta; the persisted `Conversation` can hold a **user turn with a partial or empty assistant reply and no terminal/error marker**.
- **Nothing** finalizes it: no recovery path inspects `ConversationStore`. On next boot the `summarise_on_launch` pass (`lib.rs:1406`) and the next `send_message` operate over a malformed transcript (dangling/partial turn) — risking a confused model context or a broken render.

This is a genuine, distinct gap: the delivery's resume thesis is worker-run-centric and silently ignores the chat path it just made killable.

## Proposed change
Give the chat path its own kill-consistency story, symmetric to the worker path's reconcile:
1. **On kill (live app):** the chat spawner's SIGKILLed `.wait()` returns a failed result; ensure `AgenticChatEngine` finalizes the interrupted turn — seal/drop the partial assistant message and mark the turn interrupted/failed — and `ConversationStore::save`s a **well-formed** transcript (no dangling user turn).
2. **On resume/boot:** a defensive pass that detects a dangling/partial assistant turn in the loaded `Conversation` and seals it before `summarise_on_launch` / the next send (mirrors LF20-06's "repair on the resume path" for the chat store).
3. Confirm the turn/partial-delta model against `../llm_chat` + `../conversational_control` (does a turn get persisted before completion, or only on success? — that determines whether (1), (2), or both are needed).

## Tests
- Behavioral (no live `claude`): drive a fake chat spawner that emits a few deltas then is killed; assert the persisted `Conversation` has no dangling/partial turn and the next `send_message` produces a coherent transcript.
- Boot/resume: seed a `Conversation` with a dangling assistant turn; run the resume pass; assert it is sealed.

## Dependencies / sequencing
- **Depends on** LF20-02 (the chat runner is now killable — the reason this gap exists) and the kill triggers LF20-03/04.
- **Independent of** the worker-run reconcile chain (LF20-05/06/07) — a parallel, chat-specific recovery; can be built alongside it.
- Cross-crate: asserts against `conversational_control`/`llm_chat`; wired/verified at the app root where the engine + `ConversationStore` are assembled.

## Out of scope
The worker/run reconcile (LF20-05/06/07); live `claude`; chat UI rendering of the interrupted turn (surfacing detail, likely LF22/LF23).
