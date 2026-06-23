use std::sync::Arc;
use tauri::{Emitter, Manager};
use tauri_plugin_sql::{Migration, MigrationKind};
use workspace::{api::WorkspaceState, store::ProjectStore};

use runners::anthropic_api::AnthropicApiRunner;
use runners::claude_cli::ClaudeCliRunner;
use runners::output::{Runner, RunnerError};
use runtime::api::RuntimeState;
use runtime::brake::Brake;
use runtime::pool::{process_one_claim, PoolContext};
use runtime::task_store::TaskStore;
use pipeline::model::{Pipeline, Team};
use pipeline::draft::DraftPipeline;
use pipeline::design_session::{design_session_turn, kickoff_generate, Step, TurnResult};
use workspace::project::Project;

use conversational_control::catalog::ToolCatalog;
use conversational_control::dispatch::ToolDispatcher;
use conversational_control::engine::CommandEngine;
use conversational_control::engine::{ConversationEngine, EngineReply};
use conversational_control::store::ConversationStore;
use conversational_control::turn::ToolCall;
use llm_chat::chat::{ChatRequest, ChatRunner, DeltaSink};
use conversational_control::api::TerminalState;
use agent_bus_core::{ToolCallRequest, ToolCallResult};
use async_trait::async_trait;

const DB_URL: &str = "sqlite:agent_bus.db";

/// Apply schema migrations to the app's own pool, in order, idempotently.
///
/// The applied version lives in SQLite's `PRAGMA user_version` rather than in
/// tauri-plugin-sql's tracking (whose migrations only run when the *frontend*
/// loads the plugin — which this app never does). `raw_sql` runs the
/// multi-statement migration files; migration 004 is a non-idempotent `ALTER`,
/// so the version gate is what keeps re-runs safe.
async fn run_migrations(pool: &sqlx::SqlitePool) -> Result<(), sqlx::Error> {
    const MIGRATIONS: &[(i64, &str)] = &[
        (1, include_str!("../migrations/001_initial.sql")),
        (2, include_str!("../migrations/002_pipeline_activation.sql")),
        (3, include_str!("../migrations/003_runtime.sql")),
        (4, include_str!("../migrations/004_comments_kind.sql")),
        (5, include_str!("../migrations/005_usage.sql")),
        (6, include_str!("../migrations/006_fanout.sql")),
        (7, include_str!("../migrations/007_invocation_audit.sql")),
        (8, include_str!("../migrations/008_nested_groups.sql")),
        (9, include_str!("../migrations/009_git_config.sql")),
    ];

    let current: i64 = sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(pool)
        .await?;

    for (version, sql) in MIGRATIONS {
        if *version > current {
            sqlx::raw_sql(sql).execute(pool).await?;
            // PRAGMA can't bind parameters; the value is our own trusted i64.
            sqlx::raw_sql(&format!("PRAGMA user_version = {version};"))
                .execute(pool)
                .await?;
        }
    }
    Ok(())
}

/// The concrete dispatcher. Lives at the root — the only module that imports
/// every context (D1/D2). Routes a ToolCallRequest to the owning supplier's
/// logic, reusing the same stores the Tauri commands use.
struct RootDispatcher {
    runtime: Arc<RuntimeState>,
    usage: Arc<usage_telemetry::api::UsageState>,
    app: tauri::AppHandle,
}

/// Concrete revise-bundle reader (Plan 4 vet F1 consumer). Reads the comments
/// table written by Review and flattens rows into Runtime's RevisionNote. Lives
/// at the root because it bridges Runtime's trait + Review's schema without
/// either crate depending on the other.
pub struct SqliteRevisionReader {
    pub pool: sqlx::SqlitePool,
}

#[async_trait]
impl runtime::revision::RevisionBundleReader for SqliteRevisionReader {
    async fn load(&self, task_id: &str) -> runtime::revision::RevisionBundle {
        let rows: Vec<(Option<String>, String, String)> = sqlx::query_as(
            "SELECT anchor_text, note, kind FROM comments WHERE task_id = ? \
             ORDER BY created_at ASC, anchor_offset ASC",
        )
        .bind(task_id)
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default();
        runtime::revision::RevisionBundle {
            notes: rows
                .into_iter()
                .map(|(anchor_text, note, kind)| runtime::revision::RevisionNote {
                    anchor_text,
                    note,
                    kind,
                })
                .collect(),
        }
    }
}

fn ok(v: serde_json::Value) -> ToolCallResult { ToolCallResult::Ok { result: v } }
fn err(e: impl ToString) -> ToolCallResult { ToolCallResult::Err { error: e.to_string() } }

/// Couples the display-only prose sink with an explicit step-boundary `reset`, so
/// the `DeltaSink` stays prose-only (vet F1) — no control marker rides the prose
/// channel. The engine calls `reset()` at the top of each model step and passes
/// `&sink` (prose fragments only) to `chat_stream`.
pub struct ConversationDeltaEmitter {
    pub sink: DeltaSink,
    pub reset: Box<dyn Fn() + Send + Sync>,
}

/// Build a display-only ConversationDeltaEmitter that emits throttled
/// `conversation.delta` Tauri events. The prose `sink` coalesces fragments and
/// flushes every ~50ms or when the buffer reaches ~80 chars. The separate `reset`
/// flushes any buffered prose then emits a `{reset:true}` boundary (a new model
/// step started) — vet F1: the prose channel stays prose-only. Display-only:
/// payload is prose + a reset flag; no stream-json idiom crosses here.
fn make_conversation_delta_sink(handle: tauri::AppHandle) -> ConversationDeltaEmitter {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    struct Buf { text: String, last: Instant }
    let state = Arc::new(Mutex::new(Buf { text: String::new(), last: Instant::now() }));

    let sink_state = state.clone();
    let sink_handle = handle.clone();
    let sink: DeltaSink = Box::new(move |frag: &str| {
        let mut b = sink_state.lock().unwrap();
        b.text.push_str(frag);
        let due = b.last.elapsed() >= Duration::from_millis(50) || b.text.len() >= 80;
        if due {
            let _ = sink_handle.emit("conversation.delta", serde_json::json!({ "text": b.text, "reset": false }));
            b.text.clear();
            b.last = Instant::now();
        }
    });

    let reset: Box<dyn Fn() + Send + Sync> = Box::new(move || {
        let mut b = state.lock().unwrap();
        if !b.text.is_empty() {
            let _ = handle.emit("conversation.delta", serde_json::json!({ "text": b.text, "reset": false }));
            b.text.clear();
        }
        let _ = handle.emit("conversation.delta", serde_json::json!({ "text": "", "reset": true }));
        b.last = Instant::now();
    });

    ConversationDeltaEmitter { sink, reset }
}

/// Coalescing buffer for one task's live-log fragments. Pure (no Tauri) so the
/// flush/emit shape is unit-tested. `push` accumulates; `flush_if_due` and
/// `force_flush` invoke the emit callback with `(task_id, combined_delta)` and
/// clear the buffer. Mirrors the conversation.delta coalescing (R4 / C2 F1).
pub struct TaskLogBuffer {
    task_id: String,
    text: String,
    last: std::time::Instant,
}

impl TaskLogBuffer {
    pub fn new(task_id: String) -> Self {
        Self { task_id, text: String::new(), last: std::time::Instant::now() }
    }

    pub fn push(&mut self, frag: &str) {
        self.text.push_str(frag);
    }

    /// Flush when ~50ms elapsed or the buffer reached ~80 chars (same thresholds
    /// as the conversation.delta sink), emitting (task_id, delta) and clearing.
    pub fn flush_if_due(&mut self, emit: &mut dyn FnMut(&str, &str)) {
        let due = self.last.elapsed() >= std::time::Duration::from_millis(50)
            || self.text.len() >= 80;
        if due {
            self.force_flush(emit);
        }
    }

    /// Emit whatever is buffered (if any) and clear; resets the timer.
    pub fn force_flush(&mut self, emit: &mut dyn FnMut(&str, &str)) {
        if !self.text.is_empty() {
            emit(&self.task_id, &self.text);
            self.text.clear();
        }
        self.last = std::time::Instant::now();
    }
}

/// Build a per-task `LogSink` factory that emits throttled `task.log` Tauri
/// events `{ task_id, delta }`. Display-only: the payload is the task id + a
/// prose fragment; no stream-json idiom crosses here. Each task gets its own
/// coalescing buffer so concurrent workers' logs never interleave within a flush.
fn make_task_log_sink(handle: tauri::AppHandle) -> Arc<runtime::pool::LogSinkFactory> {
    Arc::new(move |task_id: &str| -> runners::output::LogSink {
        use std::sync::Mutex;
        let buf = Arc::new(Mutex::new(TaskLogBuffer::new(task_id.to_string())));
        let handle = handle.clone();
        Box::new(move |frag: &str| {
            let mut b = buf.lock().unwrap();
            b.push(frag);
            b.flush_if_due(&mut |tid: &str, delta: &str| {
                let _ = handle.emit(
                    "task.log",
                    serde_json::json!({ "task_id": tid, "delta": delta }),
                );
            });
        })
    })
}

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
    delta: Option<ConversationDeltaEmitter>,
}

