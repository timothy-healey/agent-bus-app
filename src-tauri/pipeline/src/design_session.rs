//! Design Session — the ephemeral, AI-assisted authoring dialogue that produces a
//! pipeline (DOMAIN.md → Pipeline Authoring). Distinct from the terminal's
//! `Conversation` aggregate; conducted over the llm_chat ACL. This module holds
//! the kickoff one-shot + the per-step turn logic and the structured-emit
//! contract: the model returns PROSE plus a fenced ```json slice; the backend
//! extracts + parses + best-effort-applies the slice (the trust boundary). The
//! prose is never parsed for state.

use crate::draft::{apply_kickoff_slice, apply_slice, best_effort_validate, DraftPipeline, KickoffSlice, PromptSlice, Slice, TeamsSlice, WiringSlice};
use llm_chat::chat::{ChatRequest, ChatRunner, ChatToolDef};
use serde::{Deserialize, Serialize};

/// Compact JSON Schema for a slice type, derived from the Rust type via schemars.
/// This is the single source of truth (the **slice schema**): the prompt and
/// `parse_slice` are generated from the SAME types, so they can never drift.
fn slice_schema<T: schemars::JsonSchema>() -> String {
    serde_json::to_string(&schemars::schema_for!(T)).unwrap_or_default()
}

/// The slice schema as a JSON `Value` (the structured-output path's tool
/// `input_schema`). Same single source of truth as `slice_schema` — derived from
/// the SAME Rust type via schemars — so the prose-prompt schema and the native
/// tool schema can never drift. Falls back to a permissive object schema if
/// serialization ever fails (the parse-after still validates the shape).
fn slice_schema_value<T: schemars::JsonSchema>() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(T))
        .unwrap_or_else(|_| serde_json::json!({ "type": "object" }))
}

/// The single tool name the structured-emit path forces. The model MUST call it,
/// so its `input_schema` (the slice schema) is the only shape it can return.
const EMIT_TOOL: &str = "emit_slice";

/// Emit a typed value `T` via the runner's NATIVE structured-output path: one
/// forced tool whose `input_schema` is the derived schema for `T`. The model's
/// `StructuredReply.args` is the slice object itself, deserialized into `T` — no
/// prose-fenced-json extraction and NO repair loop (forced tool_choice guarantees
/// a schema-shaped call). The runner is consumed via the `ChatRunner` trait only:
/// Design Session never sees the anthropic tool_use idiom (the ACL seal). Returns
/// `(reply_prose, Some(T))` on success, `(error_or_empty, None)` otherwise — the
/// SAME contract as `chat_with_repair_parsed` so callers are path-agnostic.
async fn emit_structured<T: serde::de::DeserializeOwned + schemars::JsonSchema>(
    runner: &dyn ChatRunner,
    dialogue_id: &str,
    system_prompt: &str,
    user_message: String,
    tool_description: &str,
) -> (String, Option<T>) {
    let tools = [ChatToolDef {
        name: EMIT_TOOL.to_string(),
        description: tool_description.to_string(),
        input_schema: slice_schema_value::<T>(),
    }];
    let req = ChatRequest {
        dialogue_id: dialogue_id.to_string(),
        system_prompt: system_prompt.to_string(),
        user_message,
        model: "claude-opus-4-8".to_string(),
        thinking_budget: 8192,
        working_dir: None,
    };
    match runner.chat_structured(&req, &tools, Some(EMIT_TOOL)).await {
        Ok(reply) => match serde_json::from_value::<T>(reply.args) {
            // The forced schema guarantees the shape; a deserialize miss here is
            // treated like the prose path's parse miss — draft unchanged.
            Ok(value) => (String::new(), Some(value)),
            Err(_) => (String::new(), None),
        },
        Err(e) => (format!("[design session error] {e}"), None),
    }
}

