# Composite Engine (agentic terminal) Implementation Plan — backlog item C1

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the god terminal a single **agentic** `ConversationEngine` (`CompositeEngine`) that, for one user message, either dispatches a leading-`/` slash command one-shot OR runs a bounded, brake-aware **within-turn agentic chat loop** in which the model emits tool-calls (fenced ```json `{tool,args}`), the engine dispatches them through the existing `RootDispatcher`, feeds each `ToolCallResult` back to the model, and stops when the model replies with plain prose.

**Architecture:** `CompositeEngine` lives at the composition root (`app` crate) — the only place allowed to import both `conversational_control` (parser, `ToolDispatcher`, `CommandEngine`) and `llm_chat` (`ChatRunner`) plus the `Brake`. It composes a slash branch (existing parser → `RootDispatcher`, restoring the full slash surface) and an agentic loop branch over `llm_chat`'s `ChatRunner`. `conversational_control` stays a **pure customer** (kernel-only deps): no new edge is added into it. The loop is **bounded** (max-steps) and **brake-aware** (checks the `Brake` before each step) — these are load-bearing safety invariants tested explicitly.

**Tech Stack:** Rust (async-trait, tokio, serde_json), the `agent_bus_core` kernel (`ToolSpec`/`ToolCallRequest`/`ToolCallResult`), the `conversational_control` parser + `ToolDispatcher` seam + `ToolCatalog`, the `llm_chat` `ChatRunner` seam + `FakeChatRunner`, `runtime::brake::Brake`, the Tauri 2 composition root.

**Source backlog item:** **C1** — "merge the slash-command and free-form chat engines so the god terminal handles both." This plan implements the **bigger** variant the operator chose: **Option B + within-turn agentic loop** (model-driven tool-use), superseding the earlier "pure routing (Option A)" draft of this plan. Follows the `llm_chat` foundation plan (`plans/2026-06-23-plan-llm-chat-foundation.md`).

---

## Open design decisions — OPERATOR MUST RULE BEFORE IMPLEMENTATION

> These are the genuine forks. Each has a recommendation; the operator rules before any task begins. They change behaviour and the test surface, not the file structure.
>
> **DDD vet (`docs/vet-composite-engine-2026-06-23.md`, 2026-06-23) — verdict: PLAN READY FOR IMPLEMENTATION.** Two additional low/medium operator forks the vet surfaced (wording / one type name, not structure):
> - **VF1 (vet F1, `adds-where-a-refactor-fits`):** `extract_tool_call_block` (DD3/Task 1) re-implements `pipeline::design_session::extract_json_block`. Fork: (a) lift a shared extractor both `pipeline` + `app` call, or (b) accept the duplication with an explicit "keep in sync" cross-reference comment. Recommend (a) if the extraction is expected to evolve (native tool-use, per roadmap); (b) is acceptable for now. **Open.**
> - **VF2 (vet F2, `off-language-naming`):** `ParsedToolCall` (Task 2) shadows the kernel's `ToolCallRequest`. Fork: rename (e.g. `ToolCallEmit`) or keep. `CompositeEngine`/`AgenticChatEngine` are acknowledged sound (honest root-strategy names matching the existing `*Engine` suffix). **Open.**

### DD1 — max-steps bound (value + behaviour on hit)

The loop must terminate. Cap the number of model↔tool round-trips per user message.
- **Recommend: `MAX_STEPS = 8`** model calls per user message. A "step" = one `ChatRunner::chat` call. On reaching the cap without a plain-prose finish, **stop and emit a clear final assistant turn**: `"[step limit reached] stopped after 8 steps; the last tool result is above."`, carrying all tool-calls accumulated so far. Never panic, never loop unbounded.
- Rationale: 8 is generous for real multi-tool flows (inject → snapshot → approve) while bounding a misbehaving model. Value lives in one `const` so it is trivially tuned.

### DD2 — Conversation recording (one composite Turn vs N turns)

The loop makes N tool-calls then a final answer inside **one user message**.
- **Recommend: one composite assistant `Turn`** whose `text` is the model's final prose and whose `tool_calls: Vec<ToolCall>` embeds all N request+result pairs in order. This matches DOMAIN.md exactly: *"Turn — one user message + one assistant response (with embedded tool-calls)"*, and preserves the `Conversation` alternation invariant (one user turn → one assistant turn) with **zero change** to `conversation.rs` / `api.rs` / `send_message_inner`. The alternative (N separate turns) would break alternation (two assistant turns in a row → `ConversationError::NonAlternating`) and contradict the aggregate.
- Consequence: `EngineReply { text, tool_calls }` already carries exactly this shape — the loop accumulates into one `EngineReply`. No trait or aggregate change.

### DD3 — tool-call wire format + completion signal

How the model asks for a tool and how it signals "done".
- **Recommend: reuse the wizard's fenced-JSON pattern.** A tool-call is a fenced block ```` ```json {"tool":"<name>","args":{...}} ``` ````; **plain prose with no such block = the final answer (done)**. Extraction reuses the same approach as `pipeline::design_session::extract_json_block` (prefer ` ```json `, fall back to a bare fence), implemented as a small local `extract_tool_call_block` in `app` so the agentic loop is not coupled to the wizard module. This is the lowest-risk format — the wizard already proves the model reliably emits it, and "no fence = done" needs no sentinel token.

### DD4 — brake / rate-limit placement

