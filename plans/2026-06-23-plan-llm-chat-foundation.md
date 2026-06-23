# Agent Bus App — Plan: `llm_chat` foundation (sub-project 1 of the brainstorming wizard)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a new `llm_chat` anti-corruption-layer crate that provides **multi-turn**, reply-text Claude dialogue (`ChatRunner` trait + `ClaudeChatRunner` over `claude --print --output-format stream-json [--resume]`, behind an injectable spawner, with internal session continuity), and rewire the god terminal to use it via an `LlmEngine` adapter wired at the composition root — without breaking the existing command-driven terminal or any existing test.

**Architecture:** One new Cargo workspace member, `llm_chat`, depending **only** on `agent_bus_core` (kernel) + infra crates (`async-trait`, `serde`, `serde_json`, `tokio`, `uuid`). It mirrors the proven `runners` seam playbook: a pure arg-builder (`command.rs`), a pure stream-json parser tuned for *assistant text + usage* (not verdicts), a `ChatRunner` trait held as `Arc<dyn ChatRunner>`, a real `ClaudeChatRunner` whose only side effect (spawning `claude`) is isolated behind an injectable `SpawnFn`, an internal `session.rs` `dialogue_id → claude_session_id` map (the F3 seal — no `session_id` ever crosses the ACL boundary), and a `FakeChatRunner` test double. The god terminal's `ConversationEngine` trait gains a real adapter, `LlmEngine`, constructed with an `Arc<dyn ChatRunner>` and wired at the **composition root** (the `app` crate, the only place that may import both `llm_chat` and `conversational_control`) — exactly the pattern `RootDispatcher` already uses. `conversational_control` stays kernel-only; it never learns about `llm_chat`.

**Tech Stack:** Rust (async-trait, tokio, serde, serde_json, thiserror, uuid), the existing `agent_bus_core` kernel, Tauri 2 composition root.

**Source spec:** `docs/superpowers/specs/2026-06-23-llm-chat-foundation-design.md` (brainstormed + DDD-vetted 2026-06-23). Honour every decision, especially F1/F2/F3 below.

**DDD anchor (DOMAIN.md + the council vet, 2026-06-23):**
- **F2 [high] — circular-dependency by design.** The shared chat capability cannot live in `conversational_control` (Pipeline Authoring would become a customer → cycle) nor in `runners` (`runners → pipeline` already, so `pipeline → runners → pipeline`). **Resolution:** a dedicated `llm_chat` crate depending only on `agent_bus_core`. Acyclic. Asserted by a `cargo tree -p llm_chat` check in the final task.
- **F1 [medium] — one-name-two-concepts.** The wizard's per-step dialogue is **not** the terminal's `Conversation` aggregate. `llm_chat` shares only the chat *capability*. It has **no** `Conversation` type and never imports `conversational_control`. The wizard's Design Session is sub-project 3, out of scope here.
- **F3 [medium] — leaky ACL.** `session_id` / `--resume` are Claude-CLI idioms and must not cross the ACL boundary. **Resolution:** session continuity is managed **inside** `llm_chat` (`session.rs`); callers pass a stable `dialogue_id`; `ChatReply` carries only `text` + `usage` — **no `session_id`**.

---

## Orientation — the real code this plan builds on (Plans 1–6 merged)

Read these before starting; every task references them.

- **The seam playbook to mirror — the `runners` crate.**
  - `src-tauri/runners/src/output.rs` — defines `RunnerUsage { model, input_tokens, output_tokens, cache_creation, cache_read }` (all `u64` except `model: String`), `RunnerError { RateLimited(String), Spawn(String), NoResult, Other(String) }` with `is_rate_limited()`, and the object-safe `#[async_trait] trait Runner` held as `Arc<dyn Runner>`. **`RunnerUsage` lives in the `runners` crate, NOT in `agent_bus_core`** — see Decision D2 for how `llm_chat` honours "reuse the usage shape" without depending on `runners`.
  - `src-tauri/runners/src/command.rs` — `pub const CLAUDE_BIN: &str = "claude"` and `pub fn build_args(req: &InvocationRequest) -> Vec<String>` building `--print --output-format stream-json --append-system-prompt <p> --settings <path> [--add-dir …] --permission-mode acceptEdits --model <m> --max-thinking-tokens <n> <user_message>`. Pure; asserted exactly in tests with no subprocess. `llm_chat` builds a **simpler** line (no settings/add-dir/permission-mode; adds `--resume <session>` on continuation).
  - `src-tauri/runners/src/claude_cli.rs` — the injectable-spawner pattern: `pub type SpawnFn = Box<dyn Fn(&[String]) -> Result<String, RunnerError> + Send + Sync>`; `ClaudeCliRunner { spawn: SpawnFn }` with `new()` (real `std::process::Command::new(CLAUDE_BIN).args(args).output()`) and `with_spawner(spawn)` (test injection). `llm_chat`'s `ClaudeChatRunner` copies this shape exactly.
  - `src-tauri/runners/src/stream_json.rs` — the inbound parser. Reads `type:"system"` (captures `model`), `type:"assistant"` (accumulates `message.content[].text` blocks + per-message `message.usage`), `type:"result"` (authoritative totals + fallback text), and detects rate-limit on `type:"error"` / `is_error:true` when the message contains `rate`/`429`/`quota`. **`llm_chat` reuses this exact JSON shape but (a) drops all verdict/artifact parsing and (b) additionally captures `session_id` from the init `system` line and the `result` line.**
  - `src-tauri/runners/src/fixtures/stream-json-sample.txt` — the committed fixture shape. The init line is `{"type":"system","subtype":"init","session_id":"sess-abc","model":"claude-opus-4-7"}`; the `result` line carries `"session_id":"sess-abc"`. `llm_chat` commits its own two fixtures (a first turn and a follow-up turn) in the same shape.
- **The terminal engine seam — `conversational_control`.**
  - `src-tauri/conversational_control/src/engine.rs` — `#[derive(Debug, Clone, PartialEq)] pub struct EngineReply { pub text: String, pub tool_calls: Vec<ToolCall> }`; `#[async_trait] pub trait ConversationEngine: Send + Sync { async fn respond(&self, input: &str, catalog: &ToolCatalog) -> EngineReply; }`; the deterministic `CommandEngine` (parse → dispatch → compose) and `FakeEngine` (seeded replies). **`LlmEngine` is a NEW impl of this trait — it lives at the composition root, NOT in this crate (which must not depend on `llm_chat`).**
  - `src-tauri/conversational_control/src/api.rs` — `TerminalState { catalog, engine: Arc<dyn ConversationEngine>, store, project_id }` and `send_message_inner(...)`: load/create `Conversation` → append user `Turn` → `engine.respond(input, catalog)` → append assistant `Turn` (with `reply.tool_calls`) → `store.save` → return the `Conversation`. **The send flow is engine-agnostic; swapping the engine changes nothing here.** The `Conversation`'s `session_id` field is the conversation's *own* persistence id (a UUID), unrelated to any Claude session — do not confuse it with the F3-sealed claude session id.
  - `src-tauri/conversational_control/src/conversation.rs` — the `Conversation` aggregate: `append` enforces alternation (first turn must be `User`; roles alternate), bumps `last_message_at`, truncates to `history_budget_tokens`. `LlmEngine` produces only the assistant text + (empty) tool-calls; the aggregate's invariants are untouched.
  - `src-tauri/conversational_control/src/catalog.rs` (`ToolCatalog`) and `src-tauri/conversational_control/src/turn.rs` (`Turn`, `Role`, `ToolCall`) — unchanged by this plan; `LlmEngine::respond` ignores the catalog (free-form chat has no slash-command parse) and returns `tool_calls: vec![]` in v1.
- **The composition root — `src-tauri/app/src/lib.rs`.** The only module importing every context. Constructs the sqlx pool, every context's state, `RootDispatcher` (the existing `ToolDispatcher` impl), the `ToolCatalog` union, and today wires `let engine: Arc<dyn ConversationEngine> = Arc::new(CommandEngine::new(dispatcher.clone()));` then `handle.manage(TerminalState { … engine … })`. **This plan adds the `LlmEngine` adapter struct here and switches that one `let engine` line to construct it over an `Arc<dyn ChatRunner>` (a `ClaudeChatRunner`).** Per-team worker loops already build `Arc<dyn runners::output::Runner> = Arc::new(ClaudeCliRunner::new())` — the chat runner is the analogous one-liner.
- **Workspace wiring.** `src-tauri/Cargo.toml` `[workspace] members = [...]` lists every crate; `ddd-council.json` maps each context to source paths. Both get one new entry.
- **The kernel.** `src-tauri/agent_bus_core/src/lib.rs` re-exports `ids`, `verdict`, `runner`, `tool_protocol`, `usage`. `usage.rs` has `UsageEvent` (a *different*, attribution-shaped struct: `ts/team_id/task_id/model/…tokens`) — NOT the right shape for a chat reply. See D2.

---

## Spec anchors (source of truth — do not invent)