impl LlmEngine {
    pub fn new(
        runner: Arc<dyn ChatRunner>,
        dialogue_id: String,
        system_prompt: String,
        model: String,
        thinking_budget: u32,
    ) -> Self {
        Self { runner, dialogue_id, system_prompt, model, thinking_budget, delta: None }
    }

    /// Attach a display-only delta emitter (root emits throttled conversation.delta).
    pub fn with_delta_sink(mut self, delta: Option<ConversationDeltaEmitter>) -> Self {
        self.delta = delta;
        self
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
        // Display-only streaming when a delta emitter is attached; reset clears
        // the live bubble first (prose sink stays prose-only — vet F1).
        let result = match &self.delta {
            Some(emitter) => {
                (emitter.reset)();
                self.runner.chat_stream(&req, &emitter.sink).await
            }
            None => self.runner.chat(&req).await,
        };
        match result {
            Ok(reply) => EngineReply { text: reply.text, tool_calls: vec![] },
            // A chat failure becomes a clear, non-panicking error turn (spec
            // §Error handling). The root's existing rate-limit handling can read
            // the text; the terminal never crashes on a missing `claude`.
            Err(e) => EngineReply { text: format!("[terminal error] {e}"), tool_calls: vec![] },
        }
    }
}

/// Extract the first fenced code block from the model's reply (DD3). Prefers a
/// ```json fence; falls back to the first bare ``` fence. Returns the block's
/// inner text (no fences), or None when there is no fence (= the model is done,
/// the reply is the final prose answer).
///
/// VF1 (documented duplication): this intentionally mirrors
/// `pipeline::design_session::extract_json_block` rather than sharing it, so the
/// agentic loop stays decoupled from the wizard module. Keep the two in sync.
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

/// Parse a model-emitted fenced block `{ "tool": "<name>", "args": { ... } }`
/// directly into the kernel's `ToolCallRequest { tool_name, args }` (VF2 — no
/// shadow `ParsedToolCall` type). The wire field is `tool`; the kernel field is
/// `tool_name`, so we read the JSON object explicitly and construct the canonical
/// type the dispatcher already takes. `args` defaults to `{}` when absent.
/// Returns a descriptive error string (fed back to the model) when the block is
/// not valid JSON or is missing the `tool` field.
pub fn parse_tool_call(block: &str) -> Result<ToolCallRequest, String> {
    let value: serde_json::Value =
        serde_json::from_str(block).map_err(|e| e.to_string())?;
    let tool_name = value
        .get("tool")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing string field `tool`".to_string())?
        .to_string();
    let args = value
        .get("args")
        .cloned()
        .unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new()));
    Ok(ToolCallRequest { tool_name, args })
}

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
    delta: Option<ConversationDeltaEmitter>,
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
            runner,
            dispatcher,
            brake,
            dialogue_id,
            system_prompt_framing,
            model,
            thinking_budget,
            max_steps: MAX_STEPS,
            delta: None,
        }
    }

    /// Attach a display-only delta emitter (root emits throttled conversation.delta).
    pub fn with_delta_sink(mut self, delta: Option<ConversationDeltaEmitter>) -> Self {
        self.delta = delta;
        self
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
            // Display-only streaming: each step resets the live bubble via the
            // explicit reset signal (prose sink stays prose-only — vet F1), then
            // forwards prose fragments. Tool-call extraction below still runs on
            // the COMPLETE reply.text (never partial JSON).
            let reply = match &self.delta {
                Some(emitter) => {
                    (emitter.reset)();
                    self.runner.chat_stream(&req, &emitter.sink).await
                }
                None => self.runner.chat(&req).await,
            };
            let reply = match reply {
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
            // VF2: parse straight into the kernel's ToolCallRequest, no shadow type.
            let request = match parse_tool_call(&block) {
                Err(e) => {
                    next_user_message = format!(
                        "Tool call rejected: that was not a valid tool call ({e}). \
                         Reply with a single fenced ```json {{\"tool\":\"<name>\",\"args\":{{…}}}} block, \
                         or your final answer as plain prose."
                    );
                    continue;
                }
                Ok(req) if catalog.by_name(&req.tool_name).is_none() => {
                    next_user_message = format!(
                        "Tool call rejected: unknown tool `{}`. Available tools: {}. \
                         Reply with a valid fenced ```json tool call, or your final answer.",
                        req.tool_name, tool_names(catalog),
                    );
                    continue;
                }
                Ok(req) => req,
            };

            // DD4: brake before dispatch too.
            if self.brake.is_on() {
                let reason = self.brake.state().reason.unwrap_or_else(|| "on".into());
                return EngineReply { text: format!("[braked] terminal paused: {reason}"), tool_calls };
            }
            let tool_name = request.tool_name.clone();
            let result = self.dispatcher.dispatch(&request).await;
            let result_json = serde_json::to_string(&result).unwrap_or_else(|_| "{}".into());
            tool_calls.push(ToolCall { request, result: Some(result) });
            // DD5: feed the result back as the next user message.
            next_user_message = format!(
                "Tool `{}` returned:\n```json\n{}\n```\nContinue: call another tool \
                 (fenced json) or reply with your final answer.",
                tool_name, result_json,
            );
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

/// Holds the chat runner for the wizard's Design Session (ephemeral; no
/// persistence). The same Arc<dyn ChatRunner> the terminal uses can be shared.
pub struct DesignSessionState {
    pub runner: Arc<dyn ChatRunner>,
}

/// OHS: one-shot kickoff — generate a full DraftPipeline from the description.
#[tauri::command(rename_all = "snake_case")]
async fn kickoff_generate_cmd(
    state: tauri::State<'_, DesignSessionState>,
    session_id: String,
    description: String,
) -> Result<DraftPipeline, String> {
    Ok(kickoff_generate(state.runner.as_ref(), &session_id, &description).await)
}

/// OHS: list the bundled seed templates (id/name/description) for the kickoff
/// picker. Pure pass-through to Pipeline Authoring's registry — no state, no chat.
#[tauri::command(rename_all = "snake_case")]
fn list_seed_templates_cmd() -> Vec<pipeline::seed_template::SeedTemplate> {
    pipeline::seed_template::seed_templates()
}

/// OHS: return a populated `DraftPipeline` seed for a template id. Unknown id is
/// an error. Mirrors `kickoff_generate_cmd`'s shape (→ DraftPipeline); the wizard
/// refines it and creates through the normal hard-validate path.
#[tauri::command(rename_all = "snake_case")]
fn seed_template_cmd(id: String) -> Result<DraftPipeline, String> {
    pipeline::seed_template::seed_template(&id).ok_or_else(|| format!("unknown seed template: {id}"))
}

/// OHS: one Design Session turn — apply a slice + return prose + updated draft.
#[tauri::command(rename_all = "snake_case")]
async fn design_session_turn_cmd(
    state: tauri::State<'_, DesignSessionState>,
    session_id: String,
    step: Step,
    draft: DraftPipeline,
    user_message: String,
) -> Result<TurnResult, String> {
    Ok(design_session_turn(state.runner.as_ref(), &session_id, step, draft, &user_message).await)
}

/// OHS: recompute best-effort validation for a manually-edited draft (W1). The
/// turn command already returns issues; this serves edits that bypass chat. Pure
/// pass-through to pipeline::draft::best_effort_validate — no state, no chat.
#[tauri::command(rename_all = "snake_case")]
fn best_effort_validate_cmd(draft: pipeline::draft::DraftPipeline) -> Vec<String> {
    pipeline::draft::best_effort_validate(&draft)
}

/// Orchestrate create-from-draft (Decision D5; vet F1). HARD validate the draft's
/// Pipeline; only on Ok create the project + write files (Workspace) + activate.
/// Nothing is written when invalid. Inner fn so it is unit-testable without a
/// Tauri State wrapper.
pub async fn create_project_from_draft_inner(
    ws: &workspace::api::WorkspaceState,
    name: String,
    root: String,
    draft: DraftPipeline,
) -> Result<Project, String> {
    // 1. HARD validate + serialize (shared gate; nothing is written when invalid).
    let (yaml_rel, yaml, prompts) = pipeline::draft::prepare_pipeline_write(&draft)?;
    let pipeline = draft.to_pipeline();

    // 2. Create the project row (Workspace; ~ already expanded inside).
    let expanded = workspace::api::expand_tilde(&root, &std::env::var("HOME").unwrap_or_default());
    let project = Project::new(name, std::path::PathBuf::from(expanded), now_unix());
    ws.store.insert(&project).await.map_err(|e| e.to_string())?;

    // 3. Write the YAML + prompt files (Workspace owns the bytes-to-disk).
    workspace::api::write_project_pipeline_inner(
        ws, project.id.0.clone(), yaml_rel, yaml, prompts,
    )
    .await?;

    // 4. Activate.
    ws.store
        .set_active_pipeline(&project.id, Some(&agent_bus_core::PipelineId(pipeline.id.clone())), now_unix())
        .await
        .map_err(|e| e.to_string())?;

    // Return the project with the active pipeline reflected.
    ws.store.get(&project.id).await.map_err(|e| e.to_string())
}

/// OHS command: validate → create + write → activate. The only new-project path.
#[tauri::command(rename_all = "snake_case")]
async fn create_project_from_draft(
    state: tauri::State<'_, WorkspaceState>,
    name: String,
    root: String,
    draft: DraftPipeline,
) -> Result<Project, String> {
    create_project_from_draft_inner(&state, name, root, draft).await
}

/// Read every team's prompt file body via Workspace's escape-guarded path
/// resolution, then convert the resolved Pipeline into an editable DraftPipeline
/// (A1; vet: Workspace owns the file read, Pipeline Authoring owns the convert).
/// Inner fn so it is testable without a Tauri State wrapper.
pub async fn pipeline_to_draft_inner(
    ws: &workspace::api::WorkspaceState,
    project_id: String,
    pipeline: &pipeline::model::Pipeline,
) -> Result<DraftPipeline, String> {
    let project = ws
        .store
        .get(&agent_bus_core::ProjectId(project_id))
        .await
        .map_err(|e| e.to_string())?;
    let root = project.root_path.to_string_lossy().into_owned();
    let mut bodies: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for team in &pipeline.teams {
        let full = workspace::api::resolve_under_root(&root, &team.prompt)?;
        // A missing prompt file is tolerated (empty body) — the editor seeds a
        // blank prompt the operator can fill, rather than failing to open.
        let body = std::fs::read_to_string(&full).unwrap_or_default();
        bodies.insert(team.id.clone(), body);
    }
    Ok(DraftPipeline::from_pipeline(pipeline, &bodies))
}

/// OHS: load the project's pipeline (resolved) into an editable DraftPipeline,
/// reading each team's prompt body back from disk. Seeds the in-app editor (A1).
#[tauri::command(rename_all = "snake_case")]
async fn pipeline_to_draft_cmd(
    state: tauri::State<'_, WorkspaceState>,
    project_id: String,
    project_root: String,
    id: String,
) -> Result<DraftPipeline, String> {
    let pipeline = pipeline::store::PipelineStore::new(project_root)
        .load(&id)
        .map_err(|e| e.to_string())?;
    pipeline_to_draft_inner(&state, project_id, &pipeline).await
}

/// Orchestrate save-pipeline-edits (A1; mirrors create_project_from_draft_inner
/// minus the row insert). HARD validate the draft's Pipeline; only on Ok overwrite
/// the project's existing YAML + prompt files (Workspace owns the write) and keep
/// the pipeline active. Nothing is written when invalid. Inner fn so it is
/// unit-testable without a Tauri State wrapper.
pub async fn save_pipeline_edits_inner(
    ws: &workspace::api::WorkspaceState,
    project_id: String,
    draft: DraftPipeline,
) -> Result<(), String> {
    // 1. HARD validate + serialize (the shared gate; nothing is written when
    //    invalid). vet F2: same core the create flow uses.
    let (yaml_rel, yaml, prompts) = pipeline::draft::prepare_pipeline_write(&draft)?;
    let pipeline_id = draft.to_pipeline().id;

    // 2. Overwrite the YAML + prompt files (Workspace owns bytes-to-disk; the
    //    project row already exists — do NOT insert, do NOT re-expand the root).
    workspace::api::write_project_pipeline_inner(ws, project_id.clone(), yaml_rel, yaml, prompts)
        .await?;

    // 3. Keep the pipeline active (idempotent — typically already active; correct
    //    if the active pointer was cleared).
    ws.store
        .set_active_pipeline(
            &agent_bus_core::ProjectId(project_id),
            Some(&agent_bus_core::PipelineId(pipeline_id)),
            now_unix(),
        )
        .await
        .map_err(|e| e.to_string())
}

/// OHS command: hard-validate → overwrite the active pipeline's YAML + prompts.
#[tauri::command(rename_all = "snake_case")]
async fn save_pipeline_edits(
    state: tauri::State<'_, WorkspaceState>,
    project_id: String,
    draft: DraftPipeline,
) -> Result<(), String> {
    save_pipeline_edits_inner(&state, project_id, draft).await
}

#[async_trait]
impl ToolDispatcher for RootDispatcher {
    async fn dispatch(&self, req: &ToolCallRequest) -> ToolCallResult {
        use tauri::Emitter;
        let a = &req.args;
        let str_arg = |k: &str| a.get(k).and_then(|v| v.as_str()).map(|s| s.to_string());
        let result = match req.tool_name.as_str() {
            "inject_topic" => {
                match runtime::api::inject_topic_inner(
                    &self.runtime, str_arg("topic").unwrap_or_default(), str_arg("target_repo"),
                ).await {
                    Ok(task) => { let _ = self.app.emit("task.changed", &task.id.0); serde_json::to_value(task).map(ok).unwrap_or_else(err) }
                    Err(e) => err(e),
                }
            }
            "approve_gate" | "reject_gate" | "revise_gate" => {
                let task_id = str_arg("task_id").unwrap_or_default();
                let verdict = match req.tool_name.as_str() {
                    "approve_gate" => agent_bus_core::Verdict::Approve,
                    "reject_gate" => agent_bus_core::Verdict::Reject,
                    _ => agent_bus_core::Verdict::Revise,
                };
                match runtime::api::apply_gate_verdict_inner(&self.runtime, &task_id, verdict).await {
                    Ok(task) => { let _ = self.app.emit("task.changed", &task.id.0); serde_json::to_value(task).map(ok).unwrap_or_else(err) }
                    Err(e) => err(e),
                }
            }
            "brake_on" => { let s = self.runtime.brake.clone(); s.set_on(str_arg("reason").unwrap_or_else(|| "manual".into())); let _ = self.app.emit("usage.changed", ()); ok(serde_json::to_value(s.state()).unwrap()) }
            "brake_off" => { self.runtime.brake.set_off(); let _ = self.app.emit("usage.changed", ()); ok(serde_json::to_value(self.runtime.brake.state()).unwrap()) }
            "scale_team" => {
                match runtime::api::scale_team_inner(&self.runtime, str_arg("team_id").unwrap_or_default()) {
                    Ok(maxn) => ok(serde_json::json!({ "max": maxn })),
                    Err(e) => err(e),
                }
            }
            "usage_snapshot" => {
                let cfg = usage_telemetry::api::load_config(&self.usage.pool).await;
                let braked = (self.usage.is_braked)();
                match usage_telemetry::snapshot::compute_snapshot(&self.usage.cc, &self.usage.worker, &cfg, braked, now_unix()).await {
                    Ok(snap) => serde_json::to_value(snap).map(ok).unwrap_or_else(err),
                    Err(e) => err(e),
                }
            }
            "usage_set_auto_meter" => {
                let enabled = a.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false);
                match usage_telemetry::api::set_auto_meter_inner(&self.usage.pool, enabled).await {
                    Ok(()) => { let _ = self.app.emit("usage.changed", ()); ok(serde_json::json!({ "auto_meter_enabled": enabled })) }
                    Err(e) => err(e),
                }
            }
            other => err(format!("tool not dispatchable in v1: {other}")),
        };
        result
    }
}

fn now_unix() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64
}

