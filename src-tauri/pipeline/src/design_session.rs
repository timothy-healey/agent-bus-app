//! Design Session — the ephemeral, AI-assisted authoring dialogue that produces a
//! pipeline (DOMAIN.md → Pipeline Authoring). Distinct from the terminal's
//! `Conversation` aggregate; conducted over the llm_chat ACL. This module holds
//! the kickoff one-shot + the per-step turn logic and the structured-emit
//! contract: the model returns PROSE plus a fenced ```json slice; the backend
//! extracts + parses + best-effort-applies the slice (the trust boundary). The
//! prose is never parsed for state.

use crate::draft::{apply_slice, best_effort_validate, DraftPipeline, PromptSlice, Slice, TeamsSlice, WiringSlice};
use llm_chat::chat::{ChatRequest, ChatRunner};
use serde::{Deserialize, Serialize};

/// Compact JSON Schema for a slice type, derived from the Rust type via schemars.
/// This is the single source of truth (the **slice schema**): the prompt and
/// `parse_slice` are generated from the SAME types, so they can never drift.
fn slice_schema<T: schemars::JsonSchema>() -> String {
    serde_json::to_string(&schemars::schema_for!(T)).unwrap_or_default()
}

/// The kickoff one-shot system prompt: prose + a fenced ```json TEAMS slice whose
/// shape is the DERIVED slice schema (one source of truth). Prose rules kept
/// (2–5 teams, slug ids).
pub fn kickoff_system_prompt() -> String {
    format!(
        "You are designing a multi-team Claude Code agent pipeline from a one-line \
description. Reply with a short paragraph of prose, THEN a fenced ```json block \
containing ONLY the team set. The object MUST include \"kind\":\"teams\" and match \
this JSON Schema (the team-set payload):\n\
```json\n{schema}\n```\n\
Use 2 to 5 teams. ids are lowercase slugs. Emit ONLY the json in the fenced block.",
        schema = slice_schema::<TeamsSlice>()
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
    let block = extract_json_block(prose)
        .ok_or_else(|| "no fenced ```json block was found in the reply".to_string())?;
    parse_slice(&block).map_err(|e| format!("the fenced json did not match the slice schema: {e}"))
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
    let mut user_message = initial_user_message;
    let mut last_text = String::new();
    for attempt in 0..=MAX_REPAIR_RETRIES {
        let req = ChatRequest {
            dialogue_id: dialogue_id.to_string(),
            system_prompt: system_prompt.to_string(),
            user_message,
            model: "claude-opus-4-8".to_string(),
            thinking_budget: 8192,
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
    let (_text, slice) = chat_with_repair(
        runner,
        &dialogue_id,
        &kickoff_system_prompt(),
        description.to_string(),
    )
    .await;
    if let Some(slice) = slice {
        apply_slice(&mut draft, slice);
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
    let (reply_text, slice) = chat_with_repair(
        runner,
        &dialogue_id,
        &step_system_prompt(step),
        turn_user_message(user_message, &draft),
    )
    .await;
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
    use llm_chat::chat::{ChatReply, ChatUsage};
    use llm_chat::fake::FakeChatRunner;

    fn reply(text: &str) -> ChatReply {
        ChatReply { text: text.into(), usage: ChatUsage::default() }
    }

    #[tokio::test]
    async fn kickoff_generate_builds_a_draft_from_a_teams_slice() {
        let canned = "I propose a two-team flow.\n\n```json\n{\"kind\":\"teams\",\"teams\":[\
            {\"id\":\"research\",\"name\":\"Research\"},{\"id\":\"writers\",\"name\":\"Writers\"}]}\n```";
        let runner = FakeChatRunner::new(vec![reply(canned)]);
        let draft = kickoff_generate(&runner, "sess-1", "Build a research+writing pipeline").await;
        assert_eq!(draft.teams.len(), 2);
        assert!(draft.teams.iter().any(|t| t.id == "research"));
        // the description is carried onto the draft
        assert_eq!(draft.description, "Build a research+writing pipeline");
        // a non-empty id is stamped so the yaml has a basename (D7)
        assert!(!draft.id.is_empty());
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
        let good = reply("```json\n{\"kind\":\"teams\",\"teams\":[\
            {\"id\":\"research\",\"name\":\"Research\"},{\"id\":\"writers\",\"name\":\"Writers\"}]}\n```");
        let runner = FakeChatRunner::new(vec![malformed, good]);
        let draft = kickoff_generate(&runner, "sess-1", "research+writing pipeline").await;
        assert_eq!(draft.teams.len(), 2);
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
    fn kickoff_prompt_embeds_the_derived_teams_schema() {
        let p = kickoff_system_prompt();
        let schema = serde_json::to_string(&schemars::schema_for!(crate::draft::TeamsSlice)).unwrap();
        assert!(p.contains(&schema));
        assert!(p.contains('2') && p.contains('5')); // 2–5 teams rule kept
    }
}