- **Recommend:** check the **brake before each step** (before each `chat` call AND before each dispatch): if `brake.is_on()`, stop the loop immediately and emit `"[braked] terminal paused: <reason>"` with whatever tool-calls accumulated so far — **no dispatch happens while braked**. A **rate-limit** `ChatError` (`is_rate_limited()`) stops the loop and surfaces as `"[terminal error] rate limited: …"`. Other `ChatError`s also stop + surface as an error turn (matching today's `LlmEngine`). The brake is reachable at the root via `runtime_state_arc.brake` (`Arc<Brake>`).
  - **Brake-in-flight semantics (vet F4):** consistent with the `Brake`'s documented "in-flight completes" contract (`runtime/src/brake.rs` / DOMAIN.md), a tool-call already dispatched when the brake flips runs to completion; the brake gate prevents the *next* step/dispatch, not mid-call cancellation. Cooperative mid-dispatch cancel is a roadmap follow-up.

### DD5 — how tool results are fed back to the model

- **Recommend:** feed each `ToolCallResult` back as the **next `user_message`** on the **same `dialogue_id`** (so `llm_chat` resumes the same Claude session — F3), wrapped in a brief frame:
  ```
  Tool `<name>` returned:
  ```json
  <ToolCallResult JSON>
  ```
  Continue: call another tool (fenced json) or reply with your final answer.
  ```
  An **unknown/invalid/malformed** tool-call is fed back the same way as an error frame (`Tool call rejected: <reason>. …`) so the model can recover; it **counts against the step cap** and never panics.

---

## Orientation — the real code this plan builds on

Read these before starting; every task references them. All paths are absolute.

- **The engine seam — `/Users/tim/projects/agent-bus-app/src-tauri/conversational_control/src/engine.rs`.**
  - `#[async_trait] pub trait ConversationEngine: Send + Sync { async fn respond(&self, input: &str, catalog: &ToolCatalog) -> EngineReply; }`.
  - `#[derive(Debug, Clone, PartialEq)] pub struct EngineReply { pub text: String, pub tool_calls: Vec<ToolCall> }`.
  - `pub struct CommandEngine { dispatcher: Arc<dyn ToolDispatcher> }`, `CommandEngine::new(dispatcher)`. Its `respond`: `match parse_command(input, catalog) { Parsed::Tool(req) => dispatch + "Done: {tool}." / "That didn't work: {tool}.", Parsed::Chat(msg) => v1 help nudge, Parsed::Error(e) => e }`.
  - `FakeEngine::new(replies)` — seeded `EngineReply` double, records `received: Mutex<Vec<String>>`.
  - **`CommandEngine`, `FakeEngine`, and ALL of this crate's tests stay UNTOUCHED.** `CompositeEngine` reuses `CommandEngine` for the slash branch.
- **The slash parser — `/Users/tim/projects/agent-bus-app/src-tauri/conversational_control/src/command.rs`.**
  - `pub enum Parsed { Tool(ToolCallRequest), Chat(String), Error(String) }`; `pub fn parse_command(input: &str, catalog: &ToolCatalog) -> Parsed`.
  - **Key fact:** `parse_command` trims, then `if !trimmed.starts_with('/') { return Parsed::Chat(...) }`. `CompositeEngine` routes by the **same** `trim_start().starts_with('/')` rule, so a malformed `/cmd` becomes a visible **error turn**, never silently rerouted into the agentic loop.
- **The dispatcher seam — `/Users/tim/projects/agent-bus-app/src-tauri/conversational_control/src/dispatch.rs`.**
  - `#[async_trait] pub trait ToolDispatcher: Send + Sync { async fn dispatch(&self, req: &ToolCallRequest) -> ToolCallResult; }`, held as `Arc<dyn ToolDispatcher>`.
  - `FakeDispatcher::{new, with(tool, result)}`, `pub received: Mutex<Vec<ToolCallRequest>>`; an unseeded tool returns `ToolCallResult::Err { error: "unknown tool: <name>" }`.
- **The send flow — `/Users/tim/projects/agent-bus-app/src-tauri/conversational_control/src/api.rs`.**
  - `TerminalState { catalog: Arc<ToolCatalog>, engine: Arc<dyn ConversationEngine>, store: Arc<ConversationStore>, project_id: String }`.
  - `send_message_inner(project_id, catalog, engine, store, input, now)`: load/create `Conversation` → append user `Turn` → `engine.respond(input, catalog)` → append **one** assistant `Turn::assistant(reply.text, reply.tool_calls, now)` → save → return. **Engine-agnostic and unchanged by this plan** (DD2 keeps it one assistant turn).
- **The catalog — `/Users/tim/projects/agent-bus-app/src-tauri/conversational_control/src/catalog.rs`.**
  - `ToolCatalog::new(Vec<ToolSpec>)`, `.specs() -> &[ToolSpec]`, `.by_name(name) -> Option<&ToolSpec>`. The agentic system prompt is built from `.specs()`; tool-call validation uses `.by_name()`.
- **The conversation aggregate — `/Users/tim/projects/agent-bus-app/src-tauri/conversational_control/src/conversation.rs`.**
  - `append` enforces strict user/assistant alternation (two same-role in a row → `ConversationError::NonAlternating`). This is **why DD2 = one composite Turn** (N assistant turns would violate it).
  - `Turn` / `ToolCall` in `/Users/tim/projects/agent-bus-app/src-tauri/conversational_control/src/turn.rs`: `ToolCall { request: ToolCallRequest, result: Option<ToolCallResult> }`; `Turn::assistant(text, tool_calls, at)`.
- **The composition root — `/Users/tim/projects/agent-bus-app/src-tauri/app/src/lib.rs`.** The only module importing every context.
  - `struct RootDispatcher { runtime, usage, app }` impl `ToolDispatcher` (lines ~238–285) — routes `inject_topic`/`approve_gate`/…/`usage_snapshot`. This is the real dispatcher both branches reach.
  - `pub struct LlmEngine { runner, dialogue_id, system_prompt, model, thinking_budget }` impl `ConversationEngine` (lines ~113–151) — today's free-form chat; `respond` calls `runner.chat(..)` once and returns `EngineReply { text, tool_calls: vec![] }`, or `[terminal error] {e}` on failure. **The agentic loop replaces this single-shot with a bounded loop.**
  - **Current wiring (the seam this plan changes):**
    - line ~18–19: `#[allow(unused_imports)] use conversational_control::engine::CommandEngine;`.
    - line ~454: `let _dispatcher: Arc<dyn ToolDispatcher> = Arc::new(RootDispatcher { … });` — constructed, `_`-prefixed, unused. This plan un-`_`-prefixes it.
    - line ~464–476: `chat_runner` built; `let engine = Arc::new(LlmEngine::new(chat_runner.clone(), project_id, <prompt>, "claude-opus-4-8", 8192));` — the one line that swaps to `CompositeEngine`.
    - The `Brake` is `brake: Arc<Brake>` (line ~396), also inside `runtime_state_arc.brake`; pass `brake.clone()` into the loop.
    - line ~491: `handle.manage(TerminalState { catalog, engine, store, project_id })` — unchanged.
  - Existing root tests: `mod llm_engine_tests` (drives `LlmEngine` with `FakeChatRunner` + a real `ConversationStore`) is the template for the new tests.
- **The chat ACL types + double.**
  - `/Users/tim/projects/agent-bus-app/src-tauri/llm_chat/src/chat.rs`: `ChatRequest { dialogue_id, system_prompt, user_message, model, thinking_budget }`, `ChatReply { text, usage }`, `ChatError` with `is_rate_limited()`, `#[async_trait] trait ChatRunner { async fn chat(&self, req) -> Result<ChatReply, ChatError> }`.
  - `/Users/tim/projects/agent-bus-app/src-tauri/llm_chat/src/fake.rs`: `FakeChatRunner::new(replies)` returns seeded `ChatReply`s **in order** (clamps to last), records `received: Mutex<Vec<ChatRequest>>`; `FakeChatRunner::failing(err)`. This is how we script a multi-step loop (tool-call reply, then final prose) with **no live `claude`**.
- **The fenced-JSON extraction to mirror — `/Users/tim/projects/agent-bus-app/src-tauri/pipeline/src/design_session.rs` `extract_json_block` (lines 18–39).** Prefers ` ```json `, falls back to a bare ` ``` ` fence. The loop reimplements this approach locally as `extract_tool_call_block` (DD3) so the agentic loop does not couple to the wizard module.
- **The brake — `/Users/tim/projects/agent-bus-app/src-tauri/runtime/src/brake.rs`.** `Brake::{is_on, state}`; `state().reason: Option<String>`. The loop checks `brake.is_on()` before each step (DD4).
- **DOMAIN.md (`/Users/tim/projects/agent-bus-app/DOMAIN.md`)** — Conversational Control language: **Conversation**, **Turn** (= "one user message + one assistant response WITH embedded tool-calls" — the DD2 anchor), **App-tool**, **Tool-call**. The agentic loop adds **no new aggregate**: it produces one composite Turn.

---

## Decisions

- **D1 — `CompositeEngine` + the agentic loop live at the composition root (`app/src/lib.rs`).** They compose the parser/`CommandEngine`/`RootDispatcher` (from `conversational_control`), the `ChatRunner` (from `llm_chat`), the `ToolCatalog`, and the `Brake` (from `runtime`) — only the root may import all of these. Placing them in `conversational_control` would force it to depend on `llm_chat`/dispatcher impls/runtime → a cross-context cycle. **`conversational_control` stays kernel-only** (verified in the final task: its `git diff` is empty, its dep tree gains nothing).
- **D2 — Route by the SAME rule the parser uses: leading `/` after `trim_start`.** Slash → one-shot `CommandEngine` (parse → dispatch → compose; `Parsed::Error` → error turn). Otherwise → the agentic loop. Single source of truth for "what is a command".
- **D3 — Reuse `CommandEngine` for the slash branch.** Held as `Arc<dyn ConversationEngine>` so it is independently fakeable; the "Done…/That didn't work…" text + embedded tool-call come straight from the untouched `CommandEngine`.
- **D4 — The agentic loop is its own struct (`AgenticChatEngine`) impl `ConversationEngine`**, holding `runner: Arc<dyn ChatRunner>`, `dispatcher: Arc<dyn ToolDispatcher>`, `brake: Arc<Brake>`, `dialogue_id`, `system_prompt_framing`, `model`, `thinking_budget`, `max_steps`. `CompositeEngine` holds two `Arc<dyn ConversationEngine>` (command branch + agentic branch). Both branches are independently fakeable in tests.
- **D5 — One composite Turn (DD2).** The loop accumulates every `ToolCall { request, result: Some(..) }` into one `EngineReply.tool_calls` and sets `EngineReply.text` to the final prose (or a bounded/braked/error message). `send_message_inner` records it as one assistant turn; alternation holds; `conversation.rs`/`api.rs` are untouched.
- **D6 — Bounded + brake-aware are correctness invariants (DD1/DD4), tested explicitly.** `MAX_STEPS` const; brake checked before each step; rate-limit + other chat errors stop + surface; invalid tool-calls fed back as errors (count against the cap) and never panic.
- **D7 — Tool-call wire format = fenced ```json `{tool,args}`; plain prose = done (DD3).** Extraction via a local `extract_tool_call_block` mirroring the wizard. Validation: the requested `tool` must be `catalog.by_name(tool).is_some()`; otherwise it is an invalid call fed back as an error.
- **D8 — No new Cargo dependency for the loop's core.** `app` already depends on `conversational_control`, `llm_chat`, `runtime`, `agent_bus_core`, `async_trait`, `serde_json`. The only files touched are `app/src/lib.rs` (add two structs + impls, swap one `let engine` line, un-`_`-prefix `dispatcher`, pass `brake`, stop `#[allow(unused_imports)]` on `CommandEngine`) and its test module. `conversational_control` and `llm_chat` are untouched.
- **D9 — `LlmEngine` stays.** The new `AgenticChatEngine` is additive; `LlmEngine` and `mod llm_engine_tests` remain green (the existing single-shot engine is still a valid `ConversationEngine` and its tests still pass). The terminal's *active* engine becomes `CompositeEngine` wrapping `CommandEngine` + `AgenticChatEngine`.

---

## File-structure diff

```
src-tauri/
  app/
    src/
      lib.rs                # CHANGED:
                            #   + fn extract_tool_call_block(&str) -> Option<String>   (DD3, mirrors wizard)
                            #   + struct ParsedToolCall { tool: String, args: Value }   (deserialized from the block)
                            #   + struct AgenticChatEngine + impl ConversationEngine    (the bounded, brake-aware loop)
                            #   + const MAX_STEPS: usize = 8                            (DD1)
                            #   + struct CompositeEngine   + impl ConversationEngine    (slash one-shot vs agentic loop)
                            #   swap `let engine = LlmEngine::new(...)` -> CompositeEngine::new(CommandEngine, AgenticChatEngine)
                            #   `_dispatcher` -> `dispatcher` (now used by both branches)
                            #   pass `brake.clone()` into AgenticChatEngine
                            #   drop `#[allow(unused_imports)]` on `use ...::CommandEngine` (now used)
                            #   + mod composite_engine_tests
  conversational_control/   # UNTOUCHED (stays kernel-only — verified in Task 8)
  llm_chat/                 # UNTOUCHED
  runtime/                  # UNTOUCHED (Brake reused as-is)
```

No new files. No new crate. No schema/migration. No new Tauri command. `LlmEngine` and `mod llm_engine_tests` are left intact (D9). `conversational_control/Cargo.toml`, `llm_chat/Cargo.toml`, and `app/Cargo.toml` are all unchanged.

---

## Tasks

> Each task is TDD: write the failing test first, run it (RED), implement minimally, run it (GREEN), then commit. All `cargo` commands run from `/Users/tim/projects/agent-bus-app/src-tauri/`. The test module name is `composite_engine_tests` and is appended to `app/src/lib.rs`. Tasks 1–7 add code + tests; Task 8 wires it live and verifies the architecture.

### Task 1: Tool-call block extraction (`extract_tool_call_block` + `ParsedToolCall`)

**Files:**
- Modify: `/Users/tim/projects/agent-bus-app/src-tauri/app/src/lib.rs` (add `extract_tool_call_block`, `ParsedToolCall`, and a new `#[cfg(test)] mod composite_engine_tests`)

- [ ] **Step 1: Write the failing test**

Append a new test module at the bottom of `app/src/lib.rs`:

```rust
#[cfg(test)]
mod composite_engine_tests {
    use super::{extract_tool_call_block, ParsedToolCall};

    #[test]
    fn extracts_a_fenced_json_tool_call_block() {
        let prose = "I'll inject that.\n\n```json\n{\"tool\":\"inject_topic\",\"args\":{\"topic\":\"03-scheduling\"}}\n```\n";
        let block = extract_tool_call_block(prose).expect("a fenced block");
        let call: ParsedToolCall = serde_json::from_str(&block).unwrap();
        assert_eq!(call.tool, "inject_topic");
        assert_eq!(call.args["topic"], "03-scheduling");
    }

    #[test]
    fn falls_back_to_a_bare_fence() {
        let prose = "ok\n```\n{\"tool\":\"usage_snapshot\",\"args\":{}}\n```";
        let block = extract_tool_call_block(prose).expect("a bare fence");
        let call: ParsedToolCall = serde_json::from_str(&block).unwrap();
        assert_eq!(call.tool, "usage_snapshot");
    }

    #[test]
    fn plain_prose_has_no_block() {
        assert!(extract_tool_call_block("T-042 is in design; nothing to do.").is_none());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p agent-bus-app composite_engine_tests`
Expected: FAIL — `cannot find function extract_tool_call_block` / `cannot find type ParsedToolCall`.

- [ ] **Step 3: Write the minimal implementation**

Add near the top of `app/src/lib.rs` (after the existing `use` block, beside `LlmEngine`). The extractor mirrors `pipeline::design_session::extract_json_block` (DD3) but is local so the agentic loop is not coupled to the wizard module:

```rust
use serde_json::Value;

/// One tool-call the model emitted inside a fenced ```json block (DD3):
/// `{ "tool": "<name>", "args": { ... } }`. `args` defaults to `{}` when absent.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ParsedToolCall {
    pub tool: String,
    #[serde(default = "empty_object")]
    pub args: Value,
}

fn empty_object() -> Value {
    Value::Object(serde_json::Map::new())
}

/// Extract the first fenced code block from the model's reply (DD3). Prefers a
/// ```json fence; falls back to the first bare ``` fence. Returns the block's
/// inner text (no fences), or None when there is no fence (= the model is done,
/// the reply is the final prose answer). Mirrors the wizard's extract_json_block
/// so the model's proven emit format is reused, but kept local to the root so the
/// agentic loop does not depend on the pipeline crate's wizard module.
pub fn extract_tool_call_block(reply: &str) -> Option<String> {
    if let Some(start) = reply.find("```json") {
        let after = &reply[start + "```json".len()..];
        if let Some(end) = after.find("```") {
            return Some(after[..end].trim().to_string());
        }
    }
    if let Some(start) = reply.find("```") {
        let after = &reply[start + 3..];
        let after = match after.find('\n') {
            Some(nl) if !after[..nl].contains("```") => &after[nl + 1..],
            _ => after,
        };
        if let Some(end) = after.find("```") {
            return Some(after[..end].trim().to_string());
        }
    }
    None
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p agent-bus-app composite_engine_tests`
Expected: PASS — `test result: ok. 3 passed`.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/app/src/lib.rs
git commit -m "feat(app): tool-call block extraction for the agentic loop (C1 task 1)"
```

---

### Task 2: `AgenticChatEngine` — a single tool-call then a final answer (two-step loop, composite turn)

**Files:**
- Modify: `/Users/tim/projects/agent-bus-app/src-tauri/app/src/lib.rs` (add `AgenticChatEngine`, `MAX_STEPS`, the loop; extend `composite_engine_tests`)

- [ ] **Step 1: Write the failing test**

Append to `composite_engine_tests`. This scripts a `FakeChatRunner` with two replies — a tool-call, then plain prose — and asserts the loop dispatches once, feeds the result back, finishes on the prose, and records **one composite turn** (DD2). Helpers (`catalog`, etc.) are defined here and reused by later tasks.

```rust
    use super::{AgenticChatEngine, CompositeEngine, MAX_STEPS};
    use conversational_control::catalog::ToolCatalog;
    use conversational_control::dispatch::{FakeDispatcher, ToolDispatcher};
    use conversational_control::engine::{ConversationEngine, EngineReply, FakeEngine};
    use llm_chat::chat::{ChatReply, ChatRunner, ChatUsage};
    use llm_chat::fake::FakeChatRunner;
    use runtime::brake::Brake;
    use agent_bus_core::{ToolCallResult, ToolSpec};
    use serde_json::json;
    use std::sync::Arc;

    fn catalog() -> ToolCatalog {
        ToolCatalog::new(vec![ToolSpec {
            name: "inject_topic".into(), description: "Inject a topic".into(),
            input_schema: json!({"type":"object"}), supplier_context: "runtime".into(),
        }])
    }

    fn reply(text: &str) -> ChatReply {
        ChatReply { text: text.into(), usage: ChatUsage::default() }
    }

    fn agentic(
        runner: Arc<FakeChatRunner>,
        disp: Arc<FakeDispatcher>,
        brake: Arc<Brake>,
    ) -> AgenticChatEngine {
        AgenticChatEngine::new(
            runner as Arc<dyn ChatRunner>,
            disp as Arc<dyn ToolDispatcher>,
            brake,
            "p".into(),
            "You are the god terminal.".into(),
            "m".into(),
            8192,
        )
    }

    #[tokio::test]
    async fn two_step_loop_dispatches_once_then_finishes_with_a_composite_turn() {
        let runner = Arc::new(FakeChatRunner::new(vec![
            // step 1: the model asks to call a tool
            reply("On it.\n```json\n{\"tool\":\"inject_topic\",\"args\":{\"topic\":\"03-scheduling\"}}\n```"),
            // step 2: fed the result, the model finishes with plain prose
            reply("Done — task T-9 was injected for 03-scheduling."),
        ]));
        let disp = Arc::new(
            FakeDispatcher::new().with("inject_topic", ToolCallResult::Ok { result: json!({"task_id":"T-9"}) }),
        );
        let brake = Arc::new(Brake::new());
        let eng = agentic(runner.clone(), disp.clone(), brake);

        let r = eng.respond("kick off research on 03-scheduling", &catalog()).await;

        // final answer is the plain-prose reply (no fenced block)
        assert_eq!(r.text, "Done — task T-9 was injected for 03-scheduling.");
        // one composite turn embedding the single resolved tool-call (DD2)
        assert_eq!(r.tool_calls.len(), 1);
        assert_eq!(r.tool_calls[0].request.tool_name, "inject_topic");
        assert!(matches!(r.tool_calls[0].result, Some(ToolCallResult::Ok { .. })));
        // dispatched exactly once
        assert_eq!(disp.received.lock().unwrap().len(), 1);
        // two model calls; the 2nd carried the tool result back on the same dialogue_id (DD5)
        let got = runner.received.lock().unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[1].dialogue_id, "p");
        assert!(got[1].user_message.contains("inject_topic"));
        assert!(got[1].user_message.contains("T-9"));
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p agent-bus-app composite_engine_tests::two_step_loop_dispatches_once_then_finishes_with_a_composite_turn`
Expected: FAIL — `cannot find type AgenticChatEngine` (also `CompositeEngine`/`MAX_STEPS` unresolved; they arrive in this/Task 5).

- [ ] **Step 3: Write the minimal implementation**

Add to `app/src/lib.rs` (beside `LlmEngine`). Uses `runtime::brake::Brake` — add `use runtime::brake::Brake;` to the top `use` block (the crate is already a dependency; `Brake` is already used elsewhere in `lib.rs` via `runtime::brake::Brake`, so the import may already exist — if so, do not duplicate it).

```rust
use conversational_control::turn::ToolCall;