- **`llm_chat` is a NEW crate depending only on `agent_bus_core`** (F2). No `pipeline`, `runtime`, `runners`, `conversational_control`, `workspace`, or `usage_telemetry` edge. Proven by `cargo tree -p llm_chat` in Task 9.
- **The boundary types** (spec §"The boundary types"):
  ```rust
  pub struct ChatRequest { pub dialogue_id: String, pub system_prompt: String, pub user_message: String, pub model: String, pub thinking_budget: u32 }
  pub struct ChatReply  { pub text: String, pub usage: ChatUsage }   // NO session_id (F3)
  #[async_trait] pub trait ChatRunner: Send + Sync { async fn chat(&self, req: &ChatRequest) -> Result<ChatReply, ChatError>; }
  ```
  `ChatError` mirrors `RunnerError`'s classes: `RateLimited(String) / Spawn(String) / NoResult / Other(String)`, with `is_rate_limited()`.
- **Distinct trait, not a second method on `Runner`** (vet) — Runtime keeps depending only on `runners::Runner`; chat consumers depend only on `llm_chat::ChatRunner`.
- **Session continuity is internal** (F3, spec §"The boundary types" final bullet): `session.rs` keeps an in-process `dialogue_id → claude_session_id` map. First turn for a `dialogue_id` omits `--resume` and **captures the session id from the stream-json init event**; later turns pass `--resume <session>`. A lost session falls back to a fresh session (the caller's `dialogue_id` is unchanged). None of this is visible outside the crate.
- **Consumer 1 — the god terminal** (spec §"Consumer 1"): `conversational_control` stays a pure customer (kernel-only). Its `ConversationEngine` trait gains an `LlmEngine` adapter wired at the composition root with an `Arc<dyn ChatRunner>`, using **the conversation's id as the `dialogue_id`**. The `Conversation` aggregate / store / invariants are unchanged; `LlmEngine` just produces the assistant turn for a user message. Non-streaming v1 (await the full `--print` response).
- **Consumer 2 — the wizard** is deferred to sub-project 3. Not built here.
- **Error handling** (spec §"Error handling"): rate-limit → `ChatError::RateLimited` (surfaced as an error turn; the root may set the brake reusing existing behaviour); spawn failure → `ChatError::Spawn` (clear error turn, no crash); parse/no result → `ChatError::NoResult`. No verdict semantics anywhere.
- **Testing** (spec §"Testing"): pure arg-construction test (incl. `--resume`); assistant-text + usage parsing from committed fixtures (first-turn init with session id; follow-up turn); session-map unit test (first omits `--resume`, second includes it, lost session falls back); `FakeChatRunner` drives `LlmEngine` end-to-end (turn appended, alternation held, persisted) with canned replies; `cargo tree -p llm_chat` kernel-only assertion. **All green with no live `claude` binary.**
- **DOMAIN.md updates** (spec §"DOMAIN.md updates", applied in Task 9): add `llm_chat` as a second ACL alongside Runners; add **Design Session** to Pipeline Authoring's ubiquitous language (stub; full def lands sub-project 3); register `llm_chat` in `ddd-council.json`.

---

## File structure (created / modified)

```
src-tauri/
├── Cargo.toml                                  M  [workspace] members += "llm_chat"
├── llm_chat/                                   A  the new ACL crate (kernel-only deps)
│   ├── Cargo.toml                              A  deps: agent_bus_core, async-trait, serde, serde_json, tokio, uuid  (NO supplier/customer crates)
│   └── src/
│       ├── lib.rs                              A  module wiring + re-exports + crate doc (dependency rule)
│       ├── chat.rs                             A  ChatUsage, ChatRequest, ChatReply, ChatError, ChatRunner trait
│       ├── command.rs                          A  build_chat_args(req, resume: Option<&str>) -> Vec<String>  (pure)
│       ├── stream_json.rs                      A  ChatAccumulator + parse_chat_stream -> (ChatReply, Option<session_id>)
│       ├── session.rs                          A  SessionMap: dialogue_id -> claude_session_id (the F3 seal)
│       ├── claude_cli.rs                       A  ClaudeChatRunner over SpawnFn; ties command+session+stream_json together
│       ├── fake.rs                             A  FakeChatRunner test double (seeded replies + recorded requests)
│       └── fixtures/
│           ├── chat-first-turn.txt             A  stream-json: system(init, session_id) + assistant + result
│           └── chat-follow-up.txt              A  stream-json: a second turn's response
├── app/
│   └── src/
│       ├── lib.rs                              M  add LlmEngine adapter; build Arc<dyn ChatRunner>; switch the terminal engine line
│       └── Cargo.toml                          M  dependency llm_chat = { path = "../llm_chat" }
DOMAIN.md                                       M  llm_chat as 2nd ACL; Design Session stub in Pipeline Authoring
ddd-council.json                                M  register llm-chat context paths
```

---

## Decisions (resolved ambiguities — autonomous)

- **D1 — `llm_chat` depends only on `agent_bus_core` + infra (F2).** Its `Cargo.toml` lists `agent_bus_core`, `async-trait`, `serde`, `serde_json`, `tokio`, `uuid` — and **no** project supplier/customer crate. The composition-root wiring is the only place `llm_chat` and `conversational_control` meet. Proven by `cargo tree -p llm_chat` (Task 9) showing no `runners`/`pipeline`/`runtime`/`conversational_control` node.
- **D2 — `llm_chat` defines its own `ChatUsage`, not `RunnerUsage`.** The spec says "reuse `agent_bus_core` usage shape", but the only usage type in the kernel is `UsageEvent` (attribution-shaped: `ts/team_id/task_id`), which is wrong for a chat reply, and `RunnerUsage` lives in the `runners` crate (which D1/F2 forbids importing). **Resolution:** `llm_chat::chat::ChatUsage` is a small struct **structurally identical to `RunnerUsage`** (`model: String`, `input_tokens/output_tokens/cache_creation/cache_read: u64`), defined locally in `llm_chat`. The composition root maps it to whatever Telemetry shape it needs (same way the terminal already bridges types at the root). This keeps the ACL kernel-only and the field set identical, satisfying the spec's intent (a usage shape callers already recognise) without the forbidden dependency.
- **D3 — The terminal switches to `LlmEngine` without breaking `CommandEngine` tests.** `CommandEngine`, `FakeEngine`, and every existing `conversational_control` test stay **exactly as they are** — they live in the `conversational_control` crate and never reference `llm_chat`, so they keep compiling and passing untouched. `LlmEngine` is a *new, third* `ConversationEngine` impl defined in the `app` crate. The only behavioural change is **one line** at the composition root: `let engine: Arc<dyn ConversationEngine> = Arc::new(CommandEngine::new(dispatcher.clone()));` becomes `Arc::new(LlmEngine::new(chat_runner.clone()));`. The `dispatcher` is still constructed (other root code uses it) so nothing else moves. Because the swap is one isolated line at the root and `LlmEngine` is tested in-crate with `FakeChatRunner` + the real `ConversationStore`/`Conversation`, no existing test changes. (v1 free-form chat performs no tool dispatch; the slash-command path is a v1.1 merge of the two engines — out of scope here, noted in Roadmap.)
- **D4 — `dialogue_id` is the conversation's `session_id` field.** `send_message_inner` loads/creates a `Conversation` whose `session_id` is a stable per-project UUID (the conversation's own id, persisted in the `conversations.id` column). `LlmEngine` cannot see the `Conversation` (it only gets `input` + `catalog` through `ConversationEngine::respond`), so the conversation id is threaded to the engine at construction is impossible per-turn. **Resolution:** `LlmEngine` holds the `project_id` String it was constructed with at the root (the root already knows `project_id`) and uses **`project_id` as the stable `dialogue_id`** — one terminal conversation per project (the singleton invariant), so `project_id` is exactly the stable per-dialogue key the spec wants. This needs no change to the `ConversationEngine` trait signature, keeping `conversational_control` untouched (D3).
- **D5 — Internal session capture from stream-json.** `parse_chat_stream` returns `(ChatReply, Option<String>)` where the second element is the claude `session_id` read from the `type:"system"` init line (preferred) or the `type:"result"` line (fallback). `ClaudeChatRunner::chat` (a) looks up `dialogue_id` in the `SessionMap` to decide whether to pass `--resume`, (b) parses the stream, (c) on success stores the captured `session_id` back under `dialogue_id`. The `session_id` is **dropped before returning** — `ChatReply` has no such field (F3).
- **D6 — Lost-session fallback.** If a `--resume <session>` invocation fails (the parser yields `NoResult`/`Other`, or the spawner errs in a way that suggests a dead session), `ClaudeChatRunner` retries **once** without `--resume` (a fresh session), and on success records the new session id under the same `dialogue_id`. The caller's `dialogue_id` is unchanged and the caller never learns a session was rotated. A `RateLimited` or `Spawn` error is **not** retried (those are not lost-session conditions) — it propagates. Tested in Task 8.
- **D7 — Non-streaming v1.** Each `chat` call awaits the full `--print` stdout, parses it whole (reusing the `parse_chat_stream` whole-buffer approach the `runners` crate uses), and returns one `ChatReply`. Incremental token streaming is explicitly out of scope (Roadmap).
- **D8 — `LlmEngine::respond` ignores the catalog and returns no tool-calls in v1.** Free-form chat in v1 produces assistant prose only (`tool_calls: vec![]`). Tool-use via the catalog is the v1.1 merge of `CommandEngine` + `LlmEngine` (Roadmap). This keeps the assistant turn valid for the aggregate's invariants with zero catalog coupling.

---

## Task 1: Scaffold the `llm_chat` crate + boundary types (`chat.rs`)

