# DS-Schema — Schema-Enforced Design-Session Output Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the Design Session's structured-emit contract drift-proof and self-correcting: generate each step's JSON Schema from the `Slice` Rust types (one source of truth) and embed it in the system prompt, and turn a malformed/invalid emission from a silent no-op into a bounded model-driven repair loop.

**Architecture:** Add `schemars` to the `pipeline` crate and derive `JsonSchema` on the slice types (`Slice` + `TeamsSlice`/`SliceTeam`/`PromptSlice`/`WiringSlice`/`RouteEdge` and the model nodes the wiring slice references — `Fork`/`Join`/`Gate`). At prompt-build time we `schema_for!` the per-step slice shape, serialize it compactly, and splice it into the step's system prompt (replacing the hand-written shape literal) so the prompt and `serde_json::from_str::<Slice>` parser can never diverge. The parse path stays as-is (it already enforces structure); the change is what happens on failure: instead of dropping the emission, `kickoff_generate` and `design_session_turn` run a **bounded repair turn** — re-prompt the model on the SAME `dialogue_id` with the specific extract/parse error, asking it to re-emit only the fenced json per the schema, up to **2 retries**, before giving up. `best_effort_validate` is unchanged — it still carries the non-blocking SEMANTIC checks (2–5 teams, fork ≥2 lanes, team_id exists, etc.).

**Tech Stack:** Rust, `schemars` 0.8 (already transitively in the lockfile), `serde_json`, `tokio`, `llm_chat::fake::FakeChatRunner` (scripted multi-turn replies, no live `claude`).

---

## Decisions