/// Resolve the active project + pipeline at startup. v1: the newest project
/// (projects[0] in the frontend ordering) and its first pipeline file. Returns
/// an empty placeholder pipeline when none exists so the app still boots.
async fn load_active(
    project_store: &ProjectStore,
) -> (String, String, Pipeline) {
    let empty = Pipeline {
        id: String::new(), name: String::new(), description: String::new(),
        schema_version: pipeline::model::SCHEMA_VERSION,
        defaults: None,
        teams: vec![], gates: vec![], escalations: vec![],
        forks: vec![], joins: vec![],
    };
    let Ok(projects) = project_store.list().await else { return (String::new(), String::new(), empty); };
    let Some(project) = projects.into_iter().next() else { return (String::new(), String::new(), empty); };
    let root = project.root_path.to_string_lossy().into_owned();
    let store = pipeline::store::PipelineStore::new(&root);
    let pipe = store
        .list_ids()
        .ok()
        .and_then(|ids| ids.into_iter().next())
        .and_then(|id| store.load(&id).ok())
        .unwrap_or(empty);
    (project.id.0, root, pipe)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let migrations = vec![
        Migration {
            version: 1,
            description: "initial — projects + conversations",
            sql: include_str!("../migrations/001_initial.sql"),
            kind: MigrationKind::Up,
        },
        Migration {
            version: 2,
            description: "pipeline activation index",
            sql: include_str!("../migrations/002_pipeline_activation.sql"),
            kind: MigrationKind::Up,
        },
        Migration {
            version: 3,
            description: "runtime — tasks/workers/comments",
            sql: include_str!("../migrations/003_runtime.sql"),
            kind: MigrationKind::Up,
        },
        Migration {
            version: 4,
            description: "review — comments.kind column",
            sql: include_str!("../migrations/004_comments_kind.sql"),
            kind: MigrationKind::Up,
        },
        Migration {
            version: 5,
            description: "usage telemetry — worker_usage_log + cc_usage_log + usage_config",
            sql: include_str!("../migrations/005_usage.sql"),
            kind: MigrationKind::Up,
        },
        Migration {
            version: 6,
            description: "fanout — task lane columns + fanout_groups/fanout_lanes",
            sql: include_str!("../migrations/006_fanout.sql"),
            kind: MigrationKind::Up,
        },
        Migration {
            version: 7,
            description: "invocation audit — per-invocation persistence/audit trail",
            sql: include_str!("../migrations/007_invocation_audit.sql"),
            kind: MigrationKind::Up,
        },
        Migration {
            version: 8,
            description: "nested groups — fanout_groups.parent_group_id + parent_lane",
            sql: include_str!("../migrations/008_nested_groups.sql"),
            kind: MigrationKind::Up,
        },
        Migration {
            version: 9,
            description: "git config — author name/email for worker worktree commits",
            sql: include_str!("../migrations/009_git_config.sql"),
            kind: MigrationKind::Up,
        },
    ];

    tauri::Builder::default()
        .plugin(
            tauri_plugin_sql::Builder::default()
                .add_migrations(DB_URL, migrations)
                .build(),
        )
        .setup(|app| {
            let handle = app.handle().clone();
            tauri::async_runtime::block_on(async move {
                let data_dir = handle.path().app_data_dir().expect("no app data dir");
                std::fs::create_dir_all(&data_dir).ok();
                let db_path = data_dir.join("agent_bus.db");

                let pool = sqlx::sqlite::SqlitePoolOptions::new()
                    .connect_with(
                        sqlx::sqlite::SqliteConnectOptions::new()
                            .filename(&db_path)
                            .create_if_missing(true),
                    )
                    .await
                    .expect("could not open store pool");

                // This pool — not tauri-plugin-sql — owns the schema. The
                // plugin only migrates when the frontend calls Database.load(),
                // which this app never does (all data goes through Rust commands
                // over this pool). So apply migrations here, idempotently.
                run_migrations(&pool)
                    .await
                    .expect("could not run migrations");

                // Workspace state (Plan 1).
                let project_store = Arc::new(ProjectStore::new(pool.clone()));
                handle.manage(WorkspaceState { store: project_store.clone() });

                // Secrets / keychain (S1). Real OS keychain on macOS; an
                // in-memory fake elsewhere keeps the seam usable in any build.
                #[cfg(target_os = "macos")]
                let keychain: Arc<dyn secrets::KeychainStore> =
                    Arc::new(secrets::SecurityFrameworkKeychain::new());
                #[cfg(not(target_os = "macos"))]
                let keychain: Arc<dyn secrets::KeychainStore> =
                    Arc::new(secrets::FakeKeychain::new());
                handle.manage(secrets::api::KeychainState { store: keychain.clone() });

                // Git author config (S1).
                handle.manage(workspace::git_config::GitConfigState { pool: pool.clone() });

                // Runtime state (Plan 3).
                let (project_id, project_root, pipe) = load_active(&project_store).await;
                let pipe = Arc::new(pipe);
                let tasks = Arc::new(TaskStore::new(pool.clone()));
                let invocation_audit = Arc::new(runtime::invocation_audit::InvocationAuditStore::new(pool.clone()));
                let brake = Arc::new(Brake::new());

                // F4 crash recovery: release any tasks stuck in `running`.
                let _ = tasks.release_orphaned_running(now_unix()).await;

                // Runtime state — keep an Arc so the terminal dispatcher can reuse the same logic.
                let runtime_state_arc = Arc::new(RuntimeState {
                    tasks: tasks.clone(), brake: brake.clone(), pipeline: pipe.clone(),
                    project_id: project_id.clone(), project_root: project_root.clone(),
                });
                handle.manage(RuntimeState {
                    tasks: tasks.clone(),
                    brake: brake.clone(),
                    pipeline: pipe.clone(),
                    project_id: project_id.clone(),
                    project_root: project_root.clone(),
                });

                // Review state (Plan 4).
                let review_state = review::api::ReviewState {
                    comments: std::sync::Arc::new(review::store::CommentStore::new(pool.clone())),
                };
                handle.manage(review_state);

                // Usage Telemetry (Plan 5). The WorkerUsageStore is the concrete
                // UsageSink (kernel seam); the brake-state callback lets Telemetry
                // read Runtime's brake without depending on the runtime crate.
                use usage_telemetry::cc_log::CcUsageStore;
                use usage_telemetry::worker_log::WorkerUsageStore;
                let cc_store = Arc::new(CcUsageStore::new(pool.clone()));
                let worker_usage = Arc::new(WorkerUsageStore::new(pool.clone()));
                let usage_sink: Arc<dyn agent_bus_core::UsageSink> = worker_usage.clone();
                let usage_state_arc = Arc::new(usage_telemetry::api::UsageState {
                    cc: cc_store.clone(), worker: worker_usage.clone(), pool: pool.clone(),
                    is_braked: Arc::new({ let b = brake.clone(); move || b.is_on() }),
                });
                handle.manage(usage_telemetry::api::UsageState {
                    cc: cc_store.clone(),
                    worker: worker_usage.clone(),
                    pool: pool.clone(),
                    is_braked: Arc::new({ let b = brake.clone(); move || b.is_on() }),
                });

                // Conversational Control (Plan 6). Build the tool catalog by UNIONing every supplier's tools().
                let mut specs = Vec::new();
                specs.extend(pipeline::api::tools());
                specs.extend(runtime::api::tools());
                specs.extend(review::api::tools());
                specs.extend(usage_telemetry::api::tools());
                specs.extend(runners::api::tools());
                specs.extend(workspace::api::tools());
                specs.extend(secrets::api::tools());
                specs.extend(conversational_control::api::tools());
                let catalog = Arc::new(ToolCatalog::new(specs));
                debug_assert!(catalog.duplicate_names().is_empty(), "tool name collision in catalog");

                // The real dispatcher both CompositeEngine branches reach: the
                // slash branch (parser -> dispatch) and the agentic loop (model
                // -> dispatch -> feed result back). Routes a ToolCallRequest to
                // the owning supplier's logic (C1).
                let dispatcher: Arc<dyn ToolDispatcher> = Arc::new(RootDispatcher {
                    runtime: runtime_state_arc.clone(),
                    usage: usage_state_arc.clone(),
                    app: handle.clone(),
                });

                // The terminal's free-form chat engine (Plan llm_chat). One
                // chat runner; the conversation's project_id is the stable
                // dialogue_id (D4). The system framing is the terminal's
                // operating prompt; model + budget are v1 defaults.
                let chat_runner: Arc<dyn llm_chat::chat::ChatRunner> =
                    Arc::new(llm_chat::claude_cli::ClaudeChatRunner::new());
                handle.manage(DesignSessionState { runner: chat_runner.clone() });
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
                    ).with_delta_sink(Some(make_conversation_delta_sink(handle.clone()))));
                let engine: Arc<dyn conversational_control::engine::ConversationEngine> =
                    Arc::new(CompositeEngine::new(command_engine, agentic_engine));

                let convo_store = Arc::new(ConversationStore::new(pool.clone()));

                // 24h-summarisation on launch (D5).
                if !project_id.is_empty() {
                    if let Ok(Some(mut convo)) = convo_store.load(&project_id).await {
                        if conversational_control::summarise::summarise_on_launch(
                            &mut convo, uuid::Uuid::new_v4().to_string(), now_unix(),
                        ) {
                            let _ = convo_store.save(&convo).await;
                        }
                    }
                }

                handle.manage(TerminalState {
                    catalog: catalog.clone(),
                    engine,
                    store: convo_store,
                    project_id: project_id.clone(),
                });

                // Spawn one continuous worker loop per team. Each loop calls
                // process_one_claim and emits task.changed on a settle.
                if !pipe.teams.is_empty() {
                    let revision_reader: Option<Arc<dyn runtime::revision::RevisionBundleReader>> =
                        Some(Arc::new(SqliteRevisionReader { pool: pool.clone() }));
                    spawn_worker_loops(handle.clone(), pipe.clone(), tasks.clone(), brake.clone(), project_root, Some(usage_sink.clone()), revision_reader, pool.clone(), Some(make_task_log_sink(handle.clone())), Some(invocation_audit.clone()), Some(keychain.clone()));
                }

                // Auto-meter sweep (D8/D9). v1 config has auto_meter_enabled=0 so
                // decide() returns NoChange and nothing happens; when v1.1 flips
                // the flag this trips/releases Runtime's brake by reason.
                {
                    let cc = cc_store.clone();
                    let worker = worker_usage.clone();
                    let brake = brake.clone();
                    let pool = pool.clone();
                    let handle = handle.clone();
                    tauri::async_runtime::spawn(async move {
                        use usage_telemetry::api::load_config;
                        use usage_telemetry::brake_policy::{BrakeDecision, AUTO_METER_REASON};
                        use usage_telemetry::snapshot::{auto_brake_decision, compute_snapshot};
                        loop {
                            tokio::time::sleep(std::time::Duration::from_secs(15)).await;
                            let cfg = load_config(&pool).await;
                            if !cfg.auto_meter_enabled { continue; }
                            let auto_on = brake.state().reason.as_deref() == Some(AUTO_METER_REASON);
                            let now = now_unix();
                            if let Ok(snap) = compute_snapshot(&cc, &worker, &cfg, brake.is_on(), now).await {
                                match auto_brake_decision(&snap, &cfg, auto_on) {
                                    BrakeDecision::SetOn(reason) => { brake.set_on(reason); let _ = handle.emit("usage.changed", ()); }
                                    BrakeDecision::Release => { brake.set_off(); let _ = handle.emit("usage.changed", ()); }
                                    BrakeDecision::NoChange => {}
                                }
                            }
                        }
                    });
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            workspace::api::workspace_create_project,
            workspace::api::workspace_list_projects,
            workspace::api::workspace_get_project,
            workspace::api::workspace_set_active_pipeline,
            workspace::api::workspace_remove_project,
            workspace::api::read_artifact,
            workspace::git_config::git_config_get,
            workspace::git_config::git_config_set,
            secrets::api::runner_set_api_key,
            secrets::api::runner_get_api_key_status,
            secrets::api::runner_clear_api_key,
            pipeline::api::pipeline_list,
            pipeline::api::pipeline_load,
            kickoff_generate_cmd,
            list_seed_templates_cmd,
            seed_template_cmd,
            design_session_turn_cmd,
            best_effort_validate_cmd,
            create_project_from_draft,
            pipeline_to_draft_cmd,
            save_pipeline_edits,
            runtime::api::inject_topic,
            runtime::api::approve_gate,
            runtime::api::reject_gate,
            runtime::api::revise_gate,
            runtime::api::list_tasks,
            runtime::api::brake_on,
            runtime::api::brake_off,
            runtime::api::brake_state,
            runtime::api::scale_team,
            review::api::add_comment,
            review::api::list_comments,
            review::api::reanchor_comments,
            review::api::delete_comment,
            review::api::record_verdict,
            usage_telemetry::api::usage_snapshot,
            usage_telemetry::api::usage_set_budget,
            usage_telemetry::api::usage_set_auto_meter,
            conversational_control::api::send_message,
            conversational_control::api::get_conversation,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Spawn a polling worker loop per team. v1 runs one loop per team (concurrent
/// workers per team is v1.1). Each iteration runs process_one_claim; on a
/// Composition-root factory: map a team's resolved RunnerConfig to a concrete
/// Runner. claude-cli is the default and always available. anthropic-api
/// resolves its per-team API key from `api_key_env` via the process
/// environment; a team requesting anthropic-api with no resolvable key yields a
/// clear RunnerError (NOT a panic) so the worker loop can surface it rather than
/// crash. The runner kind is chosen per team — Runtime depends only on
/// `Arc<dyn Runner>` and never learns which kind it got (the ACL seal).
fn runner_for(
    config: &pipeline::model::RunnerConfig,
    resolve_key: &dyn Fn(&pipeline::model::RunnerConfig) -> Option<String>,
) -> Result<Arc<dyn Runner>, RunnerError> {
    use agent_bus_core::RunnerKind;
    match config.kind {
        RunnerKind::ClaudeCli => Ok(Arc::new(ClaudeCliRunner::new())),
        RunnerKind::AnthropicApi => {
            // Keychain-first, then api_key_env — both live in the injected
            // resolver (built at the root). The resolved key is a plain String
            // passed into AnthropicApiRunner::new: NO keychain/OS type crosses
            // the Runner trait (the ACL seal). An unresolvable key is a clear
            // error, NOT a panic.
            let key = resolve_key(config).ok_or_else(|| {
                RunnerError::Other(
                    "anthropic-api runner: no API key found in the keychain or `api_key_env`".into(),
                )
            })?;
            Ok(Arc::new(AnthropicApiRunner::new(key)))
        }
    }
}

/// settle it emits a `task.changed` event the frontend listens for.
#[allow(clippy::too_many_arguments)]
fn spawn_worker_loops(
    handle: tauri::AppHandle,
    pipeline: Arc<Pipeline>,
    tasks: Arc<TaskStore>,
    brake: Arc<Brake>,
    project_root: String,
    usage_sink: Option<Arc<dyn agent_bus_core::UsageSink>>,
    revision_reader: Option<Arc<dyn runtime::revision::RevisionBundleReader>>,
    pool: sqlx::SqlitePool,
    log_sink: Option<Arc<runtime::pool::LogSinkFactory>>,
    audit: Option<Arc<runtime::invocation_audit::InvocationAuditStore>>,
    keychain: Option<Arc<dyn secrets::KeychainStore>>,
) {
    let fanout = Arc::new(runtime::fanout_store::FanOutStore::new(pool));
    for team in pipeline.teams.clone() {
        // Select the runner kind per team at the composition root. If a team
        // requests anthropic-api but its key can't be resolved, keep the worker
        // loop alive on the default claude-cli runner rather than panicking —
        // we never crash the whole pool over one team's runner config.
        let effective = team.effective_runner();
        let resolve_key = |c: &pipeline::model::RunnerConfig| -> Option<String> {
            // Keychain-first: account = api_key_env name if present, else the
            // default "anthropic-api" account. Then fall back to the env var.
            let account = c.api_key_env.as_deref().unwrap_or("anthropic-api");
            if let Some(kc) = keychain.as_ref() {
                if let Ok(k) = kc.get(secrets::api::SERVICE, account) {
                    if !k.is_empty() {
                        return Some(k);
                    }
                }
            }
            c.api_key_env.as_ref().and_then(|n| std::env::var(n).ok())
        };
        let runner: Arc<dyn Runner> = match runner_for(&effective, &resolve_key) {
            Ok(r) => r,
            Err(e) => {
                eprintln!(
                    "app: team `{}` runner selection failed ({e}); falling back to claude-cli",
                    team.id
                );
                Arc::new(ClaudeCliRunner::new())
            }
        };
        let ctx = PoolContext {
            pipeline: pipeline.clone(),
            runner,
            tasks: tasks.clone(),
            fanout: fanout.clone(),
            brake: brake.clone(),
            project_root: std::path::PathBuf::from(&project_root),
            read_prompt: Arc::new({
                let root = project_root.clone();
                move |t: &Team| {
                    std::fs::read_to_string(std::path::Path::new(&root).join(&t.prompt))
                        .unwrap_or_default()
                }
            }),
            usage_sink: usage_sink.clone(),
            revision_reader: revision_reader.clone(),
            log_sink: log_sink.clone(),
            audit: audit.clone(),
        };
        let handle = handle.clone();
        let team = team.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                match process_one_claim(&ctx, &team).await {
                    Ok(runtime::pool::ClaimOutcome::Settled { task_id, .. }) => {
                        let _ = handle.emit("task.changed", task_id);
                        // A settle recorded worker usage; tell the meter to refresh.
                        let _ = handle.emit("usage.changed", ());
                    }
                    Ok(runtime::pool::ClaimOutcome::RateLimited { .. }) => {
                        // Reactive brake (spec) — already the behaviour; reason
                        // surfaces on the meter via brake_state.
                        ctx.brake.set_on("rate-limit");
                        let _ = handle.emit("task.changed", "rate-limited");
                        let _ = handle.emit("usage.changed", ());
                    }
                    _ => {}
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        });
    }
}

#[cfg(test)]
mod runner_factory_tests {
    use super::runner_for;
    use agent_bus_core::{EffortMode, RunnerKind};
    use pipeline::model::RunnerConfig;

    fn cfg(kind: RunnerKind, api_key_env: Option<&str>) -> RunnerConfig {
        RunnerConfig {
            kind,
            model: "claude-opus-4-7".into(),
            effort: EffortMode::Standard,
            api_key_env: api_key_env.map(|s| s.to_string()),
        }
    }

    // A resolver that mimics the root's env fallback (no keychain in unit tests).
    fn env_resolver(c: &RunnerConfig) -> Option<String> {
        c.api_key_env.as_ref().and_then(|n| std::env::var(n).ok())
    }

    #[test]
    fn claude_cli_kind_builds_a_runner() {
        let r = runner_for(&cfg(RunnerKind::ClaudeCli, None), &env_resolver);
        assert!(r.is_ok(), "claude-cli must always build");
    }

    #[test]
    fn anthropic_api_with_resolvable_key_builds_a_runner() {
        std::env::set_var("R1_TEST_KEY_PRESENT", "sk-test-123");
        let r = runner_for(
            &cfg(RunnerKind::AnthropicApi, Some("R1_TEST_KEY_PRESENT")),
            &env_resolver,
        );
        std::env::remove_var("R1_TEST_KEY_PRESENT");
        assert!(r.is_ok(), "anthropic-api with a resolvable key must build");
    }

    #[test]
    fn anthropic_api_resolves_via_keychain_first() {
        // A resolver that returns a key WITHOUT any env var set proves the
        // keychain-first path: the factory uses whatever the resolver yields.
        let kc_resolver = |_c: &RunnerConfig| Some("sk-from-keychain".to_string());
        let r = runner_for(&cfg(RunnerKind::AnthropicApi, None), &kc_resolver);
        assert!(r.is_ok(), "a keychain-resolved key must build even with no api_key_env");
    }

    #[test]
    fn anthropic_api_with_no_resolvable_key_is_a_clear_error_not_a_panic() {
        // Arc<dyn Runner> is not Debug, so match the Result rather than unwrap_err.
        let none_resolver = |_c: &RunnerConfig| None;
        match runner_for(&cfg(RunnerKind::AnthropicApi, None), &none_resolver) {
            Err(runners::output::RunnerError::Other(msg)) => {
                assert!(msg.to_lowercase().contains("api key"), "msg: {msg}");
            }
            Err(other) => panic!("expected Other, got {other:?}"),
            Ok(_) => panic!("expected a clear error, got a runner"),
        }
    }
}

#[cfg(test)]
mod task_log_tests {
    use super::TaskLogBuffer;
    use std::sync::{Arc, Mutex};

    #[test]
    fn buffer_coalesces_until_flushed_then_emits_task_id_and_delta() {
        let emitted: Arc<Mutex<Vec<(String, String)>>> = Arc::new(Mutex::new(vec![]));
        let e = emitted.clone();
        let mut buf = TaskLogBuffer::new("T-1".into());
        // push two fragments; force_flush combines and emits once with the task id
        buf.push("chunk-a ");
        buf.push("chunk-b");
        buf.force_flush(&mut |task_id: &str, delta: &str| {
            e.lock().unwrap().push((task_id.to_string(), delta.to_string()));
        });
        assert_eq!(*emitted.lock().unwrap(), vec![("T-1".to_string(), "chunk-a chunk-b".to_string())]);
    }

    #[test]
    fn force_flush_on_empty_buffer_emits_nothing() {
        let emitted: Arc<Mutex<Vec<(String, String)>>> = Arc::new(Mutex::new(vec![]));
        let e = emitted.clone();
        let mut buf = TaskLogBuffer::new("T-2".into());
        buf.force_flush(&mut |t: &str, d: &str| e.lock().unwrap().push((t.to_string(), d.to_string())));
        assert!(emitted.lock().unwrap().is_empty());
    }

    #[test]
    fn flush_if_due_emits_once_buffer_exceeds_threshold() {
        let emitted: Arc<Mutex<Vec<(String, String)>>> = Arc::new(Mutex::new(vec![]));
        let e = emitted.clone();
        let mut buf = TaskLogBuffer::new("T-3".into());
        // a >80-char fragment trips the size threshold immediately
        let big = "x".repeat(90);
        buf.push(&big);
        buf.flush_if_due(&mut |t: &str, d: &str| e.lock().unwrap().push((t.to_string(), d.to_string())));
        let got = emitted.lock().unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, "T-3");
        assert_eq!(got[0].1.len(), 90);
    }
}

#[cfg(test)]
mod revision_reader_tests {
    use super::SqliteRevisionReader;
    use runtime::revision::RevisionBundleReader;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::str::FromStr;

    async fn pool_with_comment(task: &str, kind: &str, anchor: Option<&str>, note: &str) -> sqlx::SqlitePool {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:").unwrap().foreign_keys(false);
        let pool = SqlitePoolOptions::new().connect_with(opts).await.unwrap();
        for sql in [
            include_str!("../migrations/001_initial.sql"),
            include_str!("../migrations/003_runtime.sql"),
            include_str!("../migrations/004_comments_kind.sql"),
        ] {
            for stmt in sql.split(';') {
                let s = stmt.trim();
                if !s.is_empty() { sqlx::query(s).execute(&pool).await.unwrap(); }
            }
        }
        sqlx::query("INSERT INTO comments (id, task_id, artifact_path, anchor_text, anchor_offset, note, kind, created_at) VALUES (?,?,?,?,?,?,?,?)")
            .bind("c1").bind(task).bind("artifacts/specs/T-1-v1.md")
            .bind(anchor).bind::<Option<i64>>(None).bind(note).bind(kind).bind(1000i64)
            .execute(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn reads_inline_comment_into_bundle() {
        let pool = pool_with_comment("T-1", "inline", Some("batch key"), "per-row").await;
        let reader = SqliteRevisionReader { pool };
        let bundle = reader.load("T-1").await;
        assert_eq!(bundle.notes.len(), 1);
        assert_eq!(bundle.notes[0].kind, "inline");
        assert_eq!(bundle.notes[0].anchor_text.as_deref(), Some("batch key"));
        assert_eq!(bundle.notes[0].note, "per-row");
    }

    #[tokio::test]
    async fn missing_task_yields_empty_bundle() {
        let pool = pool_with_comment("T-1", "inline", None, "x").await;
        let reader = SqliteRevisionReader { pool };
        assert!(reader.load("T-NOPE").await.notes.is_empty());
    }
}

#[cfg(test)]
mod migration_tests {
    use super::run_migrations;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

    // Regression: the GUI boot path opens a *file* pool and the app — not
    // tauri-plugin-sql — must create + migrate the schema. The in-memory store
    // tests never exercised this, so a fresh launch panicked ("unable to open
    // database file") and would then have hit "no such table".
    #[tokio::test]
    async fn fresh_file_is_created_migrated_and_idempotent() {
        let dir = std::env::temp_dir().join("agent_bus_app_migration_test");
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("boot_test.db");
        let _ = std::fs::remove_file(&db); // ensure a truly fresh file

        // create_if_missing(true) is the fix for the code-14 panic.
        let pool = SqlitePoolOptions::new()
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(&db)
                    .create_if_missing(true),
            )
            .await
            .expect("pool opens and creates a missing file");

        run_migrations(&pool).await.expect("first migration run");
        // Second run must be a no-op — migration 004's ALTER would error if
        // re-applied, so this proves the user_version gate works.
        run_migrations(&pool)
            .await
            .expect("second run is idempotent");

        let projects: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='projects'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(projects, 1, "projects table created (migration 001)");

        let kind_cols: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pragma_table_info('comments') WHERE name='kind'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(kind_cols, 1, "migration 004 column present exactly once");

        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(version, 9, "all nine migrations recorded");

        let _ = std::fs::remove_file(&db);
    }
}

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

#[cfg(test)]
mod design_session_tests {
    use super::*;
    use llm_chat::chat::{ChatReply, ChatUsage};
    use llm_chat::fake::FakeChatRunner;
    use pipeline::design_session::{design_session_turn, kickoff_generate, Step};

    #[test]
    fn seed_template_cmd_returns_a_draft_for_a_known_id() {
        let d = pipeline::seed_template::seed_template("ddd-spec-plan-impl").unwrap();
        assert_eq!(d.teams.len(), 7);
        assert!(pipeline::seed_template::seed_template("nope").is_none());
        assert!(!pipeline::seed_template::seed_templates().is_empty());
    }

    #[tokio::test]
    async fn root_kickoff_produces_a_draft_from_a_canned_reply() {
        let canned = "two teams.\n```json\n{\"kind\":\"teams\",\"teams\":[{\"id\":\"research\",\"name\":\"Research\"}]}\n```";
        let runner = FakeChatRunner::new(vec![ChatReply { text: canned.into(), usage: ChatUsage::default() }]);
        let draft = kickoff_generate(&runner, "s1", "design a flow").await;
        assert_eq!(draft.teams.len(), 1);
        assert_eq!(draft.teams[0].id, "research");
    }

    #[tokio::test]
    async fn root_turn_applies_a_teams_slice() {
        let runner = FakeChatRunner::new(vec![ChatReply {
            text: "ok\n```json\n{\"kind\":\"teams\",\"teams\":[{\"id\":\"a\",\"name\":\"A\"},{\"id\":\"b\",\"name\":\"B\"}]}\n```".into(),
            usage: ChatUsage::default(),
        }]);
        let draft = pipeline::draft::DraftPipeline::empty();
        let out = design_session_turn(&runner, "s1", Step::Teams, draft, "add two teams").await;
        assert_eq!(out.updated_draft.teams.len(), 2);
    }

    #[test]
    fn root_best_effort_reports_issues_for_an_incomplete_draft() {
        let mut d = pipeline::draft::DraftPipeline::empty();
        d.teams.push(pipeline::draft::DraftTeam::new("research", "Research"));
        let issues = pipeline::draft::best_effort_validate(&d);
        assert!(issues.iter().any(|i| i.contains("research") && i.contains("prompt")));
    }

    use pipeline::draft::DraftTeam;
    use workspace::api::WorkspaceState;
    use workspace::store::ProjectStore;
    use std::sync::Arc as StdArc;

    async fn workspace_state(pool: sqlx::SqlitePool) -> WorkspaceState {
        sqlx::query(include_str!("../migrations/001_initial.sql")).execute(&pool).await.unwrap();
        WorkspaceState { store: StdArc::new(ProjectStore::new(pool)) }
    }

    fn complete_draft(root_friendly_id: &str) -> DraftPipeline {
        let mut d = DraftPipeline::empty();
        d.id = root_friendly_id.into();
        d.name = "Demo".into();
        let mut a = DraftTeam::new("research", "Research");
        a.prompt_body = "investigate".into();
        a.outputs.on_approve = Some("writers".into());
        let mut b = DraftTeam::new("writers", "Writers");
        b.prompt_body = "write".into();
        d.teams.push(a);
        d.teams.push(b);
        d
    }

    #[tokio::test]
    async fn create_from_an_invalid_draft_writes_nothing_and_errors() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        let ws = workspace_state(pool).await;
        let root = std::env::temp_dir().join(format!("abp-cpfd-bad-{}", uuid::Uuid::new_v4()));
        // an invalid draft: a team routes to a non-existent node -> hard validate fails
        let mut d = complete_draft("bad");
        d.teams[0].outputs.on_approve = Some("ghost".into());
        let err = create_project_from_draft_inner(&ws, "Demo".into(), root.to_string_lossy().into(), d)
            .await
            .unwrap_err();
        assert!(!err.is_empty());
        // nothing written
        assert!(!root.join("pipelines").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn create_from_a_valid_draft_writes_files_and_activates() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        let ws = workspace_state(pool).await;
        let root = std::env::temp_dir().join(format!("abp-cpfd-ok-{}", uuid::Uuid::new_v4()));
        let project = create_project_from_draft_inner(&ws, "Demo".into(), root.to_string_lossy().into(), complete_draft("demo"))
            .await
            .unwrap();
        // files written
        assert!(root.join("pipelines/demo.yaml").exists());
        assert_eq!(std::fs::read_to_string(root.join("prompts/research.md")).unwrap(), "investigate");
        // active pipeline set to the draft id
        let reloaded = ws.store.get(&agent_bus_core::ProjectId(project.id.0.clone())).await.unwrap();
        assert_eq!(reloaded.active_pipeline_id.map(|p| p.0), Some("demo".to_string()));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn save_pipeline_edits_overwrites_yaml_and_prompts() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        let ws = workspace_state(pool).await;
        let root = std::env::temp_dir().join(format!("abp-save-{}", uuid::Uuid::new_v4()));
        let project = create_project_from_draft_inner(&ws, "Demo".into(), root.to_string_lossy().into(), complete_draft("demo"))
            .await.unwrap();

        // edit: change a team prompt body, then save
        let mut edited = complete_draft("demo");
        edited.teams[0].prompt_body = "REWRITTEN body".into();
        save_pipeline_edits_inner(&ws, project.id.0.clone(), edited).await.unwrap();

        let body = std::fs::read_to_string(root.join("prompts/research.md")).unwrap();
        assert_eq!(body, "REWRITTEN body");
        let reloaded = pipeline::store::PipelineStore::new(root.to_string_lossy().into_owned()).load("demo").unwrap();
        assert_eq!(reloaded.id, "demo");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn save_pipeline_edits_rejects_an_invalid_draft_without_writing() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        let ws = workspace_state(pool).await;
        let root = std::env::temp_dir().join(format!("abp-save-bad-{}", uuid::Uuid::new_v4()));
        let project = create_project_from_draft_inner(&ws, "Demo".into(), root.to_string_lossy().into(), complete_draft("demo"))
            .await.unwrap();
        let before = std::fs::read_to_string(root.join("prompts/research.md")).unwrap();

        let mut bad = complete_draft("demo");
        bad.teams[0].prompt_body = "should NOT be written".into();
        bad.teams[0].outputs.on_approve = Some("ghost-node".into());
        let err = save_pipeline_edits_inner(&ws, project.id.0.clone(), bad).await.unwrap_err();
        assert!(!err.is_empty());
        assert_eq!(std::fs::read_to_string(root.join("prompts/research.md")).unwrap(), before);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn pipeline_to_draft_inner_reads_prompt_bodies_back() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        let ws = workspace_state(pool).await;
        let root = std::env::temp_dir().join(format!("abp-todraft-{}", uuid::Uuid::new_v4()));
        let project = create_project_from_draft_inner(&ws, "Demo".into(), root.to_string_lossy().into(), complete_draft("demo"))
            .await.unwrap();
        let loaded = pipeline::store::PipelineStore::new(root.to_string_lossy().into_owned()).load("demo").unwrap();

        let draft = pipeline_to_draft_inner(&ws, project.id.0.clone(), &loaded).await.unwrap();
        let research = draft.teams.iter().find(|t| t.id == "research").unwrap();
        assert_eq!(research.prompt_body, "investigate");
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[cfg(test)]
mod composite_engine_tests {
    use super::{extract_tool_call_block, parse_tool_call};

    // --- Task 1: tool-call block extraction + parse into the kernel type (VF2) -

    #[test]
    fn extracts_a_fenced_json_tool_call_block() {
        let prose = "I'll inject that.\n\n```json\n{\"tool\":\"inject_topic\",\"args\":{\"topic\":\"03-scheduling\"}}\n```\n";
        let block = extract_tool_call_block(prose).expect("a fenced block");
        // VF2: parse straight into agent_bus_core::ToolCallRequest, no shadow type.
        let req = parse_tool_call(&block).unwrap();
        assert_eq!(req.tool_name, "inject_topic");
        assert_eq!(req.args["topic"], "03-scheduling");
    }

    #[test]
    fn falls_back_to_a_bare_fence() {
        let prose = "ok\n```\n{\"tool\":\"usage_snapshot\",\"args\":{}}\n```";
        let block = extract_tool_call_block(prose).expect("a bare fence");
        let req = parse_tool_call(&block).unwrap();
        assert_eq!(req.tool_name, "usage_snapshot");
    }

    #[test]
    fn plain_prose_has_no_block() {
        assert!(extract_tool_call_block("T-042 is in design; nothing to do.").is_none());
    }

    #[test]
    fn missing_args_defaults_to_empty_object() {
        let req = parse_tool_call("{\"tool\":\"usage_snapshot\"}").unwrap();
        assert_eq!(req.tool_name, "usage_snapshot");
        assert_eq!(req.args, serde_json::json!({}));
    }

    #[test]
    fn invalid_json_is_an_error() {
        assert!(parse_tool_call("{ this is not json").is_err());
    }

    // --- Tasks 2-7: the agentic loop, routing, and end-to-end -----------------

    use super::{AgenticChatEngine, CompositeEngine, MAX_STEPS};
    use agent_bus_core::{ToolCallResult, ToolSpec};
    use conversational_control::catalog::ToolCatalog;
    use conversational_control::dispatch::{FakeDispatcher, ToolDispatcher};
    use conversational_control::engine::{ConversationEngine, EngineReply, FakeEngine};
    use llm_chat::chat::{ChatReply, ChatRunner, ChatUsage};
    use llm_chat::fake::FakeChatRunner;
    use runtime::brake::Brake;
    use serde_json::json;
    use std::sync::Arc;

    fn catalog() -> ToolCatalog {
        ToolCatalog::new(vec![ToolSpec {
            name: "inject_topic".into(),
            description: "Inject a topic".into(),
            input_schema: json!({"type":"object"}),
            supplier_context: "runtime".into(),
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

    // C2: display-only streaming forwards prose deltas while behaviour is unchanged.
    #[tokio::test]
    async fn agentic_engine_streams_deltas_for_a_plain_answer() {
        use super::ConversationDeltaEmitter;
        use llm_chat::chat::DeltaSink;
        let runner = Arc::new(FakeChatRunner::with_deltas(
            vec![reply("All clear, nothing to do.")],
            vec![vec!["All clear, ".into(), "nothing to do.".into()]],
        ));
        let seen = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let s = seen.clone();
        let sink: DeltaSink = Box::new(move |d: &str| s.lock().unwrap().push(d.to_string()));
        let resets = Arc::new(std::sync::Mutex::new(0usize));
        let r2 = resets.clone();
        let reset: Box<dyn Fn() + Send + Sync> = Box::new(move || { *r2.lock().unwrap() += 1; });
        let eng = agentic(runner.clone(), Arc::new(FakeDispatcher::new()), Arc::new(Brake::new()))
            .with_delta_sink(Some(ConversationDeltaEmitter { sink, reset }));
        let out = eng.respond("status?", &catalog()).await;
        assert_eq!(out.text, "All clear, nothing to do.");
        assert!(out.tool_calls.is_empty());
        assert_eq!(*seen.lock().unwrap(), vec!["All clear, ".to_string(), "nothing to do.".to_string()]);
        // reset fired once at the start of the single model step.
        assert_eq!(*resets.lock().unwrap(), 1);
    }

    // Task 2: one tool-call then a final answer => one composite turn.
    #[tokio::test]
    async fn two_step_loop_dispatches_once_then_finishes_with_a_composite_turn() {
        let runner = Arc::new(FakeChatRunner::new(vec![
            reply("On it.\n```json\n{\"tool\":\"inject_topic\",\"args\":{\"topic\":\"03-scheduling\"}}\n```"),
            reply("Done — task T-9 was injected for 03-scheduling."),
        ]));
        let disp = Arc::new(
            FakeDispatcher::new().with("inject_topic", ToolCallResult::Ok { result: json!({"task_id":"T-9"}) }),
        );
        let brake = Arc::new(Brake::new());
        let eng = agentic(runner.clone(), disp.clone(), brake);

        let r = eng.respond("kick off research on 03-scheduling", &catalog()).await;

        assert_eq!(r.text, "Done — task T-9 was injected for 03-scheduling.");
        assert_eq!(r.tool_calls.len(), 1);
        assert_eq!(r.tool_calls[0].request.tool_name, "inject_topic");
        assert!(matches!(r.tool_calls[0].result, Some(ToolCallResult::Ok { .. })));
        assert_eq!(disp.received.lock().unwrap().len(), 1);
        let got = runner.received.lock().unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[1].dialogue_id, "p");
        assert!(got[1].user_message.contains("inject_topic"));
        assert!(got[1].user_message.contains("T-9"));
    }

    // Task 3: plain-prose first reply finishes with no dispatch.
    #[tokio::test]
    async fn plain_prose_first_reply_finishes_with_no_dispatch() {
        let runner = Arc::new(FakeChatRunner::new(vec![reply("T-042 is in design; nothing to do.")]));
        let disp = Arc::new(FakeDispatcher::new());
        let brake = Arc::new(Brake::new());
        let eng = agentic(runner.clone(), disp.clone(), brake);

        let r = eng.respond("how is T-042 going?", &catalog()).await;

        assert_eq!(r.text, "T-042 is in design; nothing to do.");
        assert!(r.tool_calls.is_empty());
        assert_eq!(disp.received.lock().unwrap().len(), 0);
        assert_eq!(runner.received.lock().unwrap().len(), 1);
    }

    // Task 4: unknown tool fed back as an error; model recovers.
    #[tokio::test]
    async fn unknown_tool_call_is_fed_back_and_the_model_recovers() {
        let runner = Arc::new(FakeChatRunner::new(vec![
            reply("```json\n{\"tool\":\"frobnicate\",\"args\":{}}\n```"),
            reply("```json\n{\"tool\":\"inject_topic\",\"args\":{\"topic\":\"x\"}}\n```"),
            reply("Injected x as T-1."),
        ]));
        let disp = Arc::new(
            FakeDispatcher::new().with("inject_topic", ToolCallResult::Ok { result: json!({"task_id":"T-1"}) }),
        );
        let brake = Arc::new(Brake::new());
        let eng = agentic(runner.clone(), disp.clone(), brake);

        let r = eng.respond("do the thing", &catalog()).await;

        assert_eq!(r.text, "Injected x as T-1.");
        assert_eq!(disp.received.lock().unwrap().len(), 1);
        assert_eq!(disp.received.lock().unwrap()[0].tool_name, "inject_topic");
        assert_eq!(r.tool_calls.len(), 1);
        let got = runner.received.lock().unwrap();
        assert_eq!(got.len(), 3);
        assert!(got[1].user_message.contains("unknown tool"));
        assert!(got[1].user_message.contains("frobnicate"));
    }

    // Task 4: malformed JSON fed back; model recovers.
    #[tokio::test]
    async fn malformed_json_tool_call_is_fed_back_then_recovers() {
        let runner = Arc::new(FakeChatRunner::new(vec![
            reply("```json\n{ this is not json\n```"),
            reply("Sorry — nothing to do after all."),
        ]));
        let disp = Arc::new(FakeDispatcher::new());
        let brake = Arc::new(Brake::new());
        let eng = agentic(runner.clone(), disp.clone(), brake);

        let r = eng.respond("go", &catalog()).await;

        assert_eq!(r.text, "Sorry — nothing to do after all.");
        assert!(r.tool_calls.is_empty());
        assert_eq!(disp.received.lock().unwrap().len(), 0);
        let got = runner.received.lock().unwrap();
        assert!(got[1].user_message.contains("not a valid tool call"));
    }

    // Task 5: max-steps cap stops cleanly.
    #[tokio::test]
    async fn loop_stops_at_max_steps_when_the_model_never_finishes() {
        let endless = reply("```json\n{\"tool\":\"inject_topic\",\"args\":{\"topic\":\"x\"}}\n```");
        let runner = Arc::new(FakeChatRunner::new(vec![endless]));
        let disp = Arc::new(
            FakeDispatcher::new().with("inject_topic", ToolCallResult::Ok { result: json!({"task_id":"T"}) }),
        );
        let brake = Arc::new(Brake::new());
        let eng = agentic(runner.clone(), disp.clone(), brake);

        let r = eng.respond("loop forever", &catalog()).await;

        assert!(r.text.contains("step limit reached"));
        assert_eq!(runner.received.lock().unwrap().len(), MAX_STEPS);
        assert_eq!(disp.received.lock().unwrap().len(), MAX_STEPS);
        assert_eq!(r.tool_calls.len(), MAX_STEPS);
    }

    // Task 5: brake already on => stop before any model call or dispatch.
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
        assert_eq!(runner.received.lock().unwrap().len(), 0);
        assert_eq!(disp.received.lock().unwrap().len(), 0);
    }

    // Task 6: CompositeEngine routing by leading '/'.
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
        assert_eq!(r.text, "CMD-BRANCH");
    }

    // Task 7: end-to-end through send_message_inner — slash turn + composite turn.
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
        let command: Arc<dyn ConversationEngine> =
            Arc::new(CommandEngine::new(disp.clone() as Arc<dyn ToolDispatcher>));

        let runner = Arc::new(FakeChatRunner::new(vec![
            reply("on it\n```json\n{\"tool\":\"inject_topic\",\"args\":{\"topic\":\"y\"}}\n```"),
            reply("Injected y as T-3."),
        ]));
        let brake = Arc::new(Brake::new());
        let chat_eng: Arc<dyn ConversationEngine> = Arc::new(agentic(runner.clone(), disp.clone(), brake));

        let engine: Arc<dyn ConversationEngine> = Arc::new(CompositeEngine::new(command, chat_eng));

        // 1) a slash command -> command branch -> one tool-call turn
        let convo = send_message_inner("p", &cat, engine.as_ref(), &store, "/inject 03-x", 500).await.unwrap();
        assert_eq!(convo.turns.len(), 2);
        assert!(convo.turns[1].text.contains("Done"));
        assert_eq!(convo.turns[1].tool_calls.len(), 1);

        // 2) a plain line on the SAME conversation -> agentic branch -> ONE composite turn
        let convo = send_message_inner("p", &cat, engine.as_ref(), &store, "kick off y", 600).await.unwrap();
        assert_eq!(convo.turns.len(), 4); // alternation held across both branches (DD2)
        assert_eq!(convo.turns[3].text, "Injected y as T-3.");
        assert_eq!(convo.turns[3].tool_calls.len(), 1);
        assert_eq!(convo.turns[3].tool_calls[0].request.tool_name, "inject_topic");

        let reloaded = store.load("p").await.unwrap().unwrap();
        assert_eq!(reloaded.turns.len(), 4);
    }

    // R2: the auto-meter sweep applies a BrakeDecision to Runtime's brake by
    // reason. These exercise the exact decision->apply path the root sweep runs.
    #[test]
    fn sweep_decision_trips_and_releases_brake_by_auto_meter_reason() {
        use runtime::brake::Brake;
        use usage_telemetry::brake_policy::{decide, BrakeDecision, AUTO_METER_REASON};

        let brake = Brake::new();
        fn apply(brake: &Brake, d: BrakeDecision) {
            match d {
                BrakeDecision::SetOn(reason) => brake.set_on(reason),
                BrakeDecision::Release => brake.set_off(),
                BrakeDecision::NoChange => {}
            }
        }

        // over threshold, not auto-on -> trips on with the auto-meter reason
        let auto_on = brake.state().reason.as_deref() == Some(AUTO_METER_REASON);
        apply(&brake, decide(0.96, auto_on, 0.95, 0.85));
        assert!(brake.is_on());
        assert_eq!(brake.state().reason.as_deref(), Some(AUTO_METER_REASON));

        // dropped below off threshold, auto-on -> releases
        let auto_on = brake.state().reason.as_deref() == Some(AUTO_METER_REASON);
        apply(&brake, decide(0.80, auto_on, 0.95, 0.85));
        assert!(!brake.is_on());
        assert_eq!(brake.state().reason, None);
    }

    #[test]
    fn sweep_never_releases_a_manual_brake() {
        use runtime::brake::Brake;
        use usage_telemetry::brake_policy::{decide, BrakeDecision, AUTO_METER_REASON};

        let brake = Brake::new();
        brake.set_on("rate-limit"); // reactive/manual reason, NOT auto-meter
        let auto_on = brake.state().reason.as_deref() == Some(AUTO_METER_REASON);
        // even with low pct, a non-auto brake must not be auto-released
        let d = decide(0.10, auto_on, 0.95, 0.85);
        assert_eq!(d, BrakeDecision::NoChange);
        assert!(brake.is_on());
        assert_eq!(brake.state().reason.as_deref(), Some("rate-limit"));
    }
}