**Files:**
- Modify: `src-tauri/Cargo.toml` (add `"llm_chat"` to `[workspace] members`)
- Create: `src-tauri/llm_chat/Cargo.toml`
- Create: `src-tauri/llm_chat/src/lib.rs`
- Create: `src-tauri/llm_chat/src/chat.rs`

- [ ] **Step 1: Add the crate to the workspace members**

In `src-tauri/Cargo.toml`, change the `members` line to include `"llm_chat"` (append before `"conversational_control"`):

```toml
members = ["app", "agent_bus_core", "workspace", "pipeline", "runners", "runtime", "review", "usage_telemetry", "llm_chat", "conversational_control"]
```

- [ ] **Step 2: Create the crate manifest (kernel-only deps — D1/F2)**

Create `src-tauri/llm_chat/Cargo.toml`:

```toml
[package]
name = "llm_chat"
version.workspace = true
edition.workspace = true

[dependencies]
agent_bus_core = { path = "../agent_bus_core" }
serde = { workspace = true }
serde_json = { workspace = true }
tokio = { workspace = true }
async-trait = { workspace = true }
uuid = { workspace = true }

[dev-dependencies]
tokio = { workspace = true }
```

(No `thiserror` — `ChatError` uses a hand-written `Display`/`Error` to keep the dep set minimal; mirrors how small kernel-adjacent crates stay lean. No supplier/customer crate.)

- [ ] **Step 3: Create the crate doc + module wiring**

Create `src-tauri/llm_chat/src/lib.rs`:

```rust
//! llm_chat — the multi-turn Claude dialogue anti-corruption layer (ACL).
//!
//! Unlike `runners` (one-shot, verdict-shaped worker invocations), this ACL
//! provides a *chat*: many turns over a stable dialogue, returning the
//! assistant's *reply text* + usage. Two consumers use it without coupling to
//! each other — the god terminal (Conversational Control) and the brainstorming
//! wizard (Pipeline Authoring, sub-project 3).
//!
//! Dependency rule (F2): this crate depends on NOTHING project-internal except
//! `agent_bus_core` (the kernel). It must never import `runners`, `pipeline`,
//! `runtime`, `usage_telemetry`, `workspace`, or `conversational_control`.
//!
//! Boundary discipline (F3): Claude-CLI idioms (`session_id`, `--resume`) are
//! sealed inside this crate. Callers pass a stable `dialogue_id`; `ChatReply`
//! carries only domain-shaped data (text + usage). Session continuity lives in
//! `session.rs` and never crosses out.

pub mod chat;
pub mod command;
pub mod stream_json;
pub mod session;
pub mod claude_cli;
pub mod fake;

pub use chat::*;
```

- [ ] **Step 4: Write the failing test for the boundary types**

Create `src-tauri/llm_chat/src/chat.rs` with ONLY a test module first (so it fails to compile, proving the test drives the impl):

```rust
//! The llm_chat boundary types — the Runtime/domain vocabulary that crosses the
//! ACL. Nothing here mentions stream-json, CLI flags, or session ids: those are
//! sealed in command.rs / stream_json.rs / session.rs.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_usage_default_is_zeroed() {
        let u = ChatUsage::default();
        assert_eq!(u.input_tokens, 0);
        assert_eq!(u.output_tokens, 0);
        assert_eq!(u.model, "");
    }

    #[test]
    fn rate_limited_is_distinguishable() {
        assert!(ChatError::RateLimited("429".into()).is_rate_limited());
        assert!(!ChatError::Spawn("no binary".into()).is_rate_limited());
        assert!(!ChatError::NoResult.is_rate_limited());
    }

    #[test]
    fn chat_reply_carries_text_and_usage_only() {
        // Compile-time proof there is no session_id field (F3): constructing a
        // ChatReply requires exactly text + usage.
        let r = ChatReply { text: "hi".into(), usage: ChatUsage::default() };
        assert_eq!(r.text, "hi");
        assert_eq!(r.usage.output_tokens, 0);
    }
}
```

- [ ] **Step 5: Run the test to verify it fails**

Run: `cd src-tauri && cargo test -p llm_chat --lib chat::`
Expected: FAIL — compile errors `cannot find type ChatUsage`, `cannot find type ChatError`, `cannot find type ChatReply` in this scope.

- [ ] **Step 6: Implement the boundary types**

Prepend the implementation above the test module in `src-tauri/llm_chat/src/chat.rs`:

```rust
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Token usage for one chat turn. Structurally identical to `runners::RunnerUsage`
/// (D2) but defined here so the ACL stays kernel-only — the composition root maps
/// it onto whatever Telemetry shape it needs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatUsage {
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation: u64,
    pub cache_read: u64,
}

/// One chat turn request. `dialogue_id` is the caller's own stable id; the ACL
/// maps it to a claude session internally (F3 — no session_id crosses out).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatRequest {
    pub dialogue_id: String,
    pub system_prompt: String,
    pub user_message: String,
    pub model: String,
    pub thinking_budget: u32,
}

/// The assistant's reply to one turn. Domain-shaped: text + usage only. There is
/// deliberately NO session_id here (F3 — session continuity is internal).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatReply {
    pub text: String,
    pub usage: ChatUsage,
}

/// Mirrors `runners::RunnerError`'s classes so chat consumers handle failure the
/// same way Runtime handles runner failure.
#[derive(Debug)]
pub enum ChatError {
    RateLimited(String),
    Spawn(String),
    NoResult,
    Other(String),
}

impl ChatError {
    /// True when the failure is a rate-limit — the one case the root treats
    /// specially (surface as an error turn; the root may set the brake).
    pub fn is_rate_limited(&self) -> bool {
        matches!(self, ChatError::RateLimited(_))
    }
}

impl fmt::Display for ChatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChatError::RateLimited(m) => write!(f, "rate limited: {m}"),
            ChatError::Spawn(m) => write!(f, "spawn failed: {m}"),
            ChatError::NoResult => write!(f, "no result parsed from chat output"),
            ChatError::Other(m) => write!(f, "chat failed: {m}"),
        }
    }
}

impl std::error::Error for ChatError {}

/// The ACL seam. Chat consumers depend only on this trait; the concrete runner
/// is selected once at the composition root. Object-safe so it is held as
/// `Arc<dyn ChatRunner>`.
#[async_trait]
pub trait ChatRunner: Send + Sync {
    async fn chat(&self, req: &ChatRequest) -> Result<ChatReply, ChatError>;
}
```

- [ ] **Step 7: Run the test to verify it passes**

Run: `cd src-tauri && cargo test -p llm_chat --lib chat::`
Expected: PASS — `test result: ok. 3 passed`.

- [ ] **Step 8: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/llm_chat/Cargo.toml src-tauri/llm_chat/src/lib.rs src-tauri/llm_chat/src/chat.rs
git commit -m "feat(llm_chat): scaffold ACL crate + boundary types (ChatRunner/ChatRequest/ChatReply/ChatError)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: Pure arg construction (`command.rs`) — including `--resume`

**Files:**
- Create: `src-tauri/llm_chat/src/command.rs`

- [ ] **Step 1: Write the failing test**

Create `src-tauri/llm_chat/src/command.rs` with the test module first:

```rust
//! claude CLI argument construction for a chat turn. Pure: takes a ChatRequest
//! (+ optional resume session id) and returns the argv vector. Simpler than the
//! worker command line (no settings / add-dir / permission-mode): chat is a
//! plain multi-turn --print invocation. The --resume flag is the ONLY place a
//! claude session id is named, and it never leaves this crate (F3).

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::ChatRequest;

    fn req() -> ChatRequest {
        ChatRequest {
            dialogue_id: "proj-1".into(),
            system_prompt: "You are the god terminal.".into(),
            user_message: "how is T-042 going?".into(),
            model: "claude-opus-4-8".into(),
            thinking_budget: 8192,
        }
    }

    #[test]
    fn first_turn_builds_the_chat_command_line_without_resume() {
        let args = build_chat_args(&req(), None);
        assert_eq!(args[0], "--print");
        assert_eq!(args[1], "--output-format");
        assert_eq!(args[2], "stream-json");
        // model + budget present
        let m = args.iter().position(|a| a == "--model").unwrap();
        assert_eq!(args[m + 1], "claude-opus-4-8");
        let b = args.iter().position(|a| a == "--max-thinking-tokens").unwrap();
        assert_eq!(args[b + 1], "8192");
        // system prompt carried
        let s = args.iter().position(|a| a == "--append-system-prompt").unwrap();
        assert_eq!(args[s + 1], "You are the god terminal.");
        // no --resume on the first turn
        assert!(!args.iter().any(|a| a == "--resume"));
        // user message is the final positional argument
        assert_eq!(args.last().unwrap(), "how is T-042 going?");
    }

    #[test]
    fn continuation_turn_includes_resume_with_the_session_id() {
        let args = build_chat_args(&req(), Some("sess-abc"));
        let r = args.iter().position(|a| a == "--resume").unwrap();
        assert_eq!(args[r + 1], "sess-abc");
        // user message still last
        assert_eq!(args.last().unwrap(), "how is T-042 going?");
    }

    #[test]
    fn no_settings_or_permission_flags_in_a_chat() {
        let args = build_chat_args(&req(), None);
        assert!(!args.iter().any(|a| a == "--settings"));
        assert!(!args.iter().any(|a| a == "--permission-mode"));
        assert!(!args.iter().any(|a| a == "--add-dir"));
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd src-tauri && cargo test -p llm_chat --lib command::`
Expected: FAIL — compile error `cannot find function build_chat_args` / `cannot find value CLAUDE_BIN`.

