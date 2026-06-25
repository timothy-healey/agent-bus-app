# Spec — Live worker view: click any running worker → output + thinking

*Design doc. Brainstormed 2026-06-25. Lets the operator click into any running worker (incl. the generator) and watch its live `claude` **output and thinking** stream. Builds on R4 (the `task-log` live-log + CardDrawer "live log" tab) and the R hardening (`invoke_stream` now streams). Also resolves LF23 (a running generator looked idle).*

## Why this exists

Clicking a running **transformer** card already opens its live output (R4 + R). But three gaps block "see any running worker's actual output and thinking":
1. **Thinking is never captured** — the stream-json parser forwards only `assistant`→`text` blocks; `thinking` blocks are dropped.
2. **The generator (source) has no card** — it's a running worker with nothing to click (its pass produces no work-item until it returns).
3. **The log is ephemeral** — fine for a *running* worker (watched live), but a finished worker's stream isn't persisted (out of scope here — running-worker viewing only).

## Decisions (from the brainstorm)

1. **Thinking shown inline, dimmed/labeled** — captured as a distinct channel and rendered interleaved with output (dimmed/italic + a subtle "thinking" marker), so you see reasoning *and* output as they stream.
2. **The generator becomes a clickable worker** — a transient, event-driven board card for the active source pass ("research · scanning…"), clickable → its live output+thinking. No DB row.

## A. Capture thinking (Runners ACL)

`runners/src/stream_json.rs` (and the mirror in `llm_chat/src/stream_json.rs`) `feed` currently emits a prose `String`. Extend it to emit **tagged deltas**:
- `pub struct LogDelta { pub kind: LogKind, pub text: String }`, `enum LogKind { Output, Thinking }`.
- In `feed`, when iterating an `assistant` message's content blocks: `type == "text"` → `Output`; `type == "thinking"` (the `thinking` field) → `Thinking`. Both are forwarded; only `Output` accumulates into the final-text used for verdict/artifact parsing (thinking never affects `parse_verdict`/`parse_items`).
- The `LogSink` becomes `Box<dyn Fn(&LogDelta)>` (was `Fn(&str)`); the streaming runners forward each tagged delta.
- The composition root's `make_task_log_sink` coalesces per kind and emits `task-log` events with a **`kind` field**: `{ task_id, delta, kind: "output" | "thinking" }`.

Thinking appears only when the team's effort > off (`--max-thinking-tokens` > 0); otherwise just output. Live `claude` stays structural-only — the thinking-parse path is fixture-tested.

## B. Live view (frontend)

- `ipc/runtime.ts` `TaskLog` gains `kind: "output" | "thinking"`; `useTaskLog` accumulates **tagged segments** per task (an ordered list of `{kind, text}` runs, coalescing consecutive same-kind deltas), not one flat string.
- The CardDrawer **"live log"** tab renders the segments inline: **output** as normal prose; **thinking** dimmed/italic with a subtle leading "thinking" marker (tokenized — `--text-3`, no new colors). Auto-scroll to the tail while streaming; the existing loading/streaming/settled/error/empty states are preserved.
- Persistence is in-memory (running workers). Viewing a *finished* worker's full stream is the separate deferred persistence item (noted, not in scope).

## C. Generator as a clickable worker

- **Stable id:** a generator pass runs under `gen:<run_id>:<source-stage>`. `engine::generate_once` builds its invocation `task_id` from that id, so the existing `invoke`/`LogSink` streams the generator's output+thinking under it (no engine plumbing beyond using the id).
- **Activity signal:** `generate_once` emits a **`generator-status`** event `{ run_id, stage, task_id, active: bool }` — `active: true` when a pass starts (before `invoke`), `false` when it settles or the generator goes dry. Emitted from the composition root's worker loop around the step (the engine returns the status; the loop emits — keeping `runtime` Tauri-free).
- **Frontend:** a `useActiveGenerators(runId)` hook accumulates currently-active generators (set on `active:true`, cleared on `active:false`). `BoardView` renders a **transient card** in the source lane for each active generator ("⟳ research · scanning…"), clickable.
- **Opening it:** clicking the transient card sets `openTaskId` to its `gen:…` id. `App`/`CardDrawer` resolve a **synthetic Task** for a `gen:` id (no DB row) — a minimal Task (stage = source, state = running, no artifact/history) so the drawer opens on the **live-log tab** showing the stream; the artifact/review/lineage/history tabs show their empty states (a generator pass has no artifact). When the pass ends, the transient card clears and the drawer (if open on it) shows the settled/empty live-log state.