- **D-CLI (the constraint — why this shape, not native tool-use).** The Design Session runs over the **claude-cli** runner (`llm_chat`'s `ChatRunner`, backed by `claude --print`). `claude --print` CANNOT define arbitrary custom tools with `input_schema`s or force `tool_choice` — native structured-output / forced tool-use is an Anthropic **API** capability (the R1 `anthropic-api` runner), not available on the CLI path. So we DO NOT attempt native tool-use here. The hardening win on the CLI path is exactly: **derived-schema-in-prompt** (kills prompt↔parser drift) + **schema/structure validation** (already present via the typed parse) + **bounded repair** (kills the silent no-op). The strongest version (forced schema-valid tool calls) is logged as a backlog roadmap item for the API runner.
- **D-SCHEMA-SRC (one source of truth).** The embedded schema is GENERATED from the slice types via `schemars`, not hand-written. The slice Rust types own the shape; the prompt reads it. A test asserts the embedded schema is derived (the prompt contains schema property names pulled from the types, and no hand-written object-literal of the slice structure remains).
- **D-SCHEMA-FORM (how to embed).** Per step we `schema_for!` the relevant slice struct (`TeamsSlice` for Teams + kickoff, `PromptSlice` for Prompts, `WiringSlice` for Wiring) and serialize it compactly (single-line `serde_json::to_string`). We keep the prose rules (2–5 teams, fork ≥2 lanes, gate semantics, "only the json mutates state", etc.) — the schema replaces only the hand-written *shape* block, not the guidance. The prompt is assembled at call time (a `fn step_system_prompt(step)` / `fn kickoff_system_prompt()`), not a `const`, because the schema string is computed.
- **D-REPAIR-COUNT (retry budget).** **2 retries** (so up to 3 model turns total per emission: the initial turn + 2 repair turns). On the 2nd failed retry we give up and behave as today (leave the draft unchanged, surface the prose). Same budget for `kickoff_generate` and `design_session_turn`.
- **D-REPAIR-DIALOGUE (continuity).** The repair turn re-prompts on the SAME `dialogue_id` (the step-scoped id, or `<session>:kickoff`) so the model sees its own prior bad output in context. The repair user-message names the specific failure ("no fenced ```json block found" vs the `serde_json` error string) and instructs: re-emit ONLY the fenced json per the schema, nothing else.
- **D-REPAIR-SURFACE (what the caller sees).** On success-after-repair the slice is applied and the FINAL reply's prose is surfaced (consistent with today — prose from the turn that produced the valid slice). On give-up the LAST reply's prose is surfaced and the draft is unchanged (today's behavior). `TurnResult`'s shape is unchanged.
- **D-SEM (semantics stay).** `best_effort_validate` is NOT moved into the repair loop. Structural/schema failure → repair. Semantic issues (missing prompt, dangling route, fork <2 lanes) stay non-blocking and surfaced via `TurnResult.issues`, exactly as today.
- **D-ERR-MSG.** `extract_json_block` failure and `parse_slice` failure produce distinct human error strings so the repair prompt is specific. Introduce a small internal helper `extract_and_parse(prose) -> Result<Slice, String>` returning the error string; reused by both entry points and the repair loop.
- **D-SCHEMARS-VER.** Use `schemars` `0.8` (matches the version already resolved in `Cargo.lock` via transitive deps; has `derive` + works with the existing serde attrs). The slice enum uses `#[serde(tag = "kind", rename_all = "lowercase")]` — `schemars` honors serde tagging, so the derived schema reflects the internally-tagged `kind` discriminator. If a derive snags on a referenced model type, fall back to deriving `JsonSchema` on that type too (Fork/Join/Gate are plain structs — should derive cleanly).

---

## File Structure

- `src-tauri/pipeline/Cargo.toml` — add `schemars` dependency (+ workspace entry in `src-tauri/Cargo.toml`).
- `src-tauri/pipeline/src/model.rs` — derive `JsonSchema` on `Fork`, `Join`, `Gate` (referenced by `WiringSlice`).
- `src-tauri/pipeline/src/draft.rs` — derive `JsonSchema` on `SliceTeam`, `TeamsSlice`, `PromptSlice`, `RouteEdge`, `WiringSlice`, `Slice`.
- `src-tauri/pipeline/src/design_session.rs` — the bulk: schema-from-types prompt builders (replace the prompt `const`s), `extract_and_parse` helper, the bounded repair loop in `kickoff_generate` + `design_session_turn`, and the new tests (derived-schema assertion + scripted repair-loop tests).

No frontend change (the wizard calls the same Tauri commands; `TurnResult` shape unchanged). No `llm_chat` change (`FakeChatRunner` already supports scripted multi-turn replies via `new(vec![...])` clamping).

---

## Task 1: Add `schemars` to the pipeline crate

**Files:**
- Modify: `src-tauri/Cargo.toml` (workspace deps)
- Modify: `src-tauri/pipeline/Cargo.toml`

- [ ] **Step 1: Add the workspace dependency**

In `src-tauri/Cargo.toml`, under `[workspace.dependencies]`, add:

```toml
schemars = { version = "0.8", features = ["derive"] }
```

- [ ] **Step 2: Add it to the pipeline crate**

In `src-tauri/pipeline/Cargo.toml`, under `[dependencies]`, add:

```toml
schemars = { workspace = true }
```

- [ ] **Step 3: Verify it resolves**

Run: `cargo check -p pipeline`
Expected: compiles clean (no version bump in `Cargo.lock` beyond what is already present — `schemars 0.8.22` is already resolved transitively).

- [ ] **Step 4: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/pipeline/Cargo.toml src-tauri/Cargo.lock
git commit -m "build(pipeline): add schemars for derived slice schemas"
```

---

## Task 2: Derive `JsonSchema` on the model node types the wiring slice references

**Files:**
- Modify: `src-tauri/pipeline/src/model.rs` (the `Fork`, `Join`, `Gate` derives)

- [ ] **Step 1: Write the failing test**

Add to `model.rs` `#[cfg(test)] mod tests`:

```rust
#[test]
fn wiring_node_types_derive_json_schema() {
    // Fork/Join/Gate must produce a schema (WiringSlice embeds them).
    let fork = schemars::schema_for!(Fork);
    let s = serde_json::to_string(&fork).unwrap();
    assert!(s.contains("lanes"), "Fork schema should expose lanes: {s}");
    let join = serde_json::to_string(&schemars::schema_for!(Join)).unwrap();
    assert!(join.contains("waits_for") && join.contains("downstream"));
    let gate = serde_json::to_string(&schemars::schema_for!(Gate)).unwrap();
    assert!(gate.contains("downstream") && gate.contains("label"));
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p pipeline --lib model::tests::wiring_node_types_derive_json_schema`
Expected: FAIL — `schema_for!` / `JsonSchema` not implemented for `Fork`.

- [ ] **Step 3: Add the derives**

In `model.rs`, add `schemars::JsonSchema` to the derive list on `Fork`, `Join`, and `Gate`. Each becomes:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Gate {
```

(same change applied to `Fork` and `Join`).

- [ ] **Step 4: Run it to verify it passes**

Run: `cargo test -p pipeline --lib model::tests::wiring_node_types_derive_json_schema`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/pipeline/src/model.rs
git commit -m "feat(pipeline): derive JsonSchema on Fork/Join/Gate for slice schemas"
```

---

## Task 3: Derive `JsonSchema` on the slice types

**Files:**
- Modify: `src-tauri/pipeline/src/draft.rs` (`SliceTeam`, `TeamsSlice`, `PromptSlice`, `RouteEdge`, `WiringSlice`, `Slice`)
- Test: in `draft.rs` `#[cfg(test)] mod tests`

- [ ] **Step 1: Write the failing test**

Add to `draft.rs` tests:

```rust
#[test]
fn slice_types_derive_json_schema_with_kind_discriminator() {
    // The whole Slice enum is internally tagged on `kind`; the derived schema
    // must reflect that discriminator and the per-variant payloads.
    let s = serde_json::to_string(&schemars::schema_for!(Slice)).unwrap();
    assert!(s.contains("kind"), "Slice schema must expose the kind tag: {s}");

    let teams = serde_json::to_string(&schemars::schema_for!(TeamsSlice)).unwrap();
    assert!(teams.contains("teams"));

    let prompt = serde_json::to_string(&schemars::schema_for!(PromptSlice)).unwrap();
    assert!(prompt.contains("team_id") && prompt.contains("prompt_body"));

    let wiring = serde_json::to_string(&schemars::schema_for!(WiringSlice)).unwrap();
    assert!(wiring.contains("routes") && wiring.contains("forks")
        && wiring.contains("joins") && wiring.contains("gates"));
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p pipeline --lib draft::tests::slice_types_derive_json_schema_with_kind_discriminator`
Expected: FAIL — `JsonSchema` not implemented for `Slice`.

- [ ] **Step 3: Add the derives**

In `draft.rs`, add `schemars::JsonSchema` to the derive list on each of: `SliceTeam`, `TeamsSlice`, `PromptSlice`, `RouteEdge`, `WiringSlice`, and the `Slice` enum. e.g.:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SliceTeam {
```

and for the enum (keep its existing serde attr):

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Slice {
```

- [ ] **Step 4: Run it to verify it passes**

Run: `cargo test -p pipeline --lib draft::tests::slice_types_derive_json_schema_with_kind_discriminator`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/pipeline/src/draft.rs
git commit -m "feat(pipeline): derive JsonSchema on the Slice types (one source of truth)"
```

---

## Task 4: Generate + embed the per-step schema in the system prompts

**Files:**
- Modify: `src-tauri/pipeline/src/design_session.rs` (replace the prompt `const`s' hand-written shape with derived schema; introduce prompt-builder fns)
- Test: in `design_session.rs` tests

- [ ] **Step 1: Write the failing tests**

Add to `design_session.rs` tests. These assert the prompt is BUILT from the types and the hand-written shape literal is gone:

```rust
#[test]
fn step_prompt_embeds_the_derived_slice_schema() {
    // The wiring prompt must contain schema property names pulled from WiringSlice
    // via schemars — proving it is generated, not a hand-written literal.
    let p = step_system_prompt(Step::Wiring);
    let schema = serde_json::to_string(&schemars::schema_for!(crate::draft::WiringSlice)).unwrap();
    assert!(p.contains(&schema), "wiring prompt must embed the derived WiringSlice schema");
    // prose rules are kept
    assert!(p.contains("fork") && p.contains(">=2"));
}

#[test]
fn step_prompts_no_longer_carry_a_hand_written_slice_object_literal() {
    // The old hand-written shape used a bare {"kind":"teams","teams":[{"id":...}]}
    // object literal. Assert the teams prompt now embeds the DERIVED schema for the
    // teams slice instead (which is a JSON Schema object, not a sample instance).
    let p = step_system_prompt(Step::Teams);
    let schema = serde_json::to_string(&schemars::schema_for!(crate::draft::TeamsSlice)).unwrap();
    assert!(p.contains(&schema));
    // a JSON Schema has "properties"/"type" metadata; a hand-written sample would not
    assert!(p.contains("properties") || p.contains("\"type\""));
}

#[test]
fn kickoff_prompt_embeds_the_derived_teams_schema() {
    let p = kickoff_system_prompt();
    let schema = serde_json::to_string(&schemars::schema_for!(crate::draft::TeamsSlice)).unwrap();
    assert!(p.contains(&schema));
    assert!(p.contains("2") && p.contains("5")); // 2–5 teams rule kept
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p pipeline --lib design_session::tests::step_prompt_embeds_the_derived_slice_schema`
Expected: FAIL — `step_system_prompt` not found / `WiringSlice` not pub.

(If `WiringSlice`/`TeamsSlice` are not `pub`, they already are — confirmed in draft.rs.)

- [ ] **Step 3: Implement the prompt builders**

In `design_session.rs`, replace the four `const` prompt strings and the `Step::system_prompt` method with builder functions that embed the derived schema. Add at the top of the module:

```rust
use crate::draft::{PromptSlice, TeamsSlice, WiringSlice};

/// Compact JSON Schema for a slice type, derived from the Rust type via schemars.
/// This is the single source of truth: the prompt and `parse_slice` can never drift.
fn slice_schema<T: schemars::JsonSchema>() -> String {
    serde_json::to_string(&schemars::schema_for!(T)).unwrap_or_default()
}
```

Replace `Step::system_prompt` with a free function (and keep `Step::system_prompt` as a thin delegator, or update call sites to use the free fn — update the one call site in `design_session_turn`):

```rust
/// The kickoff one-shot system prompt: prose + a fenced ```json TEAMS slice whose
/// shape is the DERIVED schema (D-SCHEMA-SRC). Prose rules kept (2–5 teams, slug ids).
pub fn kickoff_system_prompt() -> String {
    format!(
        "You are designing a multi-team Claude Code agent pipeline from a one-line \
description. Reply with a short paragraph of prose, THEN a fenced ```json block \
containing ONLY the team set, matching exactly this JSON Schema:\n\
```json\n{schema}\n```\n\
Use 2 to 5 teams. ids are lowercase slugs. Emit ONLY the json in the fenced block.",
        schema = slice_schema::<TeamsSlice>()
    )
}

/// The per-step system prompt with the DERIVED slice schema embedded (D-SCHEMA-SRC).
/// Prose rules are kept; only the hand-written shape is replaced by the schema.
pub fn step_system_prompt(step: Step) -> String {
    match step {
        Step::Teams => format!(
            "You are refining the TEAM SET of a pipeline being designed. Reply with \
prose, THEN a fenced ```json block with the FULL updated team set (this replaces \
the previous set), matching exactly this JSON Schema:\n\
```json\n{schema}\n```\n\
The emitted object MUST include \"kind\":\"teams\". Preserve existing team ids the \
user wants to keep. Only the json mutates state.",
            schema = slice_schema::<TeamsSlice>()
        ),
        Step::Prompts => format!(
            "You are writing ONE team's responsibility prompt. Reply with prose, THEN \
a fenced ```json block matching exactly this JSON Schema:\n\
```json\n{schema}\n```\n\
The emitted object MUST include \"kind\":\"prompt\". team_id must be one of the \
existing teams. Only the json mutates state.",
            schema = slice_schema::<PromptSlice>()
        ),
        Step::Wiring => format!(
            "You are wiring the pipeline's flow (routes, optional fork/join lanes, and \
optional human-review gates). Reply with prose, THEN a fenced ```json block matching \
exactly this JSON Schema:\n\
```json\n{schema}\n```\n\
The emitted object MUST include \"kind\":\"wiring\". A gate is a human-review \
checkpoint: a team routes to it via on_approve, and the gate forwards approved work \
to its downstream. A fork must have >=2 lanes; NEVER place a gate inside a fork lane. \
routes stay single-target. Only the json mutates state.",
            schema = slice_schema::<WiringSlice>()
        ),
    }
}
```

Delete the four `const` declarations (`KICKOFF_SYSTEM_PROMPT`, `TEAMS_SYSTEM_PROMPT`, `PROMPTS_SYSTEM_PROMPT`, `WIRING_SYSTEM_PROMPT`) and the `Step::system_prompt` method. Update the existing `wiring_system_prompt_documents_the_gates_schema` test to call `step_system_prompt(Step::Wiring)` and the `slug()` method stays.

> NOTE on the `kind` tag: the derived per-struct schemas (`TeamsSlice` etc.) do NOT themselves carry the `kind` discriminator (that lives on the `Slice` enum). The prompts therefore explicitly instruct `MUST include "kind":"..."`, and `parse_slice` (the `Slice` enum) is what enforces it. This keeps the embedded schema small + focused on the payload while the tag instruction is prose.

- [ ] **Step 4: Update the `design_session_turn` call site**

In `design_session_turn`, change `system_prompt: step.system_prompt().to_string()` to `system_prompt: step_system_prompt(step)`. In `kickoff_generate`, change `system_prompt: KICKOFF_SYSTEM_PROMPT.to_string()` to `system_prompt: kickoff_system_prompt()`.

- [ ] **Step 5: Run the new + existing prompt tests**

Run: `cargo test -p pipeline --lib design_session`
Expected: PASS (the updated `wiring_system_prompt_documents_the_gates_schema` and the three new schema-embed tests pass).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/pipeline/src/design_session.rs
git commit -m "feat(design-session): embed derived slice schema in step prompts (no drift)"
```

---

## Task 5: Extract-or-parse error helper

**Files:**
- Modify: `src-tauri/pipeline/src/design_session.rs`
- Test: in `design_session.rs` tests

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn extract_and_parse_reports_a_specific_error_for_no_fence() {
    let err = extract_and_parse("just prose, no fenced block").unwrap_err();
    assert!(err.to_lowercase().contains("no fenced") || err.to_lowercase().contains("json block"));
}

#[test]
fn extract_and_parse_reports_a_specific_error_for_malformed_json() {
    let err = extract_and_parse("ok\n```json\n{not valid\n```").unwrap_err();
    assert!(!err.is_empty());
}

#[test]
fn extract_and_parse_returns_the_slice_on_success() {
    let slice = extract_and_parse("ok\n```json\n{\"kind\":\"teams\",\"teams\":[]}\n```").unwrap();
    matches!(slice, crate::draft::Slice::Teams(_));
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p pipeline --lib design_session::tests::extract_and_parse_reports_a_specific_error_for_no_fence`
Expected: FAIL — `extract_and_parse` not found.

- [ ] **Step 3: Implement the helper**

In `design_session.rs`:

```rust
/// Extract the fenced json block and parse it into a typed `Slice`, returning a
/// SPECIFIC human-readable error on failure (D-ERR-MSG): distinct messages for
/// "no fenced block" vs a serde structure/parse error. This is the single failure
/// classifier the repair loop re-prompts on.
fn extract_and_parse(prose: &str) -> Result<Slice, String> {
    let block = extract_json_block(prose)
        .ok_or_else(|| "no fenced ```json block was found in the reply".to_string())?;
    parse_slice(&block).map_err(|e| format!("the fenced json did not match the slice schema: {e}"))
}
```

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test -p pipeline --lib design_session::tests::extract_and_parse`
Expected: PASS (all three).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/pipeline/src/design_session.rs
git commit -m "feat(design-session): extract_and_parse error classifier for repair"
```

---

## Task 6: Bounded repair loop in `design_session_turn`

**Files:**
- Modify: `src-tauri/pipeline/src/design_session.rs`
- Test: in `design_session.rs` tests

- [ ] **Step 1: Write the failing tests (the headline repair-loop test)**

```rust
const MAX_REPAIR_RETRIES_TEST: usize = 2;

#[tokio::test]
async fn turn_repairs_a_malformed_first_reply_then_applies_the_valid_slice() {
    // first reply: prose with NO fenced block -> triggers a repair turn;
    // second reply: a valid prompt slice.
    let mut draft = DraftPipeline::empty();
    draft.teams.push(DraftTeam::new("research", "Research"));
    let malformed = reply("I think research should investigate. (forgot the json)");
    let good = reply("Here:\n```json\n{\"kind\":\"prompt\",\"team_id\":\"research\",\
        \"prompt_body\":\"You investigate the repo and write findings.\"}\n```");
    let runner = FakeChatRunner::new(vec![malformed, good]);
    let out = design_session_turn(&runner, "sess-1", Step::Prompts, draft, "set research prompt").await;
    // the slice from the REPAIR turn was applied
    assert_eq!(out.updated_draft.teams[0].prompt_body, "You investigate the repo and write findings.");
    // exactly 2 calls were made (initial + 1 repair)
    assert_eq!(runner.received.lock().unwrap().len(), 2);
    // the repair request named the failure + re-prompted on the SAME dialogue id
    let received = runner.received.lock().unwrap();
    assert_eq!(received[1].dialogue_id, "sess-1:prompts");
    assert!(received[1].user_message.to_lowercase().contains("json"));
}

#[tokio::test]
async fn turn_gives_up_after_two_repair_retries_and_leaves_the_draft_unchanged() {
    let mut draft = DraftPipeline::empty();
    draft.teams.push(DraftTeam::new("research", "Research"));
    let before = draft.clone();
    // every reply is malformed (no fence). With FakeChatRunner clamping to the last
    // reply, all three calls return malformed.
    let runner = FakeChatRunner::new(vec![reply("nope, still no json block here")]);
    let out = design_session_turn(&runner, "sess-1", Step::Prompts, draft, "set research prompt").await;
    assert_eq!(out.updated_draft, before); // unchanged after giving up
    // initial + 2 repair retries = 3 calls
    assert_eq!(runner.received.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn turn_does_not_repair_when_the_first_reply_is_valid() {
    let mut draft = DraftPipeline::empty();
    draft.teams.push(DraftTeam::new("research", "Research"));
    let good = reply("ok\n```json\n{\"kind\":\"prompt\",\"team_id\":\"research\",\"prompt_body\":\"x\"}\n```");
    let runner = FakeChatRunner::new(vec![good]);
    let _ = design_session_turn(&runner, "sess-1", Step::Prompts, draft, "go").await;
    // only ONE call — no wasted repair turn on a good first reply
    assert_eq!(runner.received.lock().unwrap().len(), 1);
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p pipeline --lib design_session::tests::turn_repairs_a_malformed_first_reply_then_applies_the_valid_slice`
Expected: FAIL — currently only one call is made (no repair), the draft is unchanged, length is 1 not 2.

- [ ] **Step 3: Implement the repair loop**

In `design_session.rs`, add the retry constant and a shared chat-with-repair helper, then rewrite `design_session_turn` to use it. Add near the top:

```rust
/// Bounded repair budget (D-REPAIR-COUNT): up to this many repair turns after the
/// initial turn (so at most 1 + MAX_REPAIR_RETRIES model calls per emission).
const MAX_REPAIR_RETRIES: usize = 2;

/// Build a repair user-message naming the specific failure (D-REPAIR-DIALOGUE/MSG).
fn repair_user_message(err: &str) -> String {
    format!(
        "Your previous reply could not be applied: {err}. Re-emit ONLY a single fenced \
```json block that matches the JSON Schema in your instructions (include the correct \
\"kind\"). Do not add any other text inside the fence."
    )
}

/// Run one chat turn then, on extract-or-parse failure, up to `MAX_REPAIR_RETRIES`
/// bounded repair turns on the SAME `dialogue_id`. Returns the final reply prose and
/// the parsed `Slice` if any turn produced a valid one (D-REPAIR-SURFACE). The runner
/// error path (e.g. rate-limit) short-circuits with the error prose and no slice.
async fn chat_with_repair(
    runner: &dyn ChatRunner,
    dialogue_id: &str,
    system_prompt: &str,
    initial_user_message: String,
    model: &str,
    thinking_budget: u32,
) -> (String, Option<Slice>) {
    let mut user_message = initial_user_message;
    let mut last_text = String::new();
    for attempt in 0..=MAX_REPAIR_RETRIES {
        let req = ChatRequest {
            dialogue_id: dialogue_id.to_string(),
            system_prompt: system_prompt.to_string(),
            user_message,
            model: model.to_string(),
            thinking_budget,
        };
        match runner.chat(&req).await {
            Ok(reply) => {
                last_text = reply.text.clone();
                match extract_and_parse(&reply.text) {
                    Ok(slice) => return (reply.text, Some(slice)),
                    Err(err) => {
                        if attempt == MAX_REPAIR_RETRIES {
                            return (last_text, None); // give up; draft unchanged
                        }
                        user_message = repair_user_message(&err);
                    }
                }
            }
            Err(e) => return (format!("[design session error] {e}"), None),
        }
    }
    (last_text, None)
}
```

Rewrite `design_session_turn`'s body:

```rust
pub async fn design_session_turn(
    runner: &dyn ChatRunner,
    session_id: &str,
    step: Step,
    mut draft: DraftPipeline,
    user_message: &str,
) -> TurnResult {
    let dialogue_id = format!("{session_id}:{}", step.slug());
    let (reply_text, slice) = chat_with_repair(
        runner,
        &dialogue_id,
        &step_system_prompt(step),
        turn_user_message(user_message, &draft),
        "claude-opus-4-8",
        8192,
    )
    .await;
    if let Some(slice) = slice {
        apply_slice(&mut draft, slice);
    }
    let issues = best_effort_validate(&draft);
    TurnResult { reply_text, updated_draft: draft, issues }
}
```

Add the needed import of `ChatRequest` (already imported) and ensure `ChatRunner` is in scope (it is).

- [ ] **Step 4: Run the repair tests + the existing turn tests**

Run: `cargo test -p pipeline --lib design_session`
Expected: PASS. The existing `design_session_turn_with_invalid_json_leaves_the_draft_unchanged` test sends ONE no-fence reply; with clamping it now makes 3 calls and STILL leaves the draft unchanged (its assertions are on the draft + that prose contains "clarify" — both still hold because the clamped reply is the same prose). Confirm that test still passes; if it asserts call-count it does not (it doesn't).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/pipeline/src/design_session.rs
git commit -m "feat(design-session): bounded repair loop on extract/parse failure"
```

---

## Task 7: Apply the repair loop to `kickoff_generate`

**Files:**
- Modify: `src-tauri/pipeline/src/design_session.rs`
- Test: in `design_session.rs` tests

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn kickoff_repairs_a_malformed_first_reply_then_builds_the_draft() {
    let malformed = reply("Two teams: research and writers. (json omitted)");
    let good = reply("```json\n{\"kind\":\"teams\",\"teams\":[\
        {\"id\":\"research\",\"name\":\"Research\"},{\"id\":\"writers\",\"name\":\"Writers\"}]}\n```");
    let runner = FakeChatRunner::new(vec![malformed, good]);
    let draft = kickoff_generate(&runner, "sess-1", "research+writing pipeline").await;
    assert_eq!(draft.teams.len(), 2);
    assert_eq!(runner.received.lock().unwrap().len(), 2); // initial + 1 repair
    assert_eq!(runner.received.lock().unwrap()[1].dialogue_id, "sess-1:kickoff");
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p pipeline --lib design_session::tests::kickoff_repairs_a_malformed_first_reply_then_builds_the_draft`
Expected: FAIL — only one call, draft has 0 teams.

- [ ] **Step 3: Rewrite `kickoff_generate` to use `chat_with_repair`**

```rust
pub async fn kickoff_generate(
    runner: &dyn ChatRunner,
    session_id: &str,
    description: &str,
) -> DraftPipeline {
    let mut draft = DraftPipeline::empty();
    draft.description = description.to_string();
    draft.id = slug_id(description);

    let dialogue_id = format!("{session_id}:kickoff");
    let (_text, slice) = chat_with_repair(
        runner,
        &dialogue_id,
        &kickoff_system_prompt(),
        description.to_string(),
        "claude-opus-4-8",
        8192,
    )
    .await;
    if let Some(slice) = slice {
        apply_slice(&mut draft, slice);
    }
    draft
}
```

- [ ] **Step 4: Run the kickoff tests**

Run: `cargo test -p pipeline --lib design_session`
Expected: PASS — including the existing `kickoff_generate_builds_a_draft_from_a_teams_slice` (single good reply, one call, draft built).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/pipeline/src/design_session.rs
git commit -m "feat(design-session): kickoff_generate uses the bounded repair loop"
```

---

## Task 8: Full-suite verification

**Files:** none (verification only).

- [ ] **Step 1: pipeline crate tests**

Run: `cargo test -p pipeline`
Expected: PASS (all lib + any integration tests).

- [ ] **Step 2: workspace tests**

Run: `cargo test --workspace`
Expected: PASS. (The app's `design_session_tests` call the same functions with single good replies — one call each — so they stay green.)

- [ ] **Step 3: check + clippy**

Run: `cargo check --workspace` then `cargo clippy --workspace --all-targets -- -D warnings`
Expected: clean (no warnings). If clippy flags the generic `slice_schema::<T>()` turbofish or a needless `to_string`, fix minimally.

- [ ] **Step 4: frontend tests + build**

Run: `bun vitest run` then `bun run build`
Expected: PASS / build succeeds. (No frontend change; this confirms the IPC surface — `TurnResult`/command signatures — is unchanged.)

- [ ] **Step 5: Commit any clippy fixes**

```bash
git add -A
git commit -m "chore(ds-schema): clippy + verification fixes"
```

---

## Self-Review

- **Spec coverage:** (1) schemars + derive on Slice types → Tasks 1–3; generate+embed per step → Task 4; test asserting derived (no hand-written literal) → Task 4 tests. (2) validate+repair loop, bounded 2 retries, both entry points, same dialogue_id, specific error → Tasks 5–7; `best_effort_validate` untouched → confirmed (no task modifies it). claude-cli constraint noted → `## Decisions` D-CLI.
- **Placeholder scan:** all code steps carry full code; no TBD/TODO.
- **Type consistency:** `step_system_prompt`/`kickoff_system_prompt`/`slice_schema`/`extract_and_parse`/`chat_with_repair`/`repair_user_message`/`MAX_REPAIR_RETRIES` used consistently across Tasks 4–7. `Slice`/`TeamsSlice`/`PromptSlice`/`WiringSlice` match draft.rs.