/// The kickoff one-shot system prompt (G5/G3): prose + a fenced ```json
/// **kickoff slice** whose shape is the DERIVED schema (one source of truth). The
/// kickoff now emits a COMPLETE recommended graph in one Generate — teams with
/// full `prompt_body` bodies (G5 — so the canvas is prefilled), an EXPLICIT `role`
/// per team (G3 — producer/reviewer; never rely on the name regex), each team's
/// route edges, the human-review `gates`, and the terminal `escalations` (a
/// `needs-human` escalation for declines). The prose rules pin the well-formed
/// review structure B4's canvas warning checks for (every reviewer arrives with
/// approve + revise→its writer + decline→needs-human).
pub fn kickoff_system_prompt() -> String {
    format!(
        "You are designing a multi-team Claude Code agent pipeline from a one-line \
description. Reply with a short paragraph of prose, THEN a fenced ```json block \
containing the COMPLETE recommended graph. The object MUST match this JSON Schema \
(the kickoff payload):\n\
```json\n{schema}\n```\n\
Rules:\n\
- Use 2 to 6 teams; ids are lowercase slugs.\n\
- Every team MUST have a non-empty `prompt_body`: a clear 2-4 sentence \
responsibility prompt written in the second person (\"You ...\").\n\
- Set each team's `role` EXPLICITLY to \"producer\" (does work, hands off) or \
\"reviewer\" (judges upstream work and emits approve/revise/reject). Include at \
least one reviewer for a non-trivial flow.\n\
- A producer routes its `on_approve` forward to the next stage (or to a gate).\n\
- A reviewer MUST set all three routes: `on_approve` to its downstream (or a \
gate), `on_revise` BACK to the team that produced the work it reviews, and \
`on_reject` to \"needs-human\".\n\
- Add a human-review `gate` (with a downstream) where a person should sign off \
(e.g. after a spec is approved); a reviewer's on_approve may target the gate.\n\
- Include one terminal escalation with id \"needs-human\" so declines have a home.\n\
- The first team in the list is the entry/source. Every other team must be \
reachable by following forward (on_approve / gate downstream) edges.\n\
Emit ONLY the json in the fenced block.",
        schema = slice_schema::<KickoffSlice>()
    )
}

/// The per-step Design Session system prompt with the DERIVED slice schema embedded
/// (the **slice schema** — one source of truth). The prose rules are kept; only the
/// hand-written shape literal is replaced by the generated schema. Built at call
/// time because the schema string is computed from the types.
pub fn step_system_prompt(step: Step) -> String {
    match step {
        Step::Teams => format!(
            "You are refining the TEAM SET of a pipeline being designed. Reply with \
prose, THEN a fenced ```json block with the FULL updated team set (this replaces \
the previous set). The object MUST include \"kind\":\"teams\" and match this JSON \
Schema (the team-set payload):\n\
```json\n{schema}\n```\n\
Preserve existing team ids the user wants to keep. Only the json mutates state.",
            schema = slice_schema::<TeamsSlice>()
        ),
        Step::Prompts => format!(
            "You are writing ONE team's responsibility prompt. Reply with prose, THEN \
a fenced ```json block. The object MUST include \"kind\":\"prompt\" and match this \
JSON Schema (the prompt payload):\n\
```json\n{schema}\n```\n\
team_id must be one of the existing teams. Only the json mutates state.",
            schema = slice_schema::<PromptSlice>()
        ),
        Step::Wiring => format!(
            "You are wiring the pipeline's flow (routes, optional fork/join lanes, and \
optional human-review gates). Reply with prose, THEN a fenced ```json block. The \
object MUST include \"kind\":\"wiring\" and match this JSON Schema (the wiring \
payload):\n\
```json\n{schema}\n```\n\
A gate is a human-review checkpoint: a team routes to it via on_approve, and the \
gate forwards approved work to its downstream. A fork must have >=2 lanes; NEVER \
place a gate inside a fork lane. routes stay single-target. Only the json mutates state.",
            schema = slice_schema::<WiringSlice>()
        ),
    }
}