/// Max model<->tool round-trips per user message (DD1). One "step" = one
/// ChatRunner::chat call. On hitting the cap the loop stops with a clear turn.
pub const MAX_STEPS: usize = 8;

/// The within-turn agentic chat loop (backlog C1, Option B). For one user
/// message it alternates model calls and tool dispatches until the model replies
/// with plain prose (no fenced tool-call = done, DD3), then returns ONE composite
/// EngineReply embedding every resolved tool-call (DD2). Bounded by MAX_STEPS
/// (DD1) and brake-aware (DD4) — both are safety invariants. Lives at the root
/// because it composes the ChatRunner ACL, the RootDispatcher, the catalog, and
/// the Brake; conversational_control stays kernel-only.
pub struct AgenticChatEngine {
    runner: Arc<dyn ChatRunner>,
    dispatcher: Arc<dyn ToolDispatcher>,
    brake: Arc<Brake>,
    dialogue_id: String,
    system_prompt_framing: String,
    model: String,
    thinking_budget: u32,
    max_steps: usize,
}

impl AgenticChatEngine {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        runner: Arc<dyn ChatRunner>,
        dispatcher: Arc<dyn ToolDispatcher>,
        brake: Arc<Brake>,
        dialogue_id: String,
        system_prompt_framing: String,
        model: String,
        thinking_budget: u32,
    ) -> Self {
        Self {
            runner, dispatcher, brake, dialogue_id,
            system_prompt_framing, model, thinking_budget,
            max_steps: MAX_STEPS,
        }
    }
}