## E. Card identity & content (LF32)

Today a card's header leads with the raw `task.id` (`T-<uuid>`, rendered in accent) and its body renders `task.topic` — which is **empty for every generated work-item**, so the body is blank and the only human-meaningful string (`item_key`, the slug, e.g. `LWV-A1-logdelta-seam`) appears nowhere. Redesign so a card reads in plain language:

- **Description source:** the generator/transformer **output contract** gains a `DESCRIPTION:` line per item — a short (≤ ~80 char) plain-language summary the agent writes alongside `KEY:`/`ARTIFACT:`/`VERDICT:`. `parse_items` captures it into `OutputItem.description`; the engine stores it on the work-item's existing `topic` field (no new column — the card already renders `topic`). Fallback when absent (legacy/transform items that emit no `DESCRIPTION:`): derive a readable title from the slug (de-kebab the `item_key`), then the stage label.
- **Card head:** lead with the description (`topic`) as the prominent title; show the **slug** (`item_key`) as secondary muted/mono metadata; **drop the raw `task.id`** from the prominent accent slot (it remains available in the history/lineage tabs for debugging, not as the headline). The existing `{current_stage} · a{attempts}` line stays (right-aligned, already padded clear of the close ✕ — LF30).
- **BoardView card** mirrors the same: title = description, sub = slug, not the `task.id`.

This makes both the board cards and the drawer header legible without changing the work-item model beyond populating `topic`.

## D. Testing (no live `claude`)

- **stream_json:** a fixture stream-json with a `thinking` block + a `text` block → `feed` yields both, tagged `Thinking`/`Output`; `final_text`/`parse_*` see only the output (thinking excluded). Mirror test for `llm_chat`.
- **LogSink/event:** the coalescing sink emits `task-log` with the right `kind`.
- **useTaskLog:** accumulates ordered tagged segments; coalesces consecutive same-kind deltas.
- **CardDrawer:** renders an output + thinking sequence with thinking visually distinct (assert the thinking marker/class).
- **Generator card:** `useActiveGenerators` sets/clears on the status events; `BoardView` shows a transient source card while active and removes it on `active:false`; clicking opens the drawer on the live-log tab for the `gen:` id.
- **Card identity (E):** `parse_items` captures `DESCRIPTION:` into `OutputItem.description`; the engine writes it to the work-item `topic`. An item with no `DESCRIPTION:` falls back to the de-kebabbed slug. `CardDrawer`/`BoardView` render the description as title + slug as secondary and no longer show the raw `task.id` in the headline (assert the `T-…` id is absent from the head and the slug/description present).

## Out of scope / non-goals
- Persisting a finished worker's full log for after-the-fact viewing (separate deferred item; running-worker live view only).
- Per-pool multiple concurrent workers per team (v1 is one loop per team — a "running worker" is the one in-flight item/pass).
- Rendering thinking with rich formatting beyond dimmed/labeled text.

## Relationship to other items
- Builds on **R4** (live-log + the CardDrawer tab) and **R** (`invoke_stream` streaming).
- The generator card resolves **LF23** ("started a run, nothing happening").
- §E resolves **LF32** (opaque `task.id` headline + blank body) and pairs with **LF31** (the generator card is the §C work).
- Pairs with **LF22** (the terminal already shows a thinking indicator for the chat; this brings thinking to the worker stream).