- [ ] **Step 3: Write the implementation**

Prepend to `src-tauri/llm_chat/src/command.rs`:

```rust
use crate::chat::ChatRequest;

/// The binary name. The composition root may override via PATH; v1 assumes
/// `claude` is resolvable.
pub const CLAUDE_BIN: &str = "claude";

/// Build the argv (excluding the program name) for one chat turn. When
/// `resume` is `Some(session)`, the turn continues that claude session
/// (`--resume <session>`); on the first turn it is `None` and a fresh session is
/// started. The session id is supplied/consumed only inside this crate (F3).
pub fn build_chat_args(req: &ChatRequest, resume: Option<&str>) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "--print".into(),
        "--output-format".into(),
        "stream-json".into(),
        "--append-system-prompt".into(),
        req.system_prompt.clone(),
        "--model".into(),
        req.model.clone(),
        "--max-thinking-tokens".into(),
        req.thinking_budget.to_string(),
    ];
    if let Some(session) = resume {
        args.push("--resume".into());
        args.push(session.to_string());
    }
    // The user message is the final positional argument.
    args.push(req.user_message.clone());
    args
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd src-tauri && cargo test -p llm_chat --lib command::`
Expected: PASS — `test result: ok. 3 passed`.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/llm_chat/src/command.rs
git commit -m "feat(llm_chat): pure chat arg construction with optional --resume

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: Stream-json fixtures (assistant text + usage + session id)

**Files:**
- Create: `src-tauri/llm_chat/src/fixtures/chat-first-turn.txt`
- Create: `src-tauri/llm_chat/src/fixtures/chat-follow-up.txt`

These are committed fixtures so the parser test (Task 4) runs with no live binary. They use the exact stream-json shape from `runners/src/fixtures/stream-json-sample.txt`, but the assistant text is plain prose (no `VERDICT:` / `ARTIFACT:` lines — chat has no verdict semantics).

- [ ] **Step 1: Create the first-turn fixture**

Create `src-tauri/llm_chat/src/fixtures/chat-first-turn.txt` with **exactly** these three lines (no trailing blank line matters; the parser skips blanks):

```
{"type":"system","subtype":"init","session_id":"sess-first","model":"claude-opus-4-8"}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"T-042 is in the design stage; "}],"usage":{"input_tokens":900,"output_tokens":5,"cache_creation_input_tokens":120,"cache_read_input_tokens":40}}}
{"type":"result","subtype":"success","is_error":false,"result":"T-042 is in the design stage; the spec is awaiting review.","usage":{"input_tokens":900,"output_tokens":18,"cache_creation_input_tokens":120,"cache_read_input_tokens":40},"session_id":"sess-first"}
```

- [ ] **Step 2: Create the follow-up fixture**

Create `src-tauri/llm_chat/src/fixtures/chat-follow-up.txt` with **exactly** these three lines (note the `session_id` is the *same* `sess-first` — a resumed turn keeps the session):

```
{"type":"system","subtype":"init","session_id":"sess-first","model":"claude-opus-4-8"}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Yes — "}],"usage":{"input_tokens":1500,"output_tokens":3,"cache_creation_input_tokens":0,"cache_read_input_tokens":900}}}
{"type":"result","subtype":"success","is_error":false,"result":"Yes — I injected the topic; it is now task T-043.","usage":{"input_tokens":1500,"output_tokens":12,"cache_creation_input_tokens":0,"cache_read_input_tokens":900},"session_id":"sess-first"}
```

- [ ] **Step 3: Commit**

```bash
git add src-tauri/llm_chat/src/fixtures/chat-first-turn.txt src-tauri/llm_chat/src/fixtures/chat-follow-up.txt
git commit -m "test(llm_chat): commit stream-json chat fixtures (first turn + follow-up)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: Stream-json parsing (`stream_json.rs`) — text + usage + captured session id

**Files:**
- Create: `src-tauri/llm_chat/src/stream_json.rs`

The parser mirrors `runners/src/stream_json.rs` but (a) drops verdict/artifact parsing and (b) returns the captured claude `session_id` alongside the `ChatReply` so `ClaudeChatRunner` can store it internally (D5). It still detects rate-limit the same way.

- [ ] **Step 1: Write the failing test**

Create `src-tauri/llm_chat/src/stream_json.rs` with the test module first:

```rust
//! stream-json parsing — the inbound half of the chat ACL. Turns Claude's
//! line-delimited JSON into a ChatReply (assistant text + usage) plus the
//! captured claude session_id (kept INSIDE the crate, F3). Reuses the exact
//! event shapes from `runners` but with no verdict/artifact convention — a chat
//! reply is plain prose.

#[cfg(test)]
mod tests {
    use super::*;

    const FIRST: &str = include_str!("fixtures/chat-first-turn.txt");
    const FOLLOW: &str = include_str!("fixtures/chat-follow-up.txt");

    #[test]
    fn parses_first_turn_text_usage_and_session_id() {
        let (reply, session) = parse_chat_stream(FIRST, "fallback-model").unwrap();
        // text is the result line's prose (authoritative when present)
        assert_eq!(reply.text, "T-042 is in the design stage; the spec is awaiting review.");
        // result usage is authoritative (900/18/120/40), not the assistant delta
        assert_eq!(reply.usage.input_tokens, 900);
        assert_eq!(reply.usage.output_tokens, 18);
        assert_eq!(reply.usage.cache_creation, 120);
        assert_eq!(reply.usage.cache_read, 40);
        assert_eq!(reply.usage.model, "claude-opus-4-8");
        // the session id was captured from the init line (stays inside the crate)
        assert_eq!(session.as_deref(), Some("sess-first"));
    }

    #[test]
    fn parses_follow_up_turn_with_same_session() {
        let (reply, session) = parse_chat_stream(FOLLOW, "m").unwrap();
        assert_eq!(reply.text, "Yes — I injected the topic; it is now task T-043.");
        assert_eq!(reply.usage.output_tokens, 12);
        assert_eq!(session.as_deref(), Some("sess-first"));
    }

    #[test]
    fn rate_limit_event_is_an_error() {
        let raw = r#"{"type":"error","error":{"message":"Rate limit exceeded (429)"}}"#;
        let err = parse_chat_stream(raw, "m").unwrap_err();
        assert!(err.is_rate_limited());
    }

    #[test]
    fn empty_stream_is_no_result() {
        let err = parse_chat_stream("\n  \n", "m").unwrap_err();
        assert!(matches!(err, crate::chat::ChatError::NoResult));
    }

    #[test]
    fn malformed_line_is_other_error() {
        let err = parse_chat_stream("not json", "m").unwrap_err();
        assert!(matches!(err, crate::chat::ChatError::Other(_)));
    }

    #[test]
    fn text_falls_back_to_assistant_blocks_when_result_text_absent() {
        let raw = concat!(
            r#"{"type":"system","subtype":"init","session_id":"s","model":"m"}"#, "\n",
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"partial reply"}],"usage":{"output_tokens":4}}}"#, "\n",
            r#"{"type":"result","subtype":"success","is_error":false,"usage":{"output_tokens":4},"session_id":"s"}"#
        );
        let (reply, session) = parse_chat_stream(raw, "m").unwrap();
        assert_eq!(reply.text, "partial reply");
        assert_eq!(session.as_deref(), Some("s"));
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd src-tauri && cargo test -p llm_chat --lib stream_json::`
Expected: FAIL — compile error `cannot find function parse_chat_stream`.

- [ ] **Step 3: Write the implementation**

Prepend to `src-tauri/llm_chat/src/stream_json.rs`:

```rust
use crate::chat::{ChatError, ChatReply, ChatUsage};
use serde_json::Value;

/// Accumulator fed one parsed JSON line at a time. Collects assistant text,
/// usage totals, and the claude session id (the F3-sealed value).
#[derive(Debug, Default)]
struct ChatAccumulator {
    text: String,
    usage: ChatUsage,
    session_id: Option<String>,
    saw_result: bool,
}