#[async_trait]
impl ConversationEngine for AgenticChatEngine {
    async fn respond(&self, input: &str, catalog: &ToolCatalog) -> EngineReply {
        let system_prompt = build_agentic_system_prompt(&self.system_prompt_framing, catalog);
        let mut tool_calls: Vec<ToolCall> = Vec::new();
        // The first turn is the user's words; subsequent turns are fed-back results.
        let mut next_user_message = input.to_string();

        for _step in 0..self.max_steps {
            // DD4: brake before each step — never call the model or dispatch while braked.
            if self.brake.is_on() {
                let reason = self.brake.state().reason.unwrap_or_else(|| "on".into());
                return EngineReply { text: format!("[braked] terminal paused: {reason}"), tool_calls };
            }

            let req = ChatRequest {
                dialogue_id: self.dialogue_id.clone(),
                system_prompt: system_prompt.clone(),
                user_message: next_user_message.clone(),
                model: self.model.clone(),
                thinking_budget: self.thinking_budget,
            };
            let reply = match self.runner.chat(&req).await {
                Ok(r) => r,
                // DD4: rate-limit (and any other chat error) stops the loop and surfaces.
                Err(e) => return EngineReply { text: format!("[terminal error] {e}"), tool_calls },
            };

            // No fenced tool-call block => the model is done; this is the final answer (DD3).
            let Some(block) = extract_tool_call_block(&reply.text) else {
                return EngineReply { text: reply.text, tool_calls };
            };

            // An unparseable / unknown tool-call is fed back as an error so the
            // model can recover; it counts against the step cap and never panics (DD5).
            let parsed: Result<ParsedToolCall, _> = serde_json::from_str(&block);
            match parsed {
                Err(e) => {
                    next_user_message = format!(
                        "Tool call rejected: that was not valid JSON ({e}). \
                         Reply with a single fenced ```json {{\"tool\":\"<name>\",\"args\":{{…}}}} block, \
                         or your final answer as plain prose."
                    );
                    continue;
                }
                Ok(call) if catalog.by_name(&call.tool).is_none() => {
                    next_user_message = format!(
                        "Tool call rejected: unknown tool `{}`. Available tools: {}. \
                         Reply with a valid fenced ```json tool call, or your final answer.",
                        call.tool, tool_names(catalog),
                    );
                    continue;
                }
                Ok(call) => {
                    // DD4: brake before dispatch too.
                    if self.brake.is_on() {
                        let reason = self.brake.state().reason.unwrap_or_else(|| "on".into());
                        return EngineReply { text: format!("[braked] terminal paused: {reason}"), tool_calls };
                    }
                    let request = ToolCallRequest { tool_name: call.tool.clone(), args: call.args };
                    let result = self.dispatcher.dispatch(&request).await;
                    let result_json = serde_json::to_string(&result).unwrap_or_else(|_| "{}".into());
                    tool_calls.push(ToolCall { request, result: Some(result) });
                    // DD5: feed the result back as the next user message.
                    next_user_message = format!(
                        "Tool `{}` returned:\n```json\n{}\n```\nContinue: call another tool \
                         (fenced json) or reply with your final answer.",
                        call.tool, result_json,
                    );
                }
            }
        }

        // DD1: hit the step cap without a plain-prose finish.
        EngineReply {
            text: format!(
                "[step limit reached] stopped after {} steps; the last tool result is above.",
                self.max_steps
            ),
            tool_calls,
        }
    }
}

