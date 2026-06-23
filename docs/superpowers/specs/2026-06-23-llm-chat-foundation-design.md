# Spec — `llm_chat` foundation (sub-project 1 of the brainstorming new-project wizard)

*Design doc. Brainstormed + DDD-vetted 2026-06-23. The first of three sub-projects; sees its own plan → implementation cycle.*

## Why this exists

The new-project **brainstorming wizard** (the headline feature) needs an interactive, multi-turn chat with Claude in each step. The app has no LLM chat loop today: `conversational_control` ships only a command-driven `CommandEngine`, and `runners` does one-shot, **verdict-shaped** invocations (`claude --print … <one message>` → parse a `Verdict`). A chat needs the opposite — multi-turn, returning the assistant's *reply text*.

This sub-project builds that capability once, as a shared anti-corruption layer, so two consumers can use it without duplicating Claude mechanics or coupling to each other: the **god terminal** (Conversational Control) and the **wizard** (Pipeline Authoring, sub-project 3).

## Scope

In scope: a new `llm_chat` ACL crate; rewiring the god terminal's engine to use it. Out of scope (later sub-projects): parallel fan-out/fan-in (sub-project 2); the wizard UI, the Design Session concept, and structured pipeline emit (sub-project 3).

## DDD decisions (from the council vet, 2026-06-23)

The design was vetted before speccing. Three findings, all resolved here:

