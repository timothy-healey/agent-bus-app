# T2 — N-NativeToolUse: native tool-use / structured output via the anthropic-api chat runner

> **REQUIRED SUB-SKILL:** superpowers:subagent-driven-development. Backlog item **N-NativeToolUse**. The API-path forced-correctness upgrade of DS-Schema (slice emit) + T1 (terminal tool args). Large; API-path-only; structural-only testing (no live key). Builds on T1's derived schemas.

**Goal:** On the **anthropic-api** path, emit structured output as **native tool calls** (custom tool `input_schema` + forced `tool_choice`) — guaranteeing schema-valid calls with no prose-fenced-json parsing or repair loop. Used for (a) the Design Session slice emission and (b) the terminal agentic tool calls. The CLI runner keeps the prompt+parse+repair path (graceful per-runner degradation). The API tool-use idiom is sealed inside the ACL.

**Architecture:** A new `AnthropicApiChatRunner` in `llm_chat` (mirrors R1's worker `AnthropicApiRunner`: an injectable `SendFn` seam, real `reqwest::blocking` in prod, canned-response closures in tests). The `ChatRunner` trait gains a **structured-output method** with a default "unsupported" impl (CLI degrades) that the API runner overrides via forced `tool_choice`. Consumers select the structured path when the active chat runner supports it, else fall back.

---

## Current state (investigated)
- `llm_chat::ChatRunner` = `chat` + `chat_stream` only; one impl `ClaudeChatRunner` (CLI). No anthropic chat runner. Root hardwires `ClaudeChatRunner::new()` (`app/src/lib.rs:1286`).
- R1's `runners/src/anthropic_api.rs` (worker `AnthropicApiRunner`) is the template for the SendFn seam + `build_request_body`/`parse_response` + 429 mapping + ACL seal (no anthropic/HTTP/serde_json types cross the trait).
- T1 exposes `arg_schema::<T>()` per supplier + the published `ToolSpec.input_schema` — reuse as forced tool `input_schema`s. DS-Schema's `slice_schema::<T>()` is the slice tool schema.

## Tasks (TDD; fixtures/fakes only — no live key)

- [ ] **Task 1 — Structured-output seam on `ChatRunner`.** Add `async fn chat_structured(&self, req: &ChatRequest, tools: &[ChatToolDef], force: Option<&str>) -> Result<StructuredReply, ChatError>` where `ChatToolDef { name, description, input_schema: Value }` and `StructuredReply { tool_name: String, args: Value, usage }`. DEFAULT impl returns `ChatError::Unsupported` (so `ClaudeChatRunner` degrades). Add `fn supports_structured(&self) -> bool { false }`. NO anthropic idiom in these kernel types (only `Value`). Tests: default is unsupported. Commit.
- [ ] **Task 2 — `AnthropicApiChatRunner` (the ACL impl).** New `llm_chat/src/anthropic_api.rs` mirroring R1: injectable `SendFn`; pure `build_request_body` (model/system/messages/max_tokens/`tools`/`tool_choice`); pure `parse_response` extracting the `tool_use` block → `StructuredReply` (tool name + input → args + usage). Implements `ChatRunner`: `chat`/`chat_stream` (plain text, like R1) AND `chat_structured` (forced tool_choice) + `supports_structured()==true`. 429/error-envelope → `ChatError`. The `tool_use`/`tool_result`/anthropic JSON idiom stays INSIDE this file (sealed). Fixture tests for build + parse (incl. a forced tool_use response) + 429. Add `reqwest` to `llm_chat` (already a dep elsewhere). Commit.
- [ ] **Task 3 — Design Session uses the structured path on the API runner.** In `pipeline/src/design_session.rs` (or its caller), when the chat runner `supports_structured()`, emit the slice via `chat_structured` with one tool whose `input_schema = slice_schema::<Slice>()` and `force` = that tool — returning the validated slice with NO `extract_and_parse`/repair. Else the existing CLI prompt+repair path (unchanged). The seam is consumed via the `ChatRunner` trait — Design Session never sees the API idiom. Tests: a fake structured runner returns a slice directly (no repair); the CLI fake still uses prompt+parse+repair. Commit.
- [ ] **Task 4 — Terminal agentic loop uses native tool-use on the API runner.** When the chat runner supports structured output, drive the terminal tool step via `chat_structured` with the catalog tools (`name`/`description`/`input_schema` from T1) and let the model pick — returning a schema-valid `{tool,args}` natively (no fenced-json parse, no T1 repair turn needed). Else the T1 prompt+validate+repair path. The dispatch + validation (T1) remain as the safety net. Tests with a fake structured runner: a native tool call dispatches; CLI path still parses+validates+repairs. Commit.
- [ ] **Task 5 — Per-runner selection at the composition root.** A `chat_runner_for(...)` factory (mirror R1's `runner_for`) that picks `AnthropicApiChatRunner` (when an anthropic key is resolvable via the keychain/`api_key_env`, S1) else `ClaudeChatRunner` (default). Wire it where `ClaudeChatRunner::new()` is hardwired (`lib.rs:1286`). The API idiom never crosses; Runtime/CC hold an opaque `Arc<dyn ChatRunner>`. Test the factory (keychain-resolvable → api; none → cli). Commit.

## Verification gates
- `cd src-tauri && cargo test --workspace`
- `cd src-tauri && cargo clippy --workspace --all-targets -- -D warnings`
- `cd /Users/tim/projects/agent-bus-app && npx vitest run`
- `npx tsc --noEmit`; `bun run build`
- Tag: `git tag plan-T2-native-tooluse`

## Spec coverage
- Structured-output seam (API implements, CLI degrades) → Tasks 1,2. ✓
- Design Session forced-slice on the API path → Task 3. ✓
- Terminal native tool-use on the API path → Task 4. ✓
- Per-runner selection, ACL seal → Tasks 2,5. ✓

## Constraints
Local commits on `main`, NEVER push. Commit per task. **ACL seal:** no anthropic/HTTP/tool_use idiom crosses `ChatRunner`; only `ChatToolDef`/`StructuredReply` (`Value`-based) + the existing `ChatReply`. CLI runner unchanged (degrades via the default). The live HTTPS call is structural-only (SendFn fixtures) — no headless key. T1's validation/dispatch stay the universal fallback + safety net.