/// Comma-separated tool names for an error frame.
fn tool_names(catalog: &ToolCatalog) -> String {
    catalog.specs().iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(", ")
}

/// Build the agentic system prompt: the operator framing + the tool catalog
/// (name/description/input_schema for each app-tool) + the emit contract (DD3).
fn build_agentic_system_prompt(framing: &str, catalog: &ToolCatalog) -> String {
    let mut tools_doc = String::new();
    for s in catalog.specs() {
        tools_doc.push_str(&format!(
            "- {} — {}\n  input_schema: {}\n",
            s.name, s.description, s.input_schema
        ));
    }
    format!(
        "{framing}\n\nYou can call these app-tools:\n{tools_doc}\n\
         To call a tool, reply with a SINGLE fenced ```json block and nothing else:\n\
         ```json\n{{\"tool\":\"<name>\",\"args\":{{ … }}}}\n```\n\
         After a tool runs you will be given its result; then call another tool or \
         finish. When you are done, reply with plain prose (NO fenced block) — that \
         plain reply is your final answer to the operator."
    )
}
```

`ChatRequest`, `ChatRunner`, `ToolDispatcher`, `ToolCallRequest`, `ToolCallResult`, `ConversationEngine`, `EngineReply`, `ToolCatalog`, `async_trait`, `Arc` are already imported at the top of `lib.rs`; add `use conversational_control::turn::ToolCall;`, `use serde_json::Value;` (from Task 1), and `use runtime::brake::Brake;` if not already present.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p agent-bus-app composite_engine_tests::two_step_loop_dispatches_once_then_finishes_with_a_composite_turn`
Expected: PASS — `test result: ok. 1 passed`.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/app/src/lib.rs
git commit -m "feat(app): AgenticChatEngine within-turn loop — tool-call then final answer (C1 task 2)"
```

---

### Task 3: Plain-prose first reply finishes immediately (no dispatch)

**Files:**
- Modify: `/Users/tim/projects/agent-bus-app/src-tauri/app/src/lib.rs` (extend `composite_engine_tests`)

- [ ] **Step 1: Write the failing test**

Append to `composite_engine_tests`. A pure-prose first reply must end the loop with zero dispatches and an empty `tool_calls`.

```rust
    #[tokio::test]
    async fn plain_prose_first_reply_finishes_with_no_dispatch() {
        let runner = Arc::new(FakeChatRunner::new(vec![reply("T-042 is in design; nothing to do.")]));
        let disp = Arc::new(FakeDispatcher::new());
        let brake = Arc::new(Brake::new());
        let eng = agentic(runner.clone(), disp.clone(), brake);

        let r = eng.respond("how is T-042 going?", &catalog()).await;

        assert_eq!(r.text, "T-042 is in design; nothing to do.");
        assert!(r.tool_calls.is_empty());
        assert_eq!(disp.received.lock().unwrap().len(), 0); // never dispatched
        assert_eq!(runner.received.lock().unwrap().len(), 1); // exactly one model call
    }
```

- [ ] **Step 2: Run test to verify it fails (or passes)**

Run: `cargo test -p agent-bus-app composite_engine_tests::plain_prose_first_reply_finishes_with_no_dispatch`
Expected: PASS immediately — Task 2's loop already returns on the first block-less reply. If it FAILS, the early-return in the loop is wrong; fix the implementation, not the test.

- [ ] **Step 3: (no new implementation)**

Behaviour is covered by Task 2's loop. This task locks it with a regression test.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p agent-bus-app composite_engine_tests`
Expected: PASS — `test result: ok. 5 passed` (3 from Task 1 + 2 loop tests).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/app/src/lib.rs
git commit -m "test(app): plain-prose reply finishes the agentic loop with no dispatch (C1 task 3)"
```

---

### Task 4: Unknown / malformed tool-call is fed back as an error and the model recovers

**Files:**
- Modify: `/Users/tim/projects/agent-bus-app/src-tauri/app/src/lib.rs` (extend `composite_engine_tests`)

- [ ] **Step 1: Write the failing test**

Append to `composite_engine_tests`. The model first asks for a tool **not in the catalog**; the engine must NOT dispatch it, must feed an error back, and the model recovers with a valid call, then finishes.

```rust
    #[tokio::test]
    async fn unknown_tool_call_is_fed_back_and_the_model_recovers() {
        let runner = Arc::new(FakeChatRunner::new(vec![
            // step 1: a tool that is NOT in the catalog
            reply("```json\n{\"tool\":\"frobnicate\",\"args\":{}}\n```"),
            // step 2: fed the error, the model corrects to a real tool
            reply("```json\n{\"tool\":\"inject_topic\",\"args\":{\"topic\":\"x\"}}\n```"),
            // step 3: fed the result, finishes
            reply("Injected x as T-1."),
        ]));
        let disp = Arc::new(
            FakeDispatcher::new().with("inject_topic", ToolCallResult::Ok { result: json!({"task_id":"T-1"}) }),
        );
        let brake = Arc::new(Brake::new());
        let eng = agentic(runner.clone(), disp.clone(), brake);

        let r = eng.respond("do the thing", &catalog()).await;

        assert_eq!(r.text, "Injected x as T-1.");
        // only the VALID tool reached the dispatcher; the unknown one did not
        assert_eq!(disp.received.lock().unwrap().len(), 1);
        assert_eq!(disp.received.lock().unwrap()[0].tool_name, "inject_topic");
        // only the dispatched (valid) call is embedded in the composite turn
        assert_eq!(r.tool_calls.len(), 1);
        // the rejection was fed back to the model on step 2
        let got = runner.received.lock().unwrap();
        assert_eq!(got.len(), 3);
        assert!(got[1].user_message.contains("unknown tool"));
        assert!(got[1].user_message.contains("frobnicate"));
    }

    #[tokio::test]
    async fn malformed_json_tool_call_is_fed_back_then_recovers() {
        let runner = Arc::new(FakeChatRunner::new(vec![
            // step 1: a fenced block that is NOT valid json
            reply("```json\n{ this is not json\n```"),
            // step 2: corrects, finishes
            reply("Sorry — nothing to do after all."),
        ]));
        let disp = Arc::new(FakeDispatcher::new());
        let brake = Arc::new(Brake::new());
        let eng = agentic(runner.clone(), disp.clone(), brake);

        let r = eng.respond("go", &catalog()).await;

        assert_eq!(r.text, "Sorry — nothing to do after all.");
        assert!(r.tool_calls.is_empty());
        assert_eq!(disp.received.lock().unwrap().len(), 0); // never dispatched garbage
        let got = runner.received.lock().unwrap();
        assert!(got[1].user_message.contains("not valid JSON"));
    }
