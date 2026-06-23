---
id: vet-c2-chat-streaming-2026-06-23
verb: vet
mode: vet
lens: strategic · critique · brief
target: plans/2026-06-23-plan-c2-chat-streaming.md
date: 2026-06-23
contexts: [LLM Chat (ACL), Conversational Control]
experts: [Architect, AI Engineer, Conversational UX Designer, Engineer]
---

# Vet — C2 Token Streaming for Chat

Pre-build DDD soundness gate on `plans/2026-06-23-plan-c2-chat-streaming.md`.
Brief register (autonomous run; operator AFK — recommended amendment auto-applied
per finding). Focus per operator: streaming stays inside the `llm_chat` ACL (no
Claude stream-json idioms leak past it); the delta channel/callback is a clean
seam; `conversational_control` stays kernel-only; the agentic loop's correctness
is unaffected by display-streaming; event naming.

## Verdict

Design is sound. The seam choice — a `DeltaSink = Box<dyn Fn(&str)>` of plain
prose — keeps stream-json, `session_id`, and the Tauri event name sealed inside
`llm_chat` and the composition root respectively; `conversational_control` is
untouched (kernel-only holds). Display-only streaming does not perturb loop
correctness: `extract_tool_call_block` still runs on the complete `reply.text`
of each step (plan Task 6), and the recorded composite `Turn` is unchanged. Two
medium findings on craft (a control marker riding the prose channel; duplicated
run/impl bodies). Both amended in-plan. No deep findings; no redesign bounce.

## Findings

### F1 [medium] clean-seam (→ §B leaky-boundary, design-stage "cross-boundary by design") — control marker `"\u{0}reset"` rides the prose `DeltaSink`

**What:** The plan overloads the `DeltaSink` prose channel with an in-band
control token (`sink("\u{0}reset")`) to signal "a new model step started, clear
the live bubble". Plan §Task 6 Step 3 and §Decisions ("we pass the sentinel
through the SAME sink for simplicity").

**Cited plan section:** Task 6 Step 3 (`sink("\u{0}reset")`), Task 6 Step 5
(`make_conversation_delta_sink` interprets `frag == "\u{0}reset"`), Decisions
bullet "Stream only the agentic loop's final-ish prose…".

**Affected code:** `src-tauri/llm_chat/src/chat.rs` (the `DeltaSink` type doc
states "plain `&str` … NO stream-json idiom … crosses the ACL"), `app/src/lib.rs`
(`AgenticChatEngine::respond`).

**Why it matters:** `DeltaSink` is documented as a *display-only prose* seam.
Smuggling a magic string through it makes the channel carry two meanings — prose
and a step-boundary command — which is exactly the kind of overloaded contract
DDD warns against (one channel, two concepts). It is fragile (a model could in
principle emit the sentinel) and muddies the seam the operator explicitly asked
to keep clean. Note: the sentinel is injected by the *root engine* and consumed
by the *root sink* — it never enters `llm_chat`'s `chat_stream` /
`parse_chat_stream_streaming`, so the ACL boundary itself does not leak. The
smell is the overloaded root-internal channel, not an ACL breach.

**Suggested amendment (applied):** Keep the prose `DeltaSink` prose-only. Move the
step-boundary signal out of the channel: the root holds the `Arc<Mutex<Buf>>`
state and exposes a separate, explicit `reset()`/flush the engine calls *between
steps* — i.e. `make_conversation_delta_sink` returns BOTH a `DeltaSink` and a
`reset` closure (or the engine emits the `{reset:true}` boundary itself via the
`AppHandle` it already holds, with the sink doing prose only). The ACL contract
(`DeltaSink` = prose `&str`) stays literally true; no magic string. Recommended
concrete shape: a small root struct `ConversationDeltaEmitter { sink: DeltaSink,
reset: Box<dyn Fn() + Send + Sync> }` built by `make_conversation_delta_sink`;
`AgenticChatEngine`/`LlmEngine` hold an `Option<ConversationDeltaEmitter>`, call
`emitter.reset()` at the top of each step and pass `&emitter.sink` to
`chat_stream`.

**Status:** resolved (plan amended — see Task 6 rewrite).

### F2 [medium] adds-where-a-refactor-fits (design-stage; no code-stage sibling) — `run_once_streaming` duplicates `run_once` + a second near-identical trait-impl body

**What:** The plan adds `run_once_streaming` beside `run_once` (same session
record + same lost-session retry), and a second full `chat_stream` impl body that
mirrors `chat`'s match arms (the D6 lost-session fallback re-written verbatim).
Plan §Task 4 Step 3.

**Cited plan section:** Task 4 Step 3 (`run_once_streaming` and the `chat_stream`
override duplicating the `chat` match block).

**Affected code:** `src-tauri/llm_chat/src/claude_cli.rs` — `run_once` (lines
47–59) and the `chat` impl with the D6 fallback (lines 70–89).

**Why it matters:** *Refactor before you add.* The lost-session retry (D6) is a
single invariant of the ClaudeChatRunner; duplicating it across `run_once` /
`run_once_streaming` and `chat` / `chat_stream` means a future fix to the D6
policy must be made in two places or it silently drifts — the non-streaming and
streaming paths could diverge on session handling, which is precisely the
correctness property F3 (no session leak) and D6 rely on.

**Suggested amendment (applied):** Collapse to one parameterised core. Make
`run_once` take an `Option<&mut dyn FnMut(&str)>` delta forwarder (None for the
plain path) and always parse via `parse_chat_stream_streaming` (a None forwarder
== no deltas == identical result to `parse_chat_stream`, proven by Task 2's
`parse_streaming_matches_non_streaming_for_fixtures`). Make the D6 fallback a
single private helper `chat_with_retry(req, forward)` that both `chat` and
`chat_stream` call — `chat` passes no forwarder, `chat_stream` passes the sink.
One session-record site, one retry policy, two thin entry points.

**Status:** resolved (plan amended — see Task 4 rewrite).

## Non-findings (checked, clean)

- **ACL idiom containment (operator focus).** `DeltaSink` is `&str` prose; no
  `session_id`, stream-json `type`, or event name crosses out of `llm_chat`.
  `parse_chat_stream_streaming` lives in `stream_json.rs` (inbound ACL half) and
  forwards only assistant prose. Holds.
- **`conversational_control` kernel-only.** No plan task touches the
  `conversational_control` crate; the engines live at the composition root
  (`app/src/lib.rs`) which is the one place allowed to import both the trait and
  `llm_chat`. The `send_message` flow (`api.rs`) is unchanged — the authoritative
  `Turn` still records the final prose only. Holds.
- **Agentic-loop correctness unaffected.** `extract_tool_call_block(&reply.text)`
  runs on the complete reply (Task 6 keeps the existing line); no partial
  tool-call JSON is ever parsed; `MAX_STEPS`, brake checks, and the composite
  `Turn` are untouched. Holds.
- **Event naming.** `conversation.delta` matches the dotted `task.changed` /
  `usage.changed` convention and the operator's recommended name. Payload
  `{ text, reset }` is display data, not a domain event of the Conversation
  aggregate. Acceptable.
- **DOMAIN.md consistency.** LLM Chat ACL still "returns assistant reply text +
  usage"; streaming is additive. Conversational Control still owns the persisted
  Turn; the streaming bubble is transient display state, not a Turn. No drift.