/// Extract the first fenced code block from the model's prose. Prefers a
/// ```` ```json ```` fence; falls back to the first bare ```` ``` ```` fence.
/// Returns the block's inner text (no fences), or None when there is no fence.
/// This is the only place the model's free text is scanned for structure
/// (Decision D2).
pub fn extract_json_block(prose: &str) -> Option<String> {
    // Prefer a ```json fence.
    if let Some(start) = prose.find("```json") {
        let after = &prose[start + "```json".len()..];
        if let Some(end) = after.find("```") {
            return Some(after[..end].trim().to_string());
        }
    }
    // Fall back to the first bare ``` fence.
    if let Some(start) = prose.find("```") {
        let after = &prose[start + 3..];
        // Skip an optional language token on the same line.
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

/// Parse an extracted block into a typed Slice (internally tagged on `kind`).
/// Errors on anything that isn't a valid slice — the caller treats an error as
/// "leave the draft unchanged, surface the prose" (Decision D2).
pub fn parse_slice(block: &str) -> Result<Slice, serde_json::Error> {
    serde_json::from_str::<Slice>(block)
}

/// Extract the fenced json block and parse it into a typed `Slice`, returning a
/// SPECIFIC human-readable error on failure: a distinct message for "no fenced
/// block" vs a serde structure/parse error. This is the single failure classifier
/// the bounded repair turn re-prompts on (so the model is told exactly what to fix).
fn extract_and_parse(prose: &str) -> Result<Slice, String> {
    extract_and_parse_as::<Slice>(prose)
}

/// Generic variant of `extract_and_parse` (G5): extract the fenced json block and
/// deserialize it into any target type `T`, with the same SPECIFIC classifier
/// messages the bounded repair turn re-prompts on. The kickoff one-shot parses a
/// `KickoffSlice` through this; the per-step turns parse the tagged `Slice`.
fn extract_and_parse_as<T: serde::de::DeserializeOwned>(prose: &str) -> Result<T, String> {
    let block = extract_json_block(prose)
        .ok_or_else(|| "no fenced ```json block was found in the reply".to_string())?;
    serde_json::from_str::<T>(&block)
        .map_err(|e| format!("the fenced json did not match the slice schema: {e}"))
}

/// Bounded **repair turn** budget: up to this many repair re-prompts AFTER the
/// initial turn (so at most `1 + MAX_REPAIR_RETRIES` model calls per emission).
/// On exhausting it we behave as before DS-Schema — draft unchanged, prose surfaced.
const MAX_REPAIR_RETRIES: usize = 2;

/// Build a repair re-prompt naming the specific extract-or-parse failure. Sent on
/// the SAME `dialogue_id` so the model sees its own prior bad output in context.
fn repair_user_message(err: &str) -> String {
    format!(
        "Your previous reply could not be applied: {err}. Re-emit ONLY a single fenced \
```json block that matches the JSON Schema in your instructions (include the correct \
\"kind\"). Do not add any other text inside the fence."
    )
}

/// Run one chat turn, then — on an extract-or-parse miss — up to `MAX_REPAIR_RETRIES`
/// bounded **repair turns** on the SAME `dialogue_id`, each naming the specific
/// failure. Returns the final reply prose plus the parsed `Slice` if any turn
/// produced a valid one (else `None` => the caller leaves the draft unchanged). A
/// runner error (e.g. rate-limit) short-circuits with the error prose and no slice.
async fn chat_with_repair(
    runner: &dyn ChatRunner,
    dialogue_id: &str,
    system_prompt: &str,
    initial_user_message: String,
) -> (String, Option<Slice>) {
    chat_with_repair_parsed(runner, dialogue_id, system_prompt, initial_user_message, extract_and_parse).await
}

/// Generic bounded-repair chat (G5): identical discipline to `chat_with_repair`
/// (≤ `1 + MAX_REPAIR_RETRIES` model calls on the SAME `dialogue_id`, repair turns
/// named by the specific failure, runner-error short-circuit), parameterised by a
/// `parse` closure so the kickoff one-shot can extract a `KickoffSlice` instead of
/// the tagged `Slice` WITHOUT forking the repair loop.
async fn chat_with_repair_parsed<T>(
    runner: &dyn ChatRunner,
    dialogue_id: &str,
    system_prompt: &str,
    initial_user_message: String,
    parse: impl Fn(&str) -> Result<T, String>,
) -> (String, Option<T>) {
    let mut user_message = initial_user_message;
    let mut last_text = String::new();
    for attempt in 0..=MAX_REPAIR_RETRIES {
        let req = ChatRequest {
            dialogue_id: dialogue_id.to_string(),
            system_prompt: system_prompt.to_string(),
            user_message,
            model: "claude-opus-4-8".to_string(),
            thinking_budget: 8192,
            working_dir: None,
        };
        match runner.chat(&req).await {
            Ok(reply) => {
                last_text = reply.text.clone();
                match parse(&reply.text) {
                    Ok(value) => return (reply.text, Some(value)),
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

/// The wizard steps that carry a chat (2–4). Step 1 is basics (no chat) and
/// step 5 is review (no chat); the kickoff one-shot is its own call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Step {
    Teams,
    Prompts,
    Wiring,
}

impl Step {
    /// The dialogue-id suffix for this step (Decision D8: one dialogue per step).
    fn slug(self) -> &'static str {
        match self {
            Step::Teams => "teams",
            Step::Prompts => "prompts",
            Step::Wiring => "wiring",
        }
    }
}

/// Build the user message for a turn: the user's words plus the current draft as
/// JSON, so manual edits the user made (the other half of the two-way binding,
/// Decision D8) are visible to the model.
fn turn_user_message(user_message: &str, draft: &DraftPipeline) -> String {
    let draft_json = serde_json::to_string(draft).unwrap_or_default();
    format!("{user_message}\n\nCurrent draft (JSON):\n{draft_json}")
}

/// The result of one Design Session turn: the assistant's prose, the draft after
/// applying any extracted slice (unchanged if none/invalid), and the live
/// best-effort validation issues for that draft (W1 — surfaced inline; never
/// blocks). Issues mirror `draft::best_effort_validate`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnResult {
    pub reply_text: String,
    pub updated_draft: DraftPipeline,
    pub issues: Vec<String>,
}

/// One-shot kickoff (Decision D7): generate a full team set from the description.
/// Stamps a non-empty id + carries the description. Best-effort applies the teams
/// slice. Never errors — a bad reply just yields a draft with no teams (the
/// wizard can retry). dialogue_id is `<session_id>:kickoff`.
pub async fn kickoff_generate(
    runner: &dyn ChatRunner,
    session_id: &str,
    description: &str,
) -> DraftPipeline {
    let mut draft = DraftPipeline::empty();
    draft.description = description.to_string();
    draft.id = slug_id(description);

    let dialogue_id = format!("{session_id}:kickoff");
    // On a runner that supports native structured output (the anthropic-api path),
    // force the kickoff tool whose input_schema is the derived KickoffSlice schema
    // — a schema-valid slice with no prose extraction or repair. Else the CLI
    // prose+parse+repair path (unchanged). Same (_text, slice) contract either way.
    let (_text, slice) = if runner.supports_structured() {
        emit_structured::<KickoffSlice>(
            runner,
            &dialogue_id,
            &kickoff_system_prompt(),
            description.to_string(),
            "Emit the COMPLETE recommended pipeline graph (teams, gates, escalations).",
        )
        .await
    } else {
        chat_with_repair_parsed(
            runner,
            &dialogue_id,
            &kickoff_system_prompt(),
            description.to_string(),
            extract_and_parse_as::<KickoffSlice>,
        )
        .await
    };
    if let Some(slice) = slice {
        apply_kickoff_slice(&mut draft, slice);
    }
    draft
}

/// One Design Session turn (Decision D2/D4). Sends the user's words + the current
/// draft; extracts + applies the fenced slice (unchanged if none/invalid);
/// returns the prose + updated draft. best_effort_validate is run for its side of
/// surfacing issues to the caller via logging in v1 (the returned draft already
/// reflects only valid mutations).
pub async fn design_session_turn(
    runner: &dyn ChatRunner,
    session_id: &str,
    step: Step,
    mut draft: DraftPipeline,
    user_message: &str,
) -> TurnResult {
    let dialogue_id = format!("{session_id}:{}", step.slug());
    // Structured path on the API runner: force the emit tool whose input_schema is
    // the derived Slice schema; the returned args ARE the tagged slice — no
    // extract_and_parse, no repair. Else the CLI prose+parse+repair path
    // (unchanged). The seam is the ChatRunner trait — no anthropic idiom here.
    let (reply_text, slice) = if runner.supports_structured() {
        emit_structured::<Slice>(
            runner,
            &dialogue_id,
            &step_system_prompt(step),
            turn_user_message(user_message, &draft),
            "Emit the pipeline slice (the structured mutation for this step).",
        )
        .await
    } else {
        chat_with_repair(
            runner,
            &dialogue_id,
            &step_system_prompt(step),
            turn_user_message(user_message, &draft),
        )
        .await
    };
    if let Some(slice) = slice {
        apply_slice(&mut draft, slice);
    }
    // best-effort issues are surfaced inline in the wizard (W1); never block.
    let issues = best_effort_validate(&draft);
    TurnResult { reply_text, updated_draft: draft, issues }
}

/// Slugify a description into a pipeline id; fall back to a fresh id when blank.
fn slug_id(description: &str) -> String {
    let slug: String = description
        .chars()
        .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .take(4)
        .collect::<Vec<_>>()
        .join("-");
    if slug.is_empty() {
        format!("pipeline-{}", &uuid::Uuid::new_v4().to_string()[..8])
    } else {
        slug
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_a_fenced_json_block() {
        let prose = "Here are the teams I suggest:\n\n```json\n{\"kind\":\"teams\",\"teams\":[]}\n```\n\nLet me know.";
        let block = extract_json_block(prose).unwrap();
        assert!(block.contains("\"kind\":\"teams\""));
    }

    #[test]
    fn extracts_a_bare_fenced_block_when_no_json_tag() {
        let prose = "ok\n```\n{\"kind\":\"prompt\",\"team_id\":\"a\",\"prompt_body\":\"x\"}\n```";
        let block = extract_json_block(prose).unwrap();
        assert!(block.contains("\"kind\":\"prompt\""));
    }

    #[test]
    fn returns_none_when_no_fence() {
        assert!(extract_json_block("just prose, no block").is_none());
    }

    #[test]
    fn parses_an_extracted_teams_slice() {
        let prose = "```json\n{\"kind\":\"teams\",\"teams\":[{\"id\":\"research\",\"name\":\"Research\"}]}\n```";
        let block = extract_json_block(prose).unwrap();
        let slice = parse_slice(&block).unwrap();
        match slice {
            crate::draft::Slice::Teams(s) => assert_eq!(s.teams[0].id, "research"),
            _ => panic!("expected a teams slice"),
        }
    }

    #[test]
    fn parse_slice_errors_on_garbage() {
        assert!(parse_slice("{not json").is_err());
    }

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
        assert!(matches!(slice, crate::draft::Slice::Teams(_)));
    }

    use crate::draft::{DraftPipeline, DraftTeam};
    use llm_chat::chat::{ChatReply, ChatUsage, StructuredReply};
    use llm_chat::fake::{FakeChatRunner, FakeStructuredChatRunner};

    fn reply(text: &str) -> ChatReply {
        ChatReply { text: text.into(), usage: ChatUsage::default() }
    }

    /// A canned kickoff reply: prose + a fenced KickoffSlice with two producers,
    /// one reviewer (approve→gate, revise→writers, decline→needs-human), a gate,
    /// and the terminal needs-human escalation. Exercises G5 (prompt bodies) + G3
    /// (roles + routes + gate) in one Generate.
    fn canned_kickoff() -> &'static str {
        "I propose a research → write → review flow with a sign-off gate.\n\n\
```json\n{\
\"teams\":[\
{\"id\":\"research\",\"name\":\"Research\",\"prompt_body\":\"You investigate the target repo and write findings.\",\"role\":\"producer\",\"on_approve\":\"writers\"},\
{\"id\":\"writers\",\"name\":\"Writers\",\"prompt_body\":\"You turn findings into a spec.\",\"role\":\"producer\",\"on_approve\":\"reviewers\"},\
{\"id\":\"reviewers\",\"name\":\"Reviewers\",\"prompt_body\":\"You review the spec and judge it.\",\"role\":\"reviewer\",\"on_approve\":\"gate-1\",\"on_revise\":\"writers\",\"on_reject\":\"needs-human\"}\
],\
\"gates\":[{\"id\":\"gate-1\",\"label\":\"Sign-off\",\"downstream\":\"needs-human\"}],\
\"escalations\":[{\"id\":\"needs-human\",\"triggers\":[]}]\
}\n```"
    }

    #[tokio::test]
    async fn kickoff_generate_builds_a_draft_from_a_kickoff_slice() {
        let runner = FakeChatRunner::new(vec![reply(canned_kickoff())]);
        let draft = kickoff_generate(&runner, "sess-1", "Build a research+writing pipeline").await;
        assert_eq!(draft.teams.len(), 3);
        assert!(draft.teams.iter().any(|t| t.id == "research"));
        // the description is carried onto the draft
        assert_eq!(draft.description, "Build a research+writing pipeline");
        // a non-empty id is stamped so the yaml has a basename (D7)
        assert!(!draft.id.is_empty());
    }

    #[tokio::test]
    async fn kickoff_generate_prefills_every_team_with_a_non_empty_prompt_body() {
        // G5 (Task 2): the kickoff one-shot emits prompt bodies (teams + prompts in
        // ONE Generate), so the canvas / NodeDrawer is prefilled — no separate step.
        let runner = FakeChatRunner::new(vec![reply(canned_kickoff())]);
        let draft = kickoff_generate(&runner, "sess-1", "research+writing pipeline").await;
        assert!(!draft.teams.is_empty());
        for t in &draft.teams {
            assert!(!t.prompt_body.trim().is_empty(), "team {} has no prompt body", t.id);
        }
    }

    #[tokio::test]
    async fn design_session_turn_applies_a_prompt_slice_and_returns_prose() {
        let mut draft = DraftPipeline::empty();
        draft.teams.push(DraftTeam::new("research", "Research"));
        let canned = "Set the research prompt.\n\n```json\n{\"kind\":\"prompt\",\"team_id\":\"research\",\
            \"prompt_body\":\"You investigate the repo and write findings.\"}\n```";
        let runner = FakeChatRunner::new(vec![reply(canned)]);
        let out = design_session_turn(&runner, "sess-1", Step::Prompts, draft, "make research investigate").await;
        assert_eq!(out.updated_draft.teams[0].prompt_body, "You investigate the repo and write findings.");
        // the prose (minus the JSON internals) is surfaced
        assert!(out.reply_text.contains("Set the research prompt."));
    }

    #[tokio::test]
    async fn design_session_turn_with_invalid_json_leaves_the_draft_unchanged() {
        let mut draft = DraftPipeline::empty();
        draft.teams.push(DraftTeam::new("research", "Research"));
        let before = draft.clone();
        // prose with NO fenced block at all
        let runner = FakeChatRunner::new(vec![reply("I can't do that — please clarify the team.")]);
        let out = design_session_turn(&runner, "sess-1", Step::Prompts, draft, "garble").await;
        assert_eq!(out.updated_draft, before); // unchanged
        assert!(out.reply_text.contains("clarify"));
    }

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
        let received = runner.received.lock().unwrap();
        assert_eq!(received.len(), 2);
        // the repair request named the failure + re-prompted on the SAME dialogue id
        assert_eq!(received[1].dialogue_id, "sess-1:prompts");
        assert!(received[1].user_message.to_lowercase().contains("json"));
    }

    #[tokio::test]
    async fn turn_gives_up_after_two_repair_retries_and_leaves_the_draft_unchanged() {
        let mut draft = DraftPipeline::empty();
        draft.teams.push(DraftTeam::new("research", "Research"));
        let before = draft.clone();
        // every reply is malformed (no fence). FakeChatRunner clamps to the last
        // reply, so all three calls return malformed.
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

    #[tokio::test]
    async fn kickoff_repairs_a_malformed_first_reply_then_builds_the_draft() {
        let malformed = reply("Two teams: research and writers. (json omitted)");
        let good = reply(canned_kickoff());
        let runner = FakeChatRunner::new(vec![malformed, good]);
        let draft = kickoff_generate(&runner, "sess-1", "research+writing pipeline").await;
        assert_eq!(draft.teams.len(), 3);
        let received = runner.received.lock().unwrap();
        assert_eq!(received.len(), 2); // initial + 1 repair
        assert_eq!(received[1].dialogue_id, "sess-1:kickoff");
    }

    #[tokio::test]
    async fn design_session_turn_passes_the_step_scoped_dialogue_id() {
        let draft = DraftPipeline::empty();
        let runner = FakeChatRunner::new(vec![reply("ok, no changes.")]);
        let _ = design_session_turn(&runner, "sess-9", Step::Teams, draft, "hi").await;
        let received = runner.received.lock().unwrap();
        assert_eq!(received[0].dialogue_id, "sess-9:teams");
    }

    #[tokio::test]
    async fn turn_returns_best_effort_issues_for_the_resulting_draft() {
        // a draft with one team and NO prompt -> best_effort flags the missing prompt
        let mut draft = DraftPipeline::empty();
        draft.teams.push(DraftTeam::new("research", "Research"));
        // a reply with no fenced block -> draft unchanged, still missing the prompt
        let runner = FakeChatRunner::new(vec![reply("noted, nothing to change.")]);
        let out = design_session_turn(&runner, "sess-1", Step::Prompts, draft, "hi").await;
        assert!(out.issues.iter().any(|i| i.contains("research") && i.contains("prompt")));
    }

    #[tokio::test]
    async fn turn_returns_empty_issues_for_a_complete_draft() {
        let mut draft = DraftPipeline::empty();
        let mut a = DraftTeam::new("research", "Research");
        a.prompt_body = "investigate".into();
        a.outputs.on_approve = Some("writers".into());
        let mut b = DraftTeam::new("writers", "Writers");
        b.prompt_body = "write".into();
        draft.teams.push(a);
        draft.teams.push(b);
        let runner = FakeChatRunner::new(vec![reply("looks good.")]);
        let out = design_session_turn(&runner, "sess-1", Step::Wiring, draft, "ok").await;
        assert!(out.issues.is_empty());
    }

    #[tokio::test]
    async fn wiring_turn_applies_a_gate_from_the_slice() {
        let mut draft = DraftPipeline::empty();
        draft.teams.push(DraftTeam::new("plan-writers", "Plan Writers"));
        draft.teams.push(DraftTeam::new("implementers", "Implementers"));
        let canned = "Adding a human review gate.\n\n```json\n{\"kind\":\"wiring\",\
            \"routes\":[{\"team_id\":\"plan-writers\",\"on_approve\":\"gate-2\",\"on_revise\":null,\"on_reject\":null}],\
            \"forks\":[],\"joins\":[],\
            \"gates\":[{\"id\":\"gate-2\",\"label\":\"Plan review\",\"downstream\":\"implementers\"}]}\n```";
        let runner = FakeChatRunner::new(vec![reply(canned)]);
        let out = design_session_turn(&runner, "sess-1", Step::Wiring, draft, "add a review gate").await;
        assert_eq!(out.updated_draft.gates.len(), 1);
        assert_eq!(out.updated_draft.gates[0].downstream, "implementers");
        assert_eq!(out.updated_draft.teams.iter().find(|t| t.id == "plan-writers").unwrap().outputs.on_approve.as_deref(), Some("gate-2"));
    }

    #[test]
    fn wiring_system_prompt_documents_the_gates_schema() {
        let p = step_system_prompt(Step::Wiring);
        assert!(p.contains("gates"));
        assert!(p.contains("downstream"));
    }

    #[test]
    fn step_prompt_embeds_the_derived_slice_schema() {
        // The wiring prompt must contain the schema serialized from WiringSlice via
        // schemars — proving it is GENERATED, not a hand-written literal.
        let p = step_system_prompt(Step::Wiring);
        let schema = serde_json::to_string(&schemars::schema_for!(crate::draft::WiringSlice)).unwrap();
        assert!(p.contains(&schema), "wiring prompt must embed the derived WiringSlice schema");
        // prose rules are kept
        assert!(p.contains("fork") && p.contains(">=2"));
    }

    #[test]
    fn step_prompts_no_longer_carry_a_hand_written_slice_object_literal() {
        // The old hand-written shape used a bare {"kind":"teams","teams":[{"id":...}]}
        // sample-instance literal. Assert the teams prompt now embeds the DERIVED
        // schema for the teams slice (a JSON Schema, not a sample instance).
        let p = step_system_prompt(Step::Teams);
        let schema = serde_json::to_string(&schemars::schema_for!(crate::draft::TeamsSlice)).unwrap();
        assert!(p.contains(&schema));
        // a JSON Schema carries "properties"/"type" metadata; a hand-written sample would not
        assert!(p.contains("properties") || p.contains("\"type\""));
    }

    #[test]
    fn kickoff_prompt_embeds_the_derived_kickoff_schema() {
        // G5: the kickoff prompt embeds the DERIVED KickoffSlice schema (one source
        // of truth) — so prompt↔parser can't drift — and instructs prompt bodies.
        let p = kickoff_system_prompt();
        let schema = serde_json::to_string(&schemars::schema_for!(crate::draft::KickoffSlice)).unwrap();
        assert!(p.contains(&schema), "kickoff prompt must embed the derived KickoffSlice schema");
        // G5: prompt-body instruction present.
        assert!(p.contains("prompt_body"));
    }

    #[test]
    fn kickoff_prompt_instructs_explicit_roles_and_reviewer_routes() {
        // G3 (Task 3): the kickoff prompt names the explicit role + the three
        // reviewer routes + a gate + the needs-human escalation, so a generated
        // graph arrives with a well-formed review structure by construction.
        let p = kickoff_system_prompt();
        assert!(p.contains("role"));
        assert!(p.contains("reviewer"));
        assert!(p.contains("on_revise") && p.contains("on_reject"));
        assert!(p.contains("gate"));
        assert!(p.contains("needs-human"));
    }

    #[tokio::test]
    async fn kickoff_generate_emits_well_formed_review_structure() {
        // G3 (Task 3): a generated draft carries explicit reviewer roles + all
        // three reviewer routes (approve + revise→writer + decline→needs-human) +
        // a gate — so B4's under-connected-reviewer warning is satisfied BY
        // CONSTRUCTION (no per-reviewer "no revise/decline route" issue).
        let runner = FakeChatRunner::new(vec![reply(canned_kickoff())]);
        let draft = kickoff_generate(&runner, "sess-1", "research+writing pipeline").await;

        let reviewer = draft.teams.iter().find(|t| t.id == "reviewers").unwrap();
        // role set EXPLICITLY (not inferred from the name)
        assert_eq!(reviewer.role, crate::model::Role::Reviewer);
        // all three reviewer routes wired
        assert_eq!(reviewer.outputs.on_approve.as_deref(), Some("gate-1"));
        assert_eq!(reviewer.outputs.on_revise.as_deref(), Some("writers")); // back to its writer
        assert_eq!(reviewer.outputs.on_reject.as_deref(), Some("needs-human")); // decline → escalation
        // a human-review gate + a needs-human escalation arrived with the graph
        assert!(draft.gates.iter().any(|g| g.id == "gate-1"));
        assert!(draft.escalations.iter().any(|e| e.id == "needs-human"));

        // B4's under-connected-reviewer signal is satisfied BY CONSTRUCTION: EVERY
        // reviewer-role team has both a revise AND a decline route (the exact
        // condition the canvas warning checks — see draftFlow `teamWarnings`).
        for t in draft.teams.iter().filter(|t| t.role == crate::model::Role::Reviewer) {
            assert!(t.outputs.on_revise.is_some(), "reviewer {} missing revise route", t.id);
            assert!(t.outputs.on_reject.is_some(), "reviewer {} missing decline route", t.id);
        }
        // best_effort_validate (semantic, non-blocking) is also clean for the draft.
        assert_eq!(best_effort_validate(&draft), Vec::<String>::new());
    }

    #[test]
    fn apply_kickoff_slice_maps_roles_routes_and_prompt_bodies() {
        use crate::draft::{apply_kickoff_slice, DraftPipeline, KickoffSlice, KickoffTeam};
        let mut d = DraftPipeline::empty();
        d.id = "keep".into();
        let slice = KickoffSlice {
            teams: vec![KickoffTeam {
                id: "rev".into(),
                name: "Reviewer".into(),
                prompt_body: "You judge work.".into(),
                role: crate::model::Role::Reviewer,
                on_approve: Some("done".into()),
                on_revise: Some("writers".into()),
                on_reject: Some("needs-human".into()),
            }],
            gates: vec![],
            escalations: vec![crate::model::Escalation { id: "needs-human".into(), triggers: vec![] }],
        };
        apply_kickoff_slice(&mut d, slice);
        assert_eq!(d.id, "keep"); // identity preserved
        let t = &d.teams[0];
        assert_eq!(t.prompt_body, "You judge work.");
        assert_eq!(t.role, crate::model::Role::Reviewer);
        assert_eq!(t.outputs.on_revise.as_deref(), Some("writers"));
        assert_eq!(d.escalations.len(), 1);
    }

    // ---- Structured (native tool-use) path on the API runner (T2 Task 3) ----

    fn structured(args: serde_json::Value) -> StructuredReply {
        StructuredReply { tool_name: "emit_slice".into(), args, usage: ChatUsage::default() }
    }

    #[tokio::test]
    async fn design_session_turn_uses_structured_emit_on_a_structured_runner() {
        // The structured runner returns the slice DIRECTLY (no prose, no repair).
        let mut draft = DraftPipeline::empty();
        draft.teams.push(DraftTeam::new("research", "Research"));
        let args = serde_json::json!({
            "kind": "prompt",
            "team_id": "research",
            "prompt_body": "You investigate the repo and write findings."
        });
        let runner = FakeStructuredChatRunner::new(vec![structured(args)]);
        let out = design_session_turn(&runner, "sess-1", Step::Prompts, draft, "set research prompt").await;
        assert_eq!(out.updated_draft.teams[0].prompt_body, "You investigate the repo and write findings.");
        // exactly ONE call — no repair turn on the forced-schema path
        assert_eq!(runner.received.lock().unwrap().len(), 1);
        // the forced tool was the slice-emit tool with the derived schema
        let calls = runner.structured_calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, vec!["emit_slice".to_string()]);
        assert_eq!(calls[0].1.as_deref(), Some("emit_slice"));
    }

    #[tokio::test]
    async fn cli_runner_still_uses_prose_parse_repair_path() {
        // A non-structured (CLI) fake: the existing prose+parse+repair path runs
        // UNCHANGED — first reply has no fence (repair), second is the valid slice.
        let mut draft = DraftPipeline::empty();
        draft.teams.push(DraftTeam::new("research", "Research"));
        let malformed = reply("I think research should investigate. (forgot the json)");
        let good = reply("Here:\n```json\n{\"kind\":\"prompt\",\"team_id\":\"research\",\
            \"prompt_body\":\"You investigate the repo and write findings.\"}\n```");
        let runner = FakeChatRunner::new(vec![malformed, good]);
        assert!(!runner.supports_structured());
        let out = design_session_turn(&runner, "sess-1", Step::Prompts, draft, "set research prompt").await;
        assert_eq!(out.updated_draft.teams[0].prompt_body, "You investigate the repo and write findings.");
        // the prose path made 2 calls (initial + repair) — proof it took the CLI branch
        assert_eq!(runner.received.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn kickoff_uses_structured_emit_on_a_structured_runner() {
        let args = serde_json::json!({
            "teams": [
                {"id":"research","name":"Research","prompt_body":"You investigate.","role":"producer","on_approve":"writers"},
                {"id":"writers","name":"Writers","prompt_body":"You write.","role":"producer","on_approve":"reviewers"},
                {"id":"reviewers","name":"Reviewers","prompt_body":"You review.","role":"reviewer","on_approve":"gate-1","on_revise":"writers","on_reject":"needs-human"}
            ],
            "gates": [{"id":"gate-1","label":"Sign-off","downstream":"needs-human"}],
            "escalations": [{"id":"needs-human","triggers":[]}]
        });
        let runner = FakeStructuredChatRunner::new(vec![structured(args)]);
        let draft = kickoff_generate(&runner, "sess-1", "research+writing pipeline").await;
        assert_eq!(draft.teams.len(), 3);
        // ONE structured call, forced on the emit tool
        let calls = runner.structured_calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].1.as_deref(), Some("emit_slice"));
    }

    #[tokio::test]
    async fn structured_emit_with_unparseable_args_leaves_draft_unchanged() {
        // A forced reply whose args don't match the slice shape => draft unchanged
        // (the same outcome as the prose path's parse miss).
        let mut draft = DraftPipeline::empty();
        draft.teams.push(DraftTeam::new("research", "Research"));
        let before = draft.clone();
        let runner = FakeStructuredChatRunner::new(vec![structured(serde_json::json!({"nope": true}))]);
        let out = design_session_turn(&runner, "sess-1", Step::Prompts, draft, "go").await;
        assert_eq!(out.updated_draft, before);
    }

    #[test]
    fn slice_schema_value_matches_the_string_schema() {
        // One source of truth: the Value schema equals the parsed String schema.
        let s: serde_json::Value =
            serde_json::from_str(&slice_schema::<crate::draft::TeamsSlice>()).unwrap();
        let v = slice_schema_value::<crate::draft::TeamsSlice>();
        assert_eq!(s, v);
    }
}