```

- [ ] **Step 2: Run test to verify it fails (or passes)**

Run: `cargo test -p agent-bus-app composite_engine_tests::unknown_tool_call_is_fed_back_and_the_model_recovers composite_engine_tests::malformed_json_tool_call_is_fed_back_then_recovers`
Expected: PASS — Task 2's loop already feeds back unknown/malformed calls. If FAIL, the validation/feed-back branches are wrong; fix the implementation, not the tests.

- [ ] **Step 3: (no new implementation)**

Covered by Task 2's `Err(e)` and `catalog.by_name(..).is_none()` branches.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p agent-bus-app composite_engine_tests`
Expected: PASS — `test result: ok. 7 passed`.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/app/src/lib.rs
git commit -m "test(app): invalid tool-calls are fed back so the model recovers (C1 task 4)"
```

---

### Task 5: Max-steps cap stops cleanly; brake stops the loop before dispatch

**Files:**
- Modify: `/Users/tim/projects/agent-bus-app/src-tauri/app/src/lib.rs` (extend `composite_engine_tests`)

- [ ] **Step 1: Write the failing test**

Append to `composite_engine_tests`. (a) A model that **never** finishes (always emits a valid tool-call) must stop at `MAX_STEPS` with the step-limit message and exactly `MAX_STEPS` dispatches. (b) A brake that is **already on** must stop before any model call or dispatch.

```rust
    #[tokio::test]
    async fn loop_stops_at_max_steps_when_the_model_never_finishes() {
        // every reply is a valid tool-call; the model never returns plain prose.
        let endless = reply("```json\n{\"tool\":\"inject_topic\",\"args\":{\"topic\":\"x\"}}\n```");
        let runner = Arc::new(FakeChatRunner::new(vec![endless])); // clamps to last forever
        let disp = Arc::new(
            FakeDispatcher::new().with("inject_topic", ToolCallResult::Ok { result: json!({"task_id":"T"}) }),
        );
        let brake = Arc::new(Brake::new());
        let eng = agentic(runner.clone(), disp.clone(), brake);

        let r = eng.respond("loop forever", &catalog()).await;

        assert!(r.text.contains("step limit reached"));
        // bounded: exactly MAX_STEPS model calls and MAX_STEPS dispatches
        assert_eq!(runner.received.lock().unwrap().len(), MAX_STEPS);
        assert_eq!(disp.received.lock().unwrap().len(), MAX_STEPS);
        assert_eq!(r.tool_calls.len(), MAX_STEPS);
    }

    #[tokio::test]
    async fn braked_loop_stops_before_any_model_call_or_dispatch() {
        let runner = Arc::new(FakeChatRunner::new(vec![reply("should never be called")]));
        let disp = Arc::new(FakeDispatcher::new());
        let brake = Arc::new(Brake::new());
        brake.set_on("rate-limit");
        let eng = agentic(runner.clone(), disp.clone(), brake);

        let r = eng.respond("anything", &catalog()).await;

        assert!(r.text.contains("braked"));
        assert!(r.text.contains("rate-limit"));
        assert!(r.tool_calls.is_empty());
        assert_eq!(runner.received.lock().unwrap().len(), 0); // never called the model
        assert_eq!(disp.received.lock().unwrap().len(), 0);   // never dispatched
    }
```

- [ ] **Step 2: Run test to verify it fails (or passes)**

Run: `cargo test -p agent-bus-app composite_engine_tests::loop_stops_at_max_steps_when_the_model_never_finishes composite_engine_tests::braked_loop_stops_before_any_model_call_or_dispatch`
Expected: PASS — Task 2's loop already enforces `max_steps` and the pre-step brake check. If FAIL, the bound or the brake gate is wrong; fix the implementation, not the tests.

- [ ] **Step 3: (no new implementation)**

Covered by Task 2's `for _step in 0..self.max_steps` bound and the `if self.brake.is_on()` pre-step gate.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p agent-bus-app composite_engine_tests`
Expected: PASS — `test result: ok. 9 passed`.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/app/src/lib.rs
git commit -m "test(app): agentic loop is bounded and brake-aware (C1 task 5)"
```

---

### Task 6: `CompositeEngine` routes slash one-shot vs the agentic loop

**Files:**
- Modify: `/Users/tim/projects/agent-bus-app/src-tauri/app/src/lib.rs` (add `CompositeEngine`; extend `composite_engine_tests`)

- [ ] **Step 1: Write the failing test**

Append to `composite_engine_tests`. Route by leading `/` (after `trim_start`): slash → command branch; otherwise → agentic branch. Use `FakeEngine` for both branches to isolate the routing decision.

```rust
    fn command_branch() -> Arc<dyn ConversationEngine> {
        Arc::new(FakeEngine::new(vec![EngineReply { text: "CMD-BRANCH".into(), tool_calls: vec![] }]))
    }
    fn chat_branch() -> Arc<dyn ConversationEngine> {
        Arc::new(FakeEngine::new(vec![EngineReply { text: "CHAT-BRANCH".into(), tool_calls: vec![] }]))
    }

    #[tokio::test]
    async fn slash_line_routes_to_the_command_branch() {
        let eng = CompositeEngine::new(command_branch(), chat_branch());
        let r = eng.respond("/inject 03-scheduling", &catalog()).await;
        assert_eq!(r.text, "CMD-BRANCH");
    }

    #[tokio::test]
    async fn plain_line_routes_to_the_agentic_branch() {
        let eng = CompositeEngine::new(command_branch(), chat_branch());
        let r = eng.respond("kick off research", &catalog()).await;
        assert_eq!(r.text, "CHAT-BRANCH");
    }

    #[tokio::test]
    async fn leading_whitespace_before_slash_still_routes_to_command() {
        let eng = CompositeEngine::new(command_branch(), chat_branch());
        let r = eng.respond("   /inject x", &catalog()).await;
        assert_eq!(r.text, "CMD-BRANCH"); // trim_start before testing the leading '/'
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p agent-bus-app composite_engine_tests::slash_line_routes_to_the_command_branch`
Expected: FAIL — `cannot find type CompositeEngine`.

- [ ] **Step 3: Write the minimal implementation**

Add to `app/src/lib.rs` (beside `AgenticChatEngine`):

```rust
/// The god terminal's unified agentic engine (backlog C1). For one user message:
/// a leading `/` (after trim) is a one-shot slash command handled by the inner
/// command engine (parser -> RootDispatcher); anything else runs the inner
/// agentic chat loop. Routes by the SAME rule the parser uses (command.rs), so a
/// malformed `/cmd` surfaces as an error turn rather than entering the loop.
/// Holding each branch as `Arc<dyn ConversationEngine>` keeps them independently
/// fakeable. Lives at the composition root because both branches need root-only
/// imports; conversational_control stays kernel-only.
pub struct CompositeEngine {
    command: Arc<dyn ConversationEngine>,
    agentic: Arc<dyn ConversationEngine>,
}

impl CompositeEngine {
    pub fn new(command: Arc<dyn ConversationEngine>, agentic: Arc<dyn ConversationEngine>) -> Self {
        Self { command, agentic }
    }
}

#[async_trait]
impl ConversationEngine for CompositeEngine {
    async fn respond(&self, input: &str, catalog: &ToolCatalog) -> EngineReply {
        if input.trim_start().starts_with('/') {
            self.command.respond(input, catalog).await
        } else {
            self.agentic.respond(input, catalog).await
        }
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p agent-bus-app composite_engine_tests`
Expected: PASS — `test result: ok. 12 passed`.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/app/src/lib.rs
git commit -m "feat(app): CompositeEngine routes slash one-shot vs agentic loop (C1 task 6)"
```

---

### Task 7: End-to-end through `send_message_inner` — slash turn + agentic composite turn in one conversation

**Files:**
- Modify: `/Users/tim/projects/agent-bus-app/src-tauri/app/src/lib.rs` (extend `composite_engine_tests`)

- [ ] **Step 1: Write the failing test**

Append to `composite_engine_tests`. Drive the fully-wired `CompositeEngine` (real `CommandEngine` + real `AgenticChatEngine`) through `send_message_inner` + a real in-memory `ConversationStore`, proving both branches persist correctly in one conversation with alternation held, and that the agentic message records **one composite assistant turn** (DD2).

```rust
    use conversational_control::api::send_message_inner;
    use conversational_control::engine::CommandEngine;
    use conversational_control::store::ConversationStore;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn store_with_project() -> ConversationStore {
        let pool = SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        sqlx::query(include_str!("../migrations/001_initial.sql")).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO projects (id,name,root_path,created_at,updated_at) VALUES ('p','n','/p',0,0)")
            .execute(&pool).await.unwrap();
        ConversationStore::new(pool)
    }

    #[tokio::test]
    async fn composite_records_a_slash_turn_then_an_agentic_composite_turn() {
        let store = store_with_project().await;
        let cat = catalog();

        let disp = Arc::new(
            FakeDispatcher::new().with("inject_topic", ToolCallResult::Ok { result: json!({"task_id":"T-3"}) }),
        );
        let command: Arc<dyn ConversationEngine> = Arc::new(CommandEngine::new(disp.clone() as Arc<dyn ToolDispatcher>));

        let runner = Arc::new(FakeChatRunner::new(vec![
            reply("on it\n```json\n{\"tool\":\"inject_topic\",\"args\":{\"topic\":\"y\"}}\n```"),
            reply("Injected y as T-3."),
        ]));
        let brake = Arc::new(Brake::new());
        let agentic: Arc<dyn ConversationEngine> = Arc::new(agentic(runner.clone(), disp.clone(), brake));

        let engine: Arc<dyn ConversationEngine> = Arc::new(CompositeEngine::new(command, agentic));

        // 1) a slash command -> command branch -> one tool-call turn
        let convo = send_message_inner("p", &cat, engine.as_ref(), &store, "/inject 03-x", 500).await.unwrap();
        assert_eq!(convo.turns.len(), 2);
        assert!(convo.turns[1].text.contains("Done"));
        assert_eq!(convo.turns[1].tool_calls.len(), 1);

        // 2) a plain line on the SAME conversation -> agentic branch -> ONE composite turn
        let convo = send_message_inner("p", &cat, engine.as_ref(), &store, "kick off y", 600).await.unwrap();
        assert_eq!(convo.turns.len(), 4); // alternation held across both branches (DD2)
        assert_eq!(convo.turns[3].text, "Injected y as T-3.");
        assert_eq!(convo.turns[3].tool_calls.len(), 1); // the embedded tool-call
        assert_eq!(convo.turns[3].tool_calls[0].request.tool_name, "inject_topic");

        // persisted
        let reloaded = store.load("p").await.unwrap().unwrap();
        assert_eq!(reloaded.turns.len(), 4);
    }