- **F2 [high] circular-dependency by design.** The shared chat capability cannot live in `conversational_control` (Pipeline Authoring would depend on the customer → cycle) nor in the existing `runners` crate (`runners → pipeline` already, so `pipeline → runners → pipeline`). **Resolution:** a dedicated `llm_chat` crate depending only on `agent_bus_core` (kernel). Both consumers depend on it; it depends on nothing project-internal → acyclic.
- **F1 [medium] one-name-two-concepts.** The wizard's per-step dialogue is **not** the god terminal's `Conversation` aggregate (DOMAIN.md: Conversation = the terminal's persistent tool-calling session, per-project singleton, 24h-summarised). **Resolution:** the wizard gets its own ephemeral **Design Session** concept in Pipeline Authoring (sub-project 3). `llm_chat` shares only the chat *capability*, never the `Conversation` aggregate.
- **F3 [medium] leaky ACL.** `session_id` / `--resume` are Claude-CLI idioms and must not cross the ACL boundary (same discipline the Plan 3 vet enforced for `stream-json` / `--max-thinking-tokens`). **Resolution:** session continuity is managed **inside** `llm_chat`; callers pass a stable *dialogue id*; `ChatReply` carries only domain-shaped data.

These also update the domain model (see "DOMAIN.md updates" below).

## Architecture

### The `llm_chat` crate (new Cargo workspace member)

Depends only on `agent_bus_core` + infra (`async-trait`, `serde`, `tokio`). No `pipeline`, `runtime`, `runners`, or `conversational_control` edge.

```
src-tauri/llm_chat/
├── Cargo.toml
└── src/
    ├── lib.rs
    ├── chat.rs        # ChatRunner trait + ChatRequest / ChatReply / ChatError
    ├── claude_cli.rs  # ClaudeChatRunner (real claude --print --resume), behind an injectable spawner
    ├── session.rs     # internal dialogue-id → claude session-id map (the F3 seal)
    └── fake.rs        # FakeChatRunner test double
```

### The boundary types (Runtime/domain vocabulary, no Claude idioms)

```rust
/// One chat turn request. `dialogue_id` is the caller's own stable id; the ACL
/// maps it to a claude session internally (F3 — no session_id crosses out).
pub struct ChatRequest {
    pub dialogue_id: String,
    pub system_prompt: String,   // the per-context framing (terminal vs a wizard step)
    pub user_message: String,
    pub model: String,
    pub thinking_budget: u32,
}

pub struct ChatReply {
    pub text: String,            // assistant reply
    pub usage: RunnerUsage,      // reuse agent_bus_core usage shape
}

#[async_trait]
pub trait ChatRunner: Send + Sync {
    async fn chat(&self, req: &ChatRequest) -> Result<ChatReply, ChatError>;
}
```

- `ChatError` mirrors the existing `RunnerError` classes (RateLimited / Spawn / NoResult / Other) so callers handle failure consistently.
- **Distinct trait, not a second method on `Runner`** (per the vet): Runtime keeps depending only on `runners::Runner`; chat consumers depend only on `llm_chat::ChatRunner`.
- Session continuity: `session.rs` keeps an in-process `dialogue_id → claude_session_id` map. First turn omits `--resume` and captures the session id from the stream-json init event; later turns for the same `dialogue_id` pass `--resume <session>`. If a session is lost, it falls back to a fresh session (the caller's `dialogue_id` is unchanged). None of this is visible outside the crate.

### Consumer 1 — god terminal (Conversational Control)

`conversational_control` stays a **pure customer** (kernel-only deps). Its `ConversationEngine` trait gains a real adapter, `LlmEngine`, wired at the **composition root** (same pattern as `RootDispatcher`), constructed with an `Arc<dyn ChatRunner>`. The `Conversation` aggregate, `ConversationStore`, turn/alternation/24h-summarise invariants are unchanged; `LlmEngine` just produces the assistant turn for a user message using the conversation's id as the `dialogue_id`.

### Consumer 2 — wizard (deferred to sub-project 3)

Noted for coherence only: Pipeline Authoring's Design Session will construct `ChatRequest`s (one `dialogue_id` per wizard step) over the same `Arc<dyn ChatRunner>`. Not built here.

## Data flow (god terminal, non-streaming v1)

1. Frontend `send_message(text)` → existing terminal command.
2. Append user turn to `Conversation` (alternation invariant).
3. `LlmEngine` builds a `ChatRequest { dialogue_id = conversation.id, system_prompt = terminal framing, user_message = text, model, thinking_budget }`.
4. `ChatRunner.chat` → `claude --print --output-format stream-json [--resume]` → parse assistant text + usage.
5. Append assistant turn; persist via `ConversationStore`; emit `conversation.changed`.
6. Usage flows to Telemetry through the existing sink at the root.

**Non-streaming v1:** each turn awaits the full `--print` response. Token streaming is a later enhancement (incremental stream-json + throttled events), explicitly out of scope here.

## Error handling

- **Rate-limit** → `ChatError::RateLimited`; surfaced as an error turn in the conversation; the root may set the brake (reusing existing behaviour). 
- **Spawn failure** (no `claude` on PATH) → `ChatError::Spawn`; clear error turn, no crash.
- **Parse / no result** → `ChatError::NoResult`; error turn.
- No verdict semantics anywhere in this path.

## Testing (all green with no live `claude`)

- **Pure arg construction** test for `claude --print … [--resume]` (mirrors the existing `build_args` test discipline).
- **Assistant-text + usage parsing** from committed stream-json fixtures (first-turn init event with a session id; a follow-up turn).
- **Session map** unit test: first turn omits `--resume`, second turn for the same `dialogue_id` includes it; a lost session falls back cleanly.
- **`FakeChatRunner`** drives `LlmEngine` end-to-end (turn appended, alternation held, persisted) with canned replies — no subprocess.
- **`llm_chat` depends on no supplier crate** — a `cargo tree -p llm_chat` assertion in the plan's verification (kernel-only).

## DOMAIN.md updates (apply when this lands)

- Add `llm_chat` as a **second ACL** alongside Runners in the bounded-contexts list: "anti-corruption layer for *multi-turn* Claude dialogue; consumed by Conversational Control and Pipeline Authoring." (Runners stays the one-shot, verdict-shaped worker ACL.)
- Add **Design Session** to Pipeline Authoring's ubiquitous language (full definition lands with sub-project 3): "an ephemeral authoring dialogue used to design a pipeline; distinct from the terminal's `Conversation`."
- Update `ddd-council.json` to register `llm_chat` (kernel-adjacent ACL; `module`/`paths` for the new crate).

## Roadmap — what this unblocks

- **Sub-project 2 — parallel flow** (schema + Runtime fan-out/fan-in). Independent of this.
- **Sub-project 3 — the wizard** (5-step UI, one-shot kickoff, per-step Design Session chat over `llm_chat`, structured pipeline emit + two-way draft, write YAML + prompts, drop the fixed-template path). Depends on this **and** sub-project 2.