impl ChatAccumulator {
    fn feed(&mut self, v: &Value) -> Result<(), ChatError> {
        let ty = v.get("type").and_then(|t| t.as_str()).unwrap_or("");

        // Rate-limit detection: an error event whose message mentions rate/429/quota.
        if ty == "error" || v.get("is_error").and_then(|b| b.as_bool()) == Some(true) {
            let msg = v
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .or_else(|| v.get("result").and_then(|r| r.as_str()))
                .unwrap_or("error")
                .to_string();
            let lower = msg.to_lowercase();
            if lower.contains("rate") || lower.contains("429") || lower.contains("quota") {
                return Err(ChatError::RateLimited(msg));
            }
        }

        // Capture the session id wherever it appears (init line or result line).
        if let Some(sid) = v.get("session_id").and_then(|s| s.as_str()) {
            self.session_id = Some(sid.to_string());
        }

        match ty {
            "system" => {
                if let Some(model) = v.get("model").and_then(|m| m.as_str()) {
                    self.usage.model = model.to_string();
                }
            }
            "assistant" => {
                if let Some(content) = v
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_array())
                {
                    for block in content {
                        if block.get("type").and_then(|t| t.as_str()) == Some("text") {
                            if let Some(t) = block.get("text").and_then(|t| t.as_str()) {
                                self.text.push_str(t);
                            }
                        }
                    }
                }
                if let Some(u) = v.get("message").and_then(|m| m.get("usage")) {
                    self.add_usage(u);
                }
            }
            "result" => {
                self.saw_result = true;
                if let Some(u) = v.get("usage") {
                    // The result usage is authoritative for totals — replace,
                    // keeping the model captured from the system line.
                    let model = std::mem::take(&mut self.usage.model);
                    self.usage = ChatUsage { model, ..Default::default() };
                    self.add_usage(u);
                }
                // Prefer the result's prose; fall back to accumulated assistant
                // text only when the result line carries no `result` string.
                if let Some(r) = v.get("result").and_then(|r| r.as_str()) {
                    self.text = r.to_string();
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn add_usage(&mut self, u: &Value) {
        let g = |k: &str| u.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
        self.usage.input_tokens += g("input_tokens");
        self.usage.output_tokens += g("output_tokens");
        self.usage.cache_creation += g("cache_creation_input_tokens");
        self.usage.cache_read += g("cache_read_input_tokens");
    }

    fn finish(self, model: &str) -> Result<(ChatReply, Option<String>), ChatError> {
        if !self.saw_result && self.text.is_empty() {
            return Err(ChatError::NoResult);
        }
        let mut usage = self.usage;
        if usage.model.is_empty() {
            usage.model = model.to_string();
        }
        Ok((ChatReply { text: self.text, usage }, self.session_id))
    }
}

/// Parse a full chat stream (newline-delimited JSON). Returns the ChatReply plus
/// the captured claude session id (consumed internally by ClaudeChatRunner, then
/// dropped — it never reaches the caller, F3).
pub fn parse_chat_stream(raw: &str, model: &str) -> Result<(ChatReply, Option<String>), ChatError> {
    let mut acc = ChatAccumulator::default();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: Value = serde_json::from_str(line)
            .map_err(|e| ChatError::Other(format!("bad stream-json line: {e}")))?;
        acc.feed(&v)?;
    }
    acc.finish(model)
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd src-tauri && cargo test -p llm_chat --lib stream_json::`
Expected: PASS — `test result: ok. 6 passed`.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/llm_chat/src/stream_json.rs
git commit -m "feat(llm_chat): parse chat stream-json (text + usage + internal session capture)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 5: The session map (`session.rs`) — the F3 seal

**Files:**
- Create: `src-tauri/llm_chat/src/session.rs`

An in-process `dialogue_id → claude_session_id` map behind a mutex, so `ClaudeChatRunner` (held as `&self` behind `Arc`) can mutate it. This is the only place a claude session id is *stored*; it never leaves the crate.

- [ ] **Step 1: Write the failing test**

Create `src-tauri/llm_chat/src/session.rs` with the test module first:

```rust
//! The session map — the F3 seal. Maps the caller's stable `dialogue_id` to the
//! claude session id captured from stream-json. First turn for a dialogue has no
//! mapping (so no --resume); after a successful turn the session is recorded, so
//! the next turn resumes it. A lost session is cleared so the next turn starts
//! fresh. None of this is observable outside `llm_chat`.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_lookup_for_a_dialogue_is_none() {
        let map = SessionMap::new();
        assert_eq!(map.get("proj-1"), None);
    }

    #[test]
    fn recorded_session_is_returned_on_the_next_turn() {
        let map = SessionMap::new();
        map.record("proj-1", "sess-abc");
        assert_eq!(map.get("proj-1").as_deref(), Some("sess-abc"));
        // a different dialogue is independent
        assert_eq!(map.get("proj-2"), None);
    }

    #[test]
    fn record_overwrites_a_rotated_session_for_the_same_dialogue() {
        let map = SessionMap::new();
        map.record("proj-1", "sess-old");
        map.record("proj-1", "sess-new");
        assert_eq!(map.get("proj-1").as_deref(), Some("sess-new"));
    }

    #[test]
    fn clear_forces_a_fresh_session_next_turn() {
        let map = SessionMap::new();
        map.record("proj-1", "sess-dead");
        map.clear("proj-1");
        assert_eq!(map.get("proj-1"), None);
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd src-tauri && cargo test -p llm_chat --lib session::`
Expected: FAIL — compile error `cannot find type SessionMap`.

- [ ] **Step 3: Write the implementation**

Prepend to `src-tauri/llm_chat/src/session.rs`:

```rust
use std::collections::HashMap;
use std::sync::Mutex;

/// In-process map from a caller's `dialogue_id` to the claude session id. Behind
/// a Mutex so a `&self` ChatRunner (held as Arc) can mutate it across turns.
#[derive(Debug, Default)]
pub struct SessionMap {
    inner: Mutex<HashMap<String, String>>,
}

impl SessionMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// The claude session for this dialogue, if one is known (=> pass --resume).
    pub fn get(&self, dialogue_id: &str) -> Option<String> {
        self.inner.lock().unwrap().get(dialogue_id).cloned()
    }

    /// Record (or overwrite) the claude session for this dialogue after a
    /// successful turn.
    pub fn record(&self, dialogue_id: &str, session_id: &str) {
        self.inner
            .lock()
            .unwrap()
            .insert(dialogue_id.to_string(), session_id.to_string());
    }

    /// Forget the session for this dialogue (a lost/dead session) so the next
    /// turn starts fresh. The caller's dialogue_id is untouched.
    pub fn clear(&self, dialogue_id: &str) {
        self.inner.lock().unwrap().remove(dialogue_id);
    }
}
```

- [ ] **Step 4: Add the module to `lib.rs`**

Confirm `pub mod session;` is present in `src-tauri/llm_chat/src/lib.rs` (it was added in Task 1 Step 3). No change needed if already there.

- [ ] **Step 5: Run the test to verify it passes**

Run: `cd src-tauri && cargo test -p llm_chat --lib session::`
Expected: PASS — `test result: ok. 4 passed`.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/llm_chat/src/session.rs
git commit -m "feat(llm_chat): internal dialogue_id->session_id map (the F3 seal)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 6: `FakeChatRunner` test double (`fake.rs`)

**Files:**
- Create: `src-tauri/llm_chat/src/fake.rs`

Mirrors `runners`' `FakeRunner` / `conversational_control`'s `FakeEngine`: seeded replies returned in order, recording the requests it received. It drives `LlmEngine` end-to-end in Task 8 with no subprocess.

- [ ] **Step 1: Write the failing test**

Create `src-tauri/llm_chat/src/fake.rs` with the test module first:

```rust
//! FakeChatRunner — a ChatRunner test double. Returns seeded ChatReplies in
//! order (clamping to the last when exhausted) and records every ChatRequest it
//! received, so tests can assert what was asked without a live `claude`.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::{ChatRunner, ChatRequest, ChatReply, ChatUsage};

    fn req(msg: &str) -> ChatRequest {
        ChatRequest {
            dialogue_id: "proj-1".into(),
            system_prompt: "sys".into(),
            user_message: msg.into(),
            model: "m".into(),
            thinking_budget: 0,
        }
    }

    #[tokio::test]
    async fn returns_seeded_replies_in_order_and_records_requests() {
        let fake = FakeChatRunner::new(vec![
            ChatReply { text: "first".into(), usage: ChatUsage::default() },
            ChatReply { text: "second".into(), usage: ChatUsage::default() },
        ]);
        let a = fake.chat(&req("one")).await.unwrap();
        let b = fake.chat(&req("two")).await.unwrap();
        assert_eq!(a.text, "first");
        assert_eq!(b.text, "second");
        let received = fake.received.lock().unwrap();
        assert_eq!(received.len(), 2);
        assert_eq!(received[0].user_message, "one");
        assert_eq!(received[1].user_message, "two");
    }

    #[tokio::test]
    async fn clamps_to_the_last_reply_when_exhausted() {
        let fake = FakeChatRunner::new(vec![ChatReply { text: "only".into(), usage: ChatUsage::default() }]);
        let _ = fake.chat(&req("a")).await.unwrap();
        let again = fake.chat(&req("b")).await.unwrap();
        assert_eq!(again.text, "only");
    }

    #[tokio::test]
    async fn can_be_seeded_to_error() {
        let fake = FakeChatRunner::failing(crate::chat::ChatError::RateLimited("429".into()));
        let err = fake.chat(&req("x")).await.unwrap_err();
        assert!(err.is_rate_limited());
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd src-tauri && cargo test -p llm_chat --lib fake::`
Expected: FAIL — compile error `cannot find type FakeChatRunner`.

- [ ] **Step 3: Write the implementation**

Prepend to `src-tauri/llm_chat/src/fake.rs`:

```rust
use crate::chat::{ChatError, ChatReply, ChatRequest, ChatRunner};
use async_trait::async_trait;
use std::sync::Mutex;

/// A ChatRunner test double. Either returns seeded replies in order (clamping to
/// the last when exhausted) or always errors with a seeded ChatError. Records
/// every request for assertions.
pub struct FakeChatRunner {
    replies: Vec<ChatReply>,
    error: Option<ChatError>,
    cursor: Mutex<usize>,
    pub received: Mutex<Vec<ChatRequest>>,
}

impl FakeChatRunner {
    /// Seed with replies returned in order.
    pub fn new(replies: Vec<ChatReply>) -> Self {
        Self { replies, error: None, cursor: Mutex::new(0), received: Mutex::new(vec![]) }
    }

    /// Seed with an error that every `chat` call returns.
    pub fn failing(error: ChatError) -> Self {
        Self { replies: vec![], error: Some(error), cursor: Mutex::new(0), received: Mutex::new(vec![]) }
    }
}

#[async_trait]
impl ChatRunner for FakeChatRunner {
    async fn chat(&self, req: &ChatRequest) -> Result<ChatReply, ChatError> {
        self.received.lock().unwrap().push(req.clone());
        if let Some(err) = &self.error {
            // Re-create the same class (ChatError is not Clone).
            return Err(match err {
                ChatError::RateLimited(m) => ChatError::RateLimited(m.clone()),
                ChatError::Spawn(m) => ChatError::Spawn(m.clone()),
                ChatError::NoResult => ChatError::NoResult,
                ChatError::Other(m) => ChatError::Other(m.clone()),
            });
        }
        let mut c = self.cursor.lock().unwrap();
        let idx = (*c).min(self.replies.len().saturating_sub(1));
        *c += 1;
        Ok(self
            .replies
            .get(idx)
            .cloned()
            .unwrap_or(ChatReply { text: String::new(), usage: Default::default() }))
    }
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd src-tauri && cargo test -p llm_chat --lib fake::`
Expected: PASS — `test result: ok. 3 passed`.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/llm_chat/src/fake.rs
git commit -m "test(llm_chat): FakeChatRunner test double (seeded replies + recorded requests)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 7: `ClaudeChatRunner` (`claude_cli.rs`) — wire command + session + parse, with lost-session fallback

**Files:**
- Create: `src-tauri/llm_chat/src/claude_cli.rs`

Ties it together: look up the dialogue's session → build args (with/without `--resume`) → spawn → parse → on success record the captured session and return the reply (session dropped). On a lost-session symptom for a resumed turn, retry once fresh (D6). The only side effect (spawning `claude`) is behind an injectable `SpawnFn` (copying `runners::claude_cli`), so the whole runner is unit-tested with canned stdout.

- [ ] **Step 1: Write the failing test**

Create `src-tauri/llm_chat/src/claude_cli.rs` with the test module first:

```rust
//! ClaudeChatRunner — the real chat runner. Composes command::build_chat_args +
//! session::SessionMap + stream_json::parse_chat_stream; the only Claude-idiom
//! side effect (spawning the subprocess) is isolated behind a Spawner so the
//! whole runner is unit-tested without a live `claude`. The captured session id
//! is recorded internally and dropped before returning (F3).

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::{ChatError, ChatRunner, ChatRequest};
    use std::sync::{Arc, Mutex};

    const FIRST: &str = include_str!("fixtures/chat-first-turn.txt");
    const FOLLOW: &str = include_str!("fixtures/chat-follow-up.txt");

    fn req(msg: &str) -> ChatRequest {
        ChatRequest {
            dialogue_id: "proj-1".into(),
            system_prompt: "sys".into(),
            user_message: msg.into(),
            model: "claude-opus-4-8".into(),
            thinking_budget: 8192,
        }
    }

    #[tokio::test]
    async fn first_turn_omits_resume_and_returns_reply_without_session_id() {
        // Record the args each call saw so we can assert --resume presence.
        let seen: Arc<Mutex<Vec<Vec<String>>>> = Arc::new(Mutex::new(vec![]));
        let s = seen.clone();
        let runner = ClaudeChatRunner::with_spawner(Box::new(move |args| {
            s.lock().unwrap().push(args.to_vec());
            Ok(FIRST.to_string())
        }));
        let reply = runner.chat(&req("how is T-042?")).await.unwrap();
        assert_eq!(reply.text, "T-042 is in the design stage; the spec is awaiting review.");
        assert_eq!(reply.usage.input_tokens, 900);
        // ChatReply has no session_id field at all (compile-time F3 guarantee).
        // First call must NOT carry --resume.
        let calls = seen.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert!(!calls[0].iter().any(|a| a == "--resume"));
    }

    #[tokio::test]
    async fn second_turn_for_same_dialogue_resumes_the_captured_session() {
        let seen: Arc<Mutex<Vec<Vec<String>>>> = Arc::new(Mutex::new(vec![]));
        let s = seen.clone();
        // First call returns FIRST (session sess-first), second returns FOLLOW.
        let n = Arc::new(Mutex::new(0usize));
        let runner = ClaudeChatRunner::with_spawner(Box::new(move |args| {
            s.lock().unwrap().push(args.to_vec());
            let mut k = n.lock().unwrap();
            let out = if *k == 0 { FIRST } else { FOLLOW };
            *k += 1;
            Ok(out.to_string())
        }));
        let _ = runner.chat(&req("first")).await.unwrap();
        let _ = runner.chat(&req("second")).await.unwrap();
        let calls = seen.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert!(!calls[0].iter().any(|a| a == "--resume"));
        // second call resumes sess-first (captured from the first turn)
        let r = calls[1].iter().position(|a| a == "--resume").unwrap();
        assert_eq!(calls[1][r + 1], "sess-first");
    }

    #[tokio::test]
    async fn lost_session_on_a_resumed_turn_retries_fresh_once() {
        // Seed a known session, then make a --resume call fail with NoResult
        // (empty stdout) but a fresh (no --resume) call succeed.
        let n = Arc::new(Mutex::new(0usize));
        let runner = ClaudeChatRunner::with_spawner(Box::new(move |args| {
            let mut k = n.lock().unwrap();
            *k += 1;
            // call 1: first turn -> FIRST (captures sess-first)
            // call 2: resumed turn -> empty (lost session -> NoResult)
            // call 3: fresh retry -> FOLLOW
            let has_resume = args.iter().any(|a| a == "--resume");
            if *k == 1 {
                Ok(FIRST.to_string())
            } else if has_resume {
                Ok(String::new()) // empty stdout => NoResult
            } else {
                Ok(FOLLOW.to_string())
            }
        }));
        let _ = runner.chat(&req("first")).await.unwrap(); // establishes sess-first
        let reply = runner.chat(&req("second")).await.unwrap(); // resume fails, retries fresh
        assert_eq!(reply.text, "Yes — I injected the topic; it is now task T-043.");
    }

    #[tokio::test]
    async fn spawn_failure_propagates_and_is_not_retried() {
        let runner = ClaudeChatRunner::with_spawner(Box::new(|_| {
            Err(ChatError::Spawn("no binary".into()))
        }));
        let err = runner.chat(&req("x")).await.unwrap_err();
        assert!(matches!(err, ChatError::Spawn(_)));
    }

    #[tokio::test]
    async fn rate_limit_propagates_and_is_not_retried() {
        let runner = ClaudeChatRunner::with_spawner(Box::new(|_| {
            Ok(r#"{"type":"error","error":{"message":"429 rate limit"}}"#.to_string())
        }));
        let err = runner.chat(&req("x")).await.unwrap_err();
        assert!(err.is_rate_limited());
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd src-tauri && cargo test -p llm_chat --lib claude_cli::`
Expected: FAIL — compile error `cannot find type ClaudeChatRunner`.

- [ ] **Step 3: Write the implementation**

Prepend to `src-tauri/llm_chat/src/claude_cli.rs`:

```rust
use crate::chat::{ChatError, ChatReply, ChatRequest, ChatRunner};
use crate::command::{build_chat_args, CLAUDE_BIN};
use crate::session::SessionMap;
use crate::stream_json::parse_chat_stream;
use async_trait::async_trait;

/// Produces the raw stream-json stdout for a given argv. Async-free + boxed so
/// the real implementation can run the child process; tests inject canned
/// stdout. Returns Err(ChatError) on spawn failure (the one error class the
/// parser can't produce).
pub type SpawnFn = Box<dyn Fn(&[String]) -> Result<String, ChatError> + Send + Sync>;

pub struct ClaudeChatRunner {
    spawn: SpawnFn,
    sessions: SessionMap,
}

impl ClaudeChatRunner {
    /// The production runner: spawns `claude` and captures stdout.
    pub fn new() -> Self {
        Self {
            spawn: Box::new(|args: &[String]| {
                let output = std::process::Command::new(CLAUDE_BIN)
                    .args(args)
                    .output()
                    .map_err(|e| ChatError::Spawn(e.to_string()))?;
                Ok(String::from_utf8_lossy(&output.stdout).into_owned())
            }),
            sessions: SessionMap::new(),
        }
    }

    /// Test/alternate constructor: inject the stdout producer.
    pub fn with_spawner(spawn: SpawnFn) -> Self {
        Self { spawn, sessions: SessionMap::new() }
    }

    /// Spawn once with the given resume option, parse, and on success record the
    /// captured session under `dialogue_id`. The session id is dropped here — it
    /// never reaches the returned ChatReply (F3).
    fn run_once(
        &self,
        req: &ChatRequest,
        resume: Option<&str>,
    ) -> Result<ChatReply, ChatError> {
        let args = build_chat_args(req, resume);
        let stdout = (self.spawn)(&args)?;
        let (reply, session_id) = parse_chat_stream(&stdout, &req.model)?;
        if let Some(sid) = session_id {
            self.sessions.record(&req.dialogue_id, &sid);
        }
        Ok(reply)
    }
}

impl Default for ClaudeChatRunner {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ChatRunner for ClaudeChatRunner {
    async fn chat(&self, req: &ChatRequest) -> Result<ChatReply, ChatError> {
        let resume = self.sessions.get(&req.dialogue_id);
        match self.run_once(req, resume.as_deref()) {
            Ok(reply) => Ok(reply),
            // Lost-session fallback (D6): a resumed turn that yields no parseable
            // result is treated as a dead session — clear it and retry once
            // fresh. RateLimited / Spawn are NOT lost-session conditions and
            // propagate as-is.
            Err(err) => {
                let was_resumed = resume.is_some();
                let recoverable = matches!(err, ChatError::NoResult | ChatError::Other(_));
                if was_resumed && recoverable {
                    self.sessions.clear(&req.dialogue_id);
                    self.run_once(req, None)
                } else {
                    Err(err)
                }
            }
        }
    }
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd src-tauri && cargo test -p llm_chat --lib claude_cli::`
Expected: PASS — `test result: ok. 5 passed`.

- [ ] **Step 5: Run the whole crate to confirm nothing regressed**

Run: `cd src-tauri && cargo test -p llm_chat`
Expected: PASS — all tests across `chat`, `command`, `stream_json`, `session`, `fake`, `claude_cli` green.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/llm_chat/src/claude_cli.rs
git commit -m "feat(llm_chat): ClaudeChatRunner (resume-aware, lost-session fallback, injectable spawner)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 8: The `LlmEngine` adapter at the composition root + end-to-end test

**Files:**
- Modify: `src-tauri/app/Cargo.toml` (add `llm_chat` dependency)
- Modify: `src-tauri/app/src/lib.rs` (add the `LlmEngine` adapter struct, its test, build an `Arc<dyn ChatRunner>`, and switch the terminal engine line)

`LlmEngine` is a new `ConversationEngine` impl living in `app` (the only crate allowed to import both `llm_chat` and `conversational_control` — D3). It holds an `Arc<dyn ChatRunner>` + the system-prompt framing + the `project_id` used as the stable `dialogue_id` (D4).

- [ ] **Step 1: Add the dependency**

In `src-tauri/app/Cargo.toml`, under `[dependencies]`, add (next to the existing context crate paths):

```toml
llm_chat = { path = "../llm_chat" }
```

- [ ] **Step 2: Write the failing test (end-to-end through `send_message_inner` with `FakeChatRunner`)**

In `src-tauri/app/src/lib.rs`, add a new test module at the end of the file (after `migration_tests`):

```rust
#[cfg(test)]
mod llm_engine_tests {
    use super::LlmEngine;
    use conversational_control::api::send_message_inner;
    use conversational_control::catalog::ToolCatalog;
    use conversational_control::engine::ConversationEngine;
    use conversational_control::store::ConversationStore;
    use llm_chat::chat::{ChatReply, ChatRunner, ChatUsage};
    use llm_chat::fake::FakeChatRunner;
    use sqlx::sqlite::SqlitePoolOptions;
    use std::sync::Arc;

    async fn store_with_project() -> ConversationStore {
        let pool = SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        sqlx::query(include_str!("../migrations/001_initial.sql")).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO projects (id,name,root_path,created_at,updated_at) VALUES ('p','n','/p',0,0)")
            .execute(&pool).await.unwrap();
        ConversationStore::new(pool)
    }

    #[tokio::test]
    async fn llm_engine_respond_returns_the_runner_reply_text() {
        let fake = Arc::new(FakeChatRunner::new(vec![ChatReply {
            text: "T-042 is in design.".into(),
            usage: ChatUsage::default(),
        }]));
        let engine = LlmEngine::new(fake.clone() as Arc<dyn ChatRunner>, "p".into(), "framing".into(), "m".into(), 8192);
        let catalog = ToolCatalog::new(vec![]);
        let reply = engine.respond("how is T-042 going?", &catalog).await;
        assert_eq!(reply.text, "T-042 is in design.");
        assert!(reply.tool_calls.is_empty()); // free-form chat dispatches nothing in v1 (D8)
        // the dialogue_id handed to the runner is the project_id (D4)
        let received = fake.received.lock().unwrap();
        assert_eq!(received[0].dialogue_id, "p");
        assert_eq!(received[0].user_message, "how is T-042 going?");
        assert_eq!(received[0].system_prompt, "framing");
    }

    #[tokio::test]
    async fn llm_engine_drives_send_message_inner_end_to_end() {
        let store = store_with_project().await;
        let fake = Arc::new(FakeChatRunner::new(vec![ChatReply {
            text: "Hello from the model.".into(),
            usage: ChatUsage::default(),
        }]));
        let engine: Arc<dyn ConversationEngine> =
            Arc::new(LlmEngine::new(fake as Arc<dyn ChatRunner>, "p".into(), "framing".into(), "m".into(), 8192));
        let catalog = ToolCatalog::new(vec![]);

        let convo = send_message_inner("p", &catalog, engine.as_ref(), &store, "hi there", 500)
            .await
            .unwrap();
        // user turn + assistant turn appended, alternation held
        assert_eq!(convo.turns.len(), 2);
        assert_eq!(convo.turns[0].text, "hi there");
        assert_eq!(convo.turns[1].text, "Hello from the model.");
        // persisted
        let reloaded = store.load("p").await.unwrap().unwrap();
        assert_eq!(reloaded.turns.len(), 2);
        assert_eq!(reloaded.turns[1].text, "Hello from the model.");
    }

    #[tokio::test]
    async fn llm_engine_surfaces_a_chat_error_as_an_error_turn() {
        let fake = Arc::new(FakeChatRunner::failing(llm_chat::chat::ChatError::Spawn("no claude on PATH".into())));
        let engine = LlmEngine::new(fake as Arc<dyn ChatRunner>, "p".into(), "framing".into(), "m".into(), 8192);
        let catalog = ToolCatalog::new(vec![]);
        let reply = engine.respond("hi", &catalog).await;
        // a clear error turn, no panic, no tool calls
        assert!(reply.text.contains("spawn failed") || reply.text.contains("error"));
        assert!(reply.tool_calls.is_empty());
    }
}
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cd src-tauri && cargo test -p app --lib llm_engine_tests`
Expected: FAIL — compile error `cannot find type LlmEngine` (and the `llm_chat` import resolves only after Step 1's manifest edit).

- [ ] **Step 4: Write the `LlmEngine` adapter**

In `src-tauri/app/src/lib.rs`, add these imports near the top (with the other `conversational_control` / context imports):

```rust
use conversational_control::engine::{ConversationEngine, EngineReply};
use conversational_control::catalog::ToolCatalog;
use llm_chat::chat::{ChatRequest, ChatRunner};
```

(If `ConversationEngine` is already imported via another path, do not duplicate the `use` — keep one.)

Then add the adapter struct (place it just after the `RootDispatcher` struct definition, before `impl ToolDispatcher`):

```rust
/// The free-form chat engine for the god terminal (spec Consumer 1). A
/// ConversationEngine that delegates each user turn to the llm_chat ACL. Lives
/// at the composition root because it is the one place allowed to import both
/// `conversational_control` (the trait) and `llm_chat` (the runner) without
/// creating a cross-context cycle (F2/D3). Uses `project_id` as the stable
/// `dialogue_id` (one terminal conversation per project — D4). Produces prose
/// only; tool dispatch via the catalog is a v1.1 merge with CommandEngine (D8).
pub struct LlmEngine {
    runner: Arc<dyn ChatRunner>,
    dialogue_id: String,
    system_prompt: String,
    model: String,
    thinking_budget: u32,
}

impl LlmEngine {
    pub fn new(
        runner: Arc<dyn ChatRunner>,
        dialogue_id: String,
        system_prompt: String,
        model: String,
        thinking_budget: u32,
    ) -> Self {
        Self { runner, dialogue_id, system_prompt, model, thinking_budget }
    }
}

#[async_trait]
impl ConversationEngine for LlmEngine {
    async fn respond(&self, input: &str, _catalog: &ToolCatalog) -> EngineReply {
        let req = ChatRequest {
            dialogue_id: self.dialogue_id.clone(),
            system_prompt: self.system_prompt.clone(),
            user_message: input.to_string(),
            model: self.model.clone(),
            thinking_budget: self.thinking_budget,
        };
        match self.runner.chat(&req).await {
            Ok(reply) => EngineReply { text: reply.text, tool_calls: vec![] },
            // A chat failure becomes a clear, non-panicking error turn (spec
            // §Error handling). The root's existing rate-limit handling can read
            // the text; the terminal never crashes on a missing `claude`.
            Err(e) => EngineReply { text: format!("[terminal error] {e}"), tool_calls: vec![] },
        }
    }
}
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cd src-tauri && cargo test -p app --lib llm_engine_tests`
Expected: PASS — `test result: ok. 3 passed`.

- [ ] **Step 6: Switch the terminal engine line to `LlmEngine`**

In `src-tauri/app/src/lib.rs`, find the existing wiring inside `setup(...)`:

```rust
                let engine: Arc<dyn conversational_control::engine::ConversationEngine> =
                    Arc::new(CommandEngine::new(dispatcher.clone()));
```

Replace it with the chat-runner-backed engine. Insert the runner construction and swap the engine (keep `dispatcher` constructed above — other root code and the catalog still reference it):

```rust
                // The terminal's free-form chat engine (Plan llm_chat). One
                // chat runner; the conversation's project_id is the stable
                // dialogue_id (D4). The system framing is the terminal's
                // operating prompt; model + budget are v1 defaults.
                let chat_runner: Arc<dyn llm_chat::chat::ChatRunner> =
                    Arc::new(llm_chat::claude_cli::ClaudeChatRunner::new());
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

(Leave the `CommandEngine` import in place — it is still referenced by `conversational_control`'s own tests and may be re-introduced for the v1.1 slash-command merge. If the Rust compiler warns `unused import: CommandEngine`, prefix it `#[allow(unused_imports)]` on the `use conversational_control::engine::CommandEngine;` line rather than deleting it, so the v1.1 merge stays one edit away.)

- [ ] **Step 7: Build the whole workspace to confirm the root compiles and nothing regressed**

Run: `cd src-tauri && cargo build`
Expected: PASS — workspace builds; no errors. (A single `unused` warning is acceptable if the `CommandEngine` import is kept.)

- [ ] **Step 8: Run the full test suite to confirm no existing test broke**

Run: `cd src-tauri && cargo test`
Expected: PASS — every crate green, including the unchanged `conversational_control` `CommandEngine` tests (D3 — they never reference `llm_chat`) and the new `app` `llm_engine_tests`.

- [ ] **Step 9: Commit**

```bash
git add src-tauri/app/Cargo.toml src-tauri/app/src/lib.rs
git commit -m "feat(app): wire LlmEngine over llm_chat ChatRunner as the god terminal engine

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 9: Register the context (ddd-council.json + DOMAIN.md) + kernel-only verification

**Files:**
- Modify: `ddd-council.json`
- Modify: `DOMAIN.md`

- [ ] **Step 1: Verify `llm_chat` is kernel-only (the F2 acyclic proof)**

Run: `cd src-tauri && cargo tree -p llm_chat -e normal`
Expected: the dependency tree for `llm_chat` lists `agent_bus_core` and the infra crates (`async-trait`, `serde`, `serde_json`, `tokio`, `uuid`) but **NO** `runners`, `pipeline`, `runtime`, `usage_telemetry`, `workspace`, or `conversational_control` node. If any supplier/customer crate appears, the dependency rule (D1/F2) is violated — fix `llm_chat/Cargo.toml` before proceeding.

- [ ] **Step 2: Register `llm_chat` in `ddd-council.json`**

In `ddd-council.json`, add a context entry under `"contexts"` (after `"runners"`, keeping JSON valid — add a comma after the prior entry):

```json
    "llm-chat":                { "module": "llm_chat",             "paths": ["src-tauri/llm_chat/**"],           "publicModules": ["chat"] },
```

(`publicModules: ["chat"]` — the boundary types in `chat.rs` are the published surface; `command`/`stream_json`/`session`/`claude_cli` are internal mechanics, `fake` is a test double.)

- [ ] **Step 3: Verify the JSON is valid**

Run: `python3 -c "import json; json.load(open('ddd-council.json')); print('valid')"`
Expected: prints `valid`.

- [ ] **Step 4: Update DOMAIN.md — add `llm_chat` as a second ACL**

In `DOMAIN.md`, in the "## Bounded contexts" list, immediately after the `**Runners (ACL)** *(supplier)*` bullet, add:

```markdown
- **LLM Chat (ACL)** *(supplier)* — anti-corruption layer for *multi-turn* Claude dialogue. Translates a stable `dialogue_id` + system framing + user message into Claude's chat idiom (`claude --print --output-format stream-json [--resume]`) and Claude's responses back into our idiom (assistant reply text + usage). Session continuity (`session_id`/`--resume`) is sealed inside the layer and never crosses the boundary. Consumed by Conversational Control (the god terminal) and Pipeline Authoring (the wizard's Design Session). Distinct from Runners, which stays the one-shot, verdict-shaped *worker* ACL.
```

- [ ] **Step 5: Update DOMAIN.md — add the Design Session stub to Pipeline Authoring**

In `DOMAIN.md`, in the "### Pipeline Authoring" ubiquitous-language list, add a bullet (full definition lands with sub-project 3):

```markdown
- **Design Session** — an ephemeral authoring dialogue used to design a pipeline, conducted over the LLM Chat ACL; distinct from the terminal's `Conversation` aggregate (one `dialogue_id` per wizard step). *(Full definition lands with sub-project 3 — the wizard.)*
```

- [ ] **Step 6: Commit**

```bash
git add ddd-council.json DOMAIN.md
git commit -m "docs(ddd): register llm_chat as the multi-turn chat ACL (council + DOMAIN.md)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Final verification (run after Task 9)

- [ ] `cd src-tauri && cargo test` — entire workspace green, no live `claude` binary used anywhere.
- [ ] `cd src-tauri && cargo tree -p llm_chat -e normal` — no supplier/customer crate node (F2 acyclic).
- [ ] `cd src-tauri && cargo build` — release-shaped composition root compiles with the `LlmEngine` wired.
- [ ] Grep guard: `cd src-tauri && grep -rn "session_id" llm_chat/src/chat.rs` returns **nothing** (F3 — `ChatReply` carries no session id).
- [ ] Grep guard: `cd src-tauri && grep -rnE "runners|pipeline::|runtime::|conversational_control" llm_chat/src` returns **nothing** (D1 — kernel-only).

---

## Self-review (against the spec)

- **F2 (new crate, kernel-only)** → Tasks 1 + 9 (manifest + `cargo tree` proof + grep guard). ✓
- **F1 (no `Conversation` in `llm_chat`)** → `llm_chat` has no such type; it imports nothing from `conversational_control`; the Design Session stub is added to DOMAIN.md only (Task 9). ✓
- **F3 (session sealed, no `session_id` out)** → `ChatReply` has only `text` + `usage` (Task 1, compile-time); `session.rs` holds the map (Task 5); `parse_chat_stream` returns the session id as a *separate* tuple element consumed only inside `ClaudeChatRunner` (Tasks 4, 7); grep guard (final). ✓
- **Boundary types `ChatRequest`/`ChatReply`/`ChatError`/`ChatRunner`** → Task 1. ✓
- **`ClaudeChatRunner` real `--print --output-format stream-json [--resume]` behind a spawner** → Tasks 2 (args), 7 (runner). ✓
- **Pure arg test incl. `--resume`** → Task 2. ✓
- **stream-json text+usage parsing from committed fixtures (first turn w/ init session id; follow-up)** → Tasks 3 + 4. ✓
- **Session-map unit test (first omits resume; second includes; lost session falls back)** → Tasks 5 (map) + 7 (fallback in `ClaudeChatRunner`). ✓
- **`FakeChatRunner` drives `LlmEngine` end-to-end (turn appended, alternation held, persisted)** → Tasks 6 + 8. ✓
- **god terminal rewired via `LlmEngine` at the root with `Arc<dyn ChatRunner>`, conversation id as `dialogue_id`** → Task 8 (D4 resolves "conversation id" = `project_id`, the singleton key). ✓
- **`conversational_control` stays kernel-only; `CommandEngine` tests unbroken** → D3; Task 8 changes only one root line + adds the adapter in `app`; final `cargo test`. ✓
- **Non-streaming v1** → D7; `ClaudeChatRunner` awaits whole stdout. ✓
- **Error handling (rate-limit/spawn/no-result → error turn, no crash)** → Tasks 7 (errors) + 8 (error turn). ✓
- **Cargo + ddd-council registration** → Tasks 1 + 9. ✓
- **DOMAIN.md updates (2nd ACL, Design Session stub)** → Task 9. ✓

No placeholders; every code step shows full code; type/method names (`ChatUsage`, `ChatRequest`, `ChatReply`, `ChatError`, `ChatRunner`, `build_chat_args`, `parse_chat_stream`, `SessionMap::{new,get,record,clear}`, `ClaudeChatRunner::{new,with_spawner,run_once}`, `FakeChatRunner::{new,failing}`, `LlmEngine::new`) are consistent across tasks.

---

## Roadmap — what this unblocks and what is deliberately deferred

- **Sub-project 2 — parallel flow** (schema + Runtime fan-out/fan-in). Independent of this crate.
- **Sub-project 3 — the wizard.** Pipeline Authoring gains a **Design Session** (one `dialogue_id` per wizard step) constructing `ChatRequest`s over the same `Arc<dyn ChatRunner>` — built on exactly the surface this plan ships. The full Design Session definition + structured pipeline emit land there.
- **Deferred within this capability (explicitly out of scope here):**
  - **Token streaming** — incremental stream-json + throttled `chat.delta` events. v1 is non-streaming (D7); the parser is already line-oriented so a streaming variant slots behind the same `ChatRunner` trait.
  - **The v1.1 slash-command + free-form merge.** Today the terminal runs *either* `CommandEngine` (slash) *or* `LlmEngine` (chat). v1.1 unifies them — e.g. a `CompositeEngine` that parses a leading `/` as a command (dispatching via `RootDispatcher`) and otherwise delegates to the chat runner, optionally letting the model emit tool-calls. The `ToolCatalog` is already threaded through `respond`, so `LlmEngine` can begin emitting `tool_calls` without a trait change (D8).
  - **Usage attribution from chat turns.** `ChatReply.usage` is produced but not yet recorded to Telemetry; the composition root will map `ChatUsage` → `agent_bus_core::UsageEvent` and feed the existing `UsageSink` (the same seam the worker loops use).