```

- [ ] **Step 2: Run test to verify it fails (or passes)**

Run: `cargo test -p agent-bus-app composite_engine_tests::composite_records_a_slash_turn_then_an_agentic_composite_turn`
Expected: PASS — all production pieces exist after Task 6; this proves the in-test composition end-to-end. If FAIL, inspect alternation/recording, not the test.

- [ ] **Step 3: (no new implementation)**

Pure integration test over existing pieces.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p agent-bus-app composite_engine_tests`
Expected: PASS — `test result: ok. 13 passed`.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/app/src/lib.rs
git commit -m "test(app): CompositeEngine end-to-end through send_message_inner (C1 task 7)"
```

---

### Task 8: Wire `CompositeEngine` as the terminal's active engine + un-`_`-prefix the dispatcher

**Files:**
- Modify: `/Users/tim/projects/agent-bus-app/src-tauri/app/src/lib.rs` (the `setup` closure + the `CommandEngine` import)

- [ ] **Step 1: Drop the `#[allow(unused_imports)]` on the `CommandEngine` import**

Lines ~18–19, from:
```rust
#[allow(unused_imports)]
use conversational_control::engine::CommandEngine;
```
to:
```rust
use conversational_control::engine::CommandEngine;
```

- [ ] **Step 2: Un-`_`-prefix the dispatcher**

Line ~454, from:
```rust
                let _dispatcher: Arc<dyn ToolDispatcher> = Arc::new(RootDispatcher {
                    runtime: runtime_state_arc.clone(),
                    usage: usage_state_arc.clone(),
                    app: handle.clone(),
                });
```
to:
```rust
                let dispatcher: Arc<dyn ToolDispatcher> = Arc::new(RootDispatcher {
                    runtime: runtime_state_arc.clone(),
                    usage: usage_state_arc.clone(),
                    app: handle.clone(),
                });
```

- [ ] **Step 3: Swap the active engine to `CompositeEngine`**

Lines ~467–476, from:
```rust
                let engine: Arc<dyn conversational_control::engine::ConversationEngine> =
                    Arc::new(LlmEngine::new(
                        chat_runner.clone(),
                        project_id.clone(),
                        "You are the god terminal for the Agent Bus app. Answer the operator's \
                         questions about the pipeline, tasks, and usage concisely."
                            .into(),
                        "claude-opus-4-8".into(),
                        8192,
                    ));
```
to:
```rust
                // The slash branch: parser -> RootDispatcher (restores the full
                // slash tool surface). The agentic branch: the bounded,
                // brake-aware within-turn loop over the same dispatcher + the
                // chat runner + the catalog. CompositeEngine routes by leading '/'.
                let command_engine: Arc<dyn conversational_control::engine::ConversationEngine> =
                    Arc::new(CommandEngine::new(dispatcher.clone()));
                let agentic_engine: Arc<dyn conversational_control::engine::ConversationEngine> =
                    Arc::new(AgenticChatEngine::new(
                        chat_runner.clone(),
                        dispatcher.clone(),
                        brake.clone(),
                        project_id.clone(),
                        "You are the god terminal for the Agent Bus app. Help the operator run \
                         and inspect the pipeline (tasks, gates, usage, the brake). Be concise."
                            .into(),
                        "claude-opus-4-8".into(),
                        8192,
                    ));
                let engine: Arc<dyn conversational_control::engine::ConversationEngine> =
                    Arc::new(CompositeEngine::new(command_engine, agentic_engine));
```

(`brake` is the `Arc<Brake>` bound at line ~396; `chat_runner` and `dispatcher` are in scope. `LlmEngine` and `mod llm_engine_tests` remain — D9.)

- [ ] **Step 4: Build clean (no `unused` warnings)**

Run: `cargo build -p agent-bus-app`
Expected: PASS, with **no `unused` warnings** for `dispatcher` or the `CommandEngine` import. Confirm there is no remaining `_dispatcher` and no `#[allow(unused_imports)]` on `CommandEngine`.

- [ ] **Step 5: Run the full app test suite**

Run: `cargo test -p agent-bus-app`
Expected: PASS — `composite_engine_tests` (13) plus the pre-existing `llm_engine_tests`, `design_session_tests`, `revision_reader_tests`, `migration_tests` all green.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/app/src/lib.rs
git commit -m "feat(app): make CompositeEngine the terminal's active engine; restore dispatcher (C1 task 8)"
```

---

### Task 9: Full-suite green + acyclic / kernel-only verification

**Files:** none (verification only).

- [ ] **Step 1: Run the whole workspace**

Run: `cargo test` (from `/Users/tim/projects/agent-bus-app/src-tauri/`)
Expected: every crate green, specifically including:
- `conversational_control` — all existing `CommandEngine`/`FakeEngine`/parser/dispatch/conversation/api tests pass **untouched** (this crate was not edited).
- `llm_chat` — `FakeChatRunner` + chat tests pass untouched.
- `app` — `composite_engine_tests` (13) + the four pre-existing modules.

- [ ] **Step 2: Verify `conversational_control` stays a pure customer (kernel-only)**

```bash
git diff --stat src-tauri/conversational_control            # MUST be empty
cargo tree -p conversational_control -e normal              # deps = agent_bus_core + infra (serde/async-trait/sqlx/uuid/thiserror) only;
                                                            # NO llm_chat, NO runners, NO runtime, NO app
cargo tree -p conversational_control -i llm_chat 2>&1 | tail -1   # expect: no match
cargo tree -p conversational_control -i runtime 2>&1 | tail -1    # expect: no match
```
Expected: empty diff; dep tree kernel-only; no reverse edge from `llm_chat`/`runtime` into `conversational_control`.

- [ ] **Step 3: Verify acyclicity of the new composition**

```bash
cargo tree -p agent-bus-app -e normal | grep -E 'conversational_control|llm_chat|runtime'   # all three appear under app
cargo tree -p llm_chat -i conversational_control 2>&1 | tail -1                              # expect: no match (no cycle)
```
Expected: `app` depends on all three; no crate depends back on `app`; no cycle introduced. `AgenticChatEngine` composing the dispatcher + runner + brake lives only in `app`.

- [ ] **Step 4: Commit (verification note)**

```bash
git commit --allow-empty -m "test(app): full suite green + conversational_control stays kernel-only (C1 task 9)"
```

---

## Test-by-decision traceability

- **D1 (root-only placement; kernel-only customer)** → structs added in `app` (Tasks 2/6); `conversational_control` diff empty + `cargo tree` checks (Task 9). ✓
- **D2 (route by leading `/` after `trim_start`)** → `slash_*`/`plain_*`/`leading_whitespace_*` (Task 6). ✓
- **D3 (reuse `CommandEngine` for slash)** → real `CommandEngine` branch in Task 7; import un-suppressed in Task 8. ✓
- **D4/DD1 (bounded loop)** → `loop_stops_at_max_steps_when_the_model_never_finishes` (Task 5). ✓
- **D4/DD4 (brake before each step; rate-limit/error stops)** → `braked_loop_stops_before_any_model_call_or_dispatch` (Task 5); chat-error path returns `[terminal error] {e}` (Task 2 loop, exercised by the error frame). ✓
- **D5/DD2 (one composite Turn)** → `two_step_loop_…_composite_turn` (Task 2), `composite_records_…_agentic_composite_turn` (Task 7, alternation held + embedded tool-call persisted). ✓
- **D7/DD3 (fenced `{tool,args}`; plain prose = done)** → `extracts_*`/`plain_prose_*` (Tasks 1/3). ✓
- **DD5 (results fed back; invalid calls fed back, recover)** → `two_step_…` feed-back assertions (Task 2), `unknown_tool_call_…`/`malformed_json_…` (Task 4). ✓
- **D8 (one file touched outside tests; dispatcher reachable; no new dep)** → un-`_`-prefix + swap (Task 8); `disp.received` asserts dispatch (Tasks 2/4); `app/Cargo.toml` unchanged (Task 9). ✓
- **D9 (`LlmEngine` + `llm_engine_tests` stay green; slash surface restored)** → full suite (Task 9). ✓

No placeholders; every code step shows full code. Type/method names (`CompositeEngine::new(command, agentic)`, `AgenticChatEngine::new(runner, dispatcher, brake, dialogue_id, framing, model, budget)`, `MAX_STEPS`, `extract_tool_call_block`, `ParsedToolCall`, `build_agentic_system_prompt`, `tool_names`, `CommandEngine::new`, `FakeDispatcher::{new,with}`, `FakeChatRunner::new`, `send_message_inner`) are consistent across tasks.

---

## DDD vet — DONE (2026-06-23)

**A DDD-council vet has run: `docs/vet-composite-engine-2026-06-23.md` — verdict: C1 PLAN READY FOR IMPLEMENTATION.** Confirmations: (a) the composite Turn preserves alternation and the loop's internal result round-trips reach the `llm_chat` session only (never recorded as fake user Turns in the `Conversation` aggregate); (b) `ToolCall` already carries `result: Option<ToolCallResult>` — **no aggregate change needed**; (c) `conversational_control` stays kernel-only and acyclic. Four findings, all low/operator-gated, none blocking: F1 duplicate extractor (→ VF1 fork above), F2 `ParsedToolCall` naming (→ VF2 fork above), F3 history-budget estimate for fat turns (→ Roadmap follow-up), F4 brake-in-flight semantics (→ DD4 clarification, applied).

The two things the ubiquitous language cares about, both confirmed sound by the vet:

1. **Conversation turn semantics.** DOMAIN.md defines a **Turn** as "one user message + one assistant response (with embedded tool-calls)". The agentic loop makes that "embedded tool-calls" clause carry real weight: N supplier round-trips now collapse into **one composite assistant Turn** (DD2). Worth a council confirmation that "one user message → one composite assistant Turn embedding N tool-calls" is the intended reading (it appears to be — the plural "tool-calls" and the `Vec<ToolCall>` on `Turn` both point that way), and that the history-budget estimate (`Turn::estimated_tokens`, +8/tool-call) is acceptable for fatter turns.
2. **A new supplier-dispatching loop at the customer's composition root.** Conversational Control is the *customer of all six*; the loop now drives suppliers iteratively (model → `RootDispatcher` → supplier → back to model) within one turn. The customer/supplier relationship and the kernel-only boundary are preserved (the loop lives in `app`, not in `conversational_control`), but the council should confirm the **brake** (a Runtime concept) being consulted by the terminal's loop is the right cross-context read, and that "tool-call protocol / rate limit / completion vs streaming" (AI Engineer's vocabulary) is honored by DD3/DD4.

A vet is **warranted** (it touches aggregate semantics + adds a cross-context loop) but **low-risk** (no new aggregate, no new edge into the kernel, no trait change). Suggested: a focused council pass on DD1–DD5 + the Turn-recording decision, before Task 1.

---

## Roadmap — what this unblocks and what is deliberately deferred

- **Streaming the agentic loop** — surface intermediate tool-calls/results to the terminal UI as they happen (incremental `chat.delta` + a per-step `tool.called`/`tool.returned` event) instead of only the final composite turn. Orthogonal to correctness; the composite Turn is still the persisted record.
- **History-budget estimate for fat composite turns (vet F3)** — `Turn::estimated_tokens` (`conversational_control/src/turn.rs`) charges only `tool_name.len()/4 + 8` per embedded tool-call, counting neither `args` nor the `result` JSON. Composite agentic turns are materially fatter than v1 slash turns, so the **History budget** invariant will under-count and let real context grow past budget. Follow-up: fold a coarse `args` + `result` size (e.g. `serde_json` length / 4 per call) into the estimate. This is a `turn.rs` change inside `conversational_control`, so it is deliberately **out of scope** for C1 (which leaves the kernel untouched) and tracked here as a future kernel follow-up.
- **Usage attribution per step** — each loop step has `ChatReply.usage`; sum them and map onto `agent_bus_core::UsageEvent` into the existing `UsageSink`. Today the loop discards `usage`; wiring it is additive.
- **Native tool-use protocol** — replace the fenced-JSON convention (DD3) with the provider's structured tool-use blocks once `llm_chat` exposes them; `AgenticChatEngine`'s extraction + validation seam is the single place to swap.
- **Per-supplier brake granularity / cancellation** — today the brake is global and checked between steps; a cooperative cancel mid-dispatch is a follow-up.
- **Slash autocompletion / help in the terminal UI** — the parser already enumerates the verb mapping; a UI affordance listing `/inject`, `/approve`, `/brake`, … is a frontend follow-up, not an engine change.

---

## Execution handoff

Plan complete and saved to `/Users/tim/projects/agent-bus-app/plans/2026-06-23-plan-composite-engine.md`. Two execution options:

1. **Subagent-Driven (recommended)** — dispatch a fresh subagent per task, review between tasks, fast iteration (REQUIRED SUB-SKILL: superpowers:subagent-driven-development).
2. **Inline Execution** — execute tasks in this session using superpowers:executing-plans, batch execution with checkpoints.

**Before either:** rule on DD1–DD5 plus the two vet forks VF1/VF2 (Open design decisions block). The DDD vet has already run — `docs/vet-composite-engine-2026-06-23.md`, verdict **C1 PLAN READY FOR IMPLEMENTATION**.
