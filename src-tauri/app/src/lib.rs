mod brake_persist;
mod events;
mod pipeline_activator;
mod process_records;
mod process_registry;

use std::sync::Arc;
use tauri::{Emitter, Manager};
use tauri_plugin_sql::{Migration, MigrationKind};
use workspace::{api::WorkspaceState, store::ProjectStore};

use runtime::api::RuntimeState;
use runtime::brake::Brake;
use runtime::task_store::TaskStore;
use pipeline::model::Pipeline;
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
        (10, include_str!("../migrations/010_project_target_repo.sql")),
        (11, include_str!("../migrations/011_skill_sources.sql")),
        (12, include_str!("../migrations/012_runtime_stores.sql")),
        (13, include_str!("../migrations/013_lifecycle_hardening.sql")),
        (14, include_str!("../migrations/014_task_worktree.sql")),
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
    /// LF20: brake-on (Stop) kills every in-flight `claude` process group.
    process_registry: Arc<process_registry::ProcessRegistry>,
    /// LH6: persist the brake row on every set_on/set_off through the dispatcher.
    brake_store: Arc<brake_persist::BrakeStore>,
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

/// The app's git-backed worktree provider (worktree isolation). Implements the
/// git-unaware `runtime::engine::WorktreeProvider` over `workspace`'s
/// `WorktreeGit` seam. Lives at the composition root so `runtime` never learns
/// `git`. The project root scopes every worktree under `<root>/worktrees/`.
pub struct GitCliWorktreeProvider {
    git: std::sync::Arc<dyn workspace::worktree::WorktreeGit>,
    project_root: String,
}

impl GitCliWorktreeProvider {
    pub fn new(git: std::sync::Arc<dyn workspace::worktree::WorktreeGit>, project_root: String) -> Self {
        Self { git, project_root }
    }

    fn worktree_path_for(&self, run_id: &str, item_key: &str) -> String {
        std::path::Path::new(&self.project_root)
            .join("worktrees")
            .join(run_id)
            .join(item_key)
            .to_string_lossy()
            .into_owned()
    }
}

impl runtime::engine::WorktreeProvider for GitCliWorktreeProvider {
    fn ensure(&self, run_id: &str, item_key: &str, _target_repo: &str) -> Result<String, String> {
        let path = self.worktree_path_for(run_id, item_key);
        // Idempotent: a worktree dir already present (resume / re-claim) is reused.
        if std::path::Path::new(&path).is_dir() {
            return Ok(path);
        }
        let branch = format!("agent-bus/{run_id}/{item_key}");
        // Create the branch off the project root repo's current HEAD; the
        // path-scope guard runs no git on an out-of-subtree path. (The common
        // case has project_root == target_repo; per the spec the worktree lives
        // under <project_root>/worktrees/ and branches off HEAD.)
        workspace::worktree::add_worktree_inner(
            self.git.as_ref(),
            &self.project_root,
            &path,
            &branch,
            "HEAD",
        )?;
        Ok(path)
    }

    fn reset(&self, worktree_path: &str) -> Result<(), String> {
        workspace::worktree::reset_worktree_inner(self.git.as_ref(), &self.project_root, worktree_path)
    }
}

#[cfg(test)]
mod worktree_provider_tests {
    use super::*;

    #[test]
    fn git_cli_worktree_provider_ensure_derives_path_and_branch() {
        use std::sync::Mutex;
        struct FakeGit { added: Mutex<Vec<(String, String, String, String)>> }
        impl workspace::worktree::WorktreeGit for FakeGit {
            fn list_porcelain(&self, _r: &str) -> Result<String, String> { Ok(String::new()) }
            fn remove(&self, _r: &str, _p: &str) -> Result<(), String> { Ok(()) }
            fn add(&self, repo: &str, path: &str, branch: &str, base: &str) -> Result<(), String> {
                self.added.lock().unwrap().push((repo.into(), path.into(), branch.into(), base.into()));
                Ok(())
            }
            fn reset(&self, _p: &str) -> Result<(), String> { Ok(()) }
        }
        let git = std::sync::Arc::new(FakeGit { added: Mutex::new(vec![]) });
        let provider = GitCliWorktreeProvider::new(git.clone(), "/proj".into());
        let path = runtime::engine::WorktreeProvider::ensure(&provider, "R-1", "alpha", "/repo").unwrap();
        assert_eq!(path, "/proj/worktrees/R-1/alpha");
        let calls = git.added.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "/proj");
        assert_eq!(calls[0].1, "/proj/worktrees/R-1/alpha");
        assert_eq!(calls[0].2, "agent-bus/R-1/alpha");
        assert_eq!(calls[0].3, "HEAD");
    }
}

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
            let _ = sink_handle.emit(crate::events::CONVERSATION_DELTA, serde_json::json!({ "text": b.text, "reset": false }));
            b.text.clear();
            b.last = Instant::now();
        }
    });

    let reset: Box<dyn Fn() + Send + Sync> = Box::new(move || {
        let mut b = state.lock().unwrap();
        if !b.text.is_empty() {
            let _ = handle.emit(crate::events::CONVERSATION_DELTA, serde_json::json!({ "text": b.text, "reset": false }));
            b.text.clear();
        }
        let _ = handle.emit(crate::events::CONVERSATION_DELTA, serde_json::json!({ "text": "", "reset": true }));
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

/// Build a per-task `LogSink` factory that emits throttled `task-log` Tauri
/// events `{ task_id, delta, kind }`. Display-only: the payload is the task id, a
/// prose fragment, and its channel (`"output"` | `"thinking"`); no stream-json
/// idiom crosses here. Each task gets one coalescing buffer PER kind so output and
/// thinking never interleave within an emitted fragment, and concurrent workers'
/// logs never interleave within a flush.
fn make_task_log_sink(handle: tauri::AppHandle) -> Arc<runtime::log_sink::LogSinkFactory> {
    use runners::output::{LogDelta, LogKind};
    Arc::new(move |task_id: &str| -> runners::output::LogSink {
        use std::sync::Mutex;
        // One coalescing buffer per kind so output and thinking never interleave
        // within a single emitted fragment; each emit carries its kind.
        let out_buf = Arc::new(Mutex::new(TaskLogBuffer::new(task_id.to_string())));
        let think_buf = Arc::new(Mutex::new(TaskLogBuffer::new(task_id.to_string())));
        let handle = handle.clone();
        Box::new(move |d: &LogDelta| {
            let (buf, kind_str) = match d.kind {
                LogKind::Output => (&out_buf, "output"),
                LogKind::Thinking => (&think_buf, "thinking"),
            };
            let mut b = buf.lock().unwrap();
            b.push(&d.text);
            let handle = handle.clone();
            b.flush_if_due(&mut |tid: &str, delta: &str| {
                let _ = handle.emit(
                    crate::events::TASK_LOG,
                    serde_json::json!({ "task_id": tid, "delta": delta, "kind": kind_str }),
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
            working_dir: None,
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

/// Bounded **repair turn** budget for invalid tool ARGUMENTS (T1, mirroring
/// DS-Schema's `chat_with_repair`): when the model emits a `{tool,args}` whose
/// args fail schema validation, re-prompt on the SAME dialogue_id naming the
/// specific arg error up to this many times before giving up with a clear,
/// non-dispatch turn. A valid first emit dispatches with no extra call.
pub const MAX_REPAIR_RETRIES: usize = 2;

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
        // T1: count of bounded arg-repair re-prompts used so far this turn.
        let mut repair_attempts: usize = 0;

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
                working_dir: None,
            };

            // T2: on a runner that supports NATIVE tool-use (the anthropic-api
            // path), drive the tool step structurally — the model picks a catalog
            // tool via tool_choice:auto and returns a schema-valid {tool,args}
            // directly (no fenced-json parse, no T1 arg-repair turn needed). A
            // prose finish surfaces as NoResult from the structured call, so we
            // fetch the final answer once via the plain path. Else the T1
            // prompt+extract+validate+repair path (UNCHANGED below). T1's
            // validate-before-dispatch stays the universal safety net for BOTH.
            let request = if self.runner.supports_structured() {
                let tools = catalog_tool_defs(catalog);
                match self.runner.chat_structured(&req, &tools, None).await {
                    Ok(structured) => {
                        ToolCallRequest { tool_name: structured.tool_name, args: structured.args }
                    }
                    // No tool_use block => the model answered in prose; it is done.
                    // Fetch the final prose answer once (display-only streaming if set).
                    Err(llm_chat::chat::ChatError::NoResult) => {
                        let reply = match &self.delta {
                            Some(emitter) => {
                                (emitter.reset)();
                                self.runner.chat_stream(&req, &emitter.sink).await
                            }
                            None => self.runner.chat(&req).await,
                        };
                        return match reply {
                            Ok(r) => EngineReply { text: r.text, tool_calls },
                            Err(e) => EngineReply { text: format!("[terminal error] {e}"), tool_calls },
                        };
                    }
                    Err(e) => return EngineReply { text: format!("[terminal error] {e}"), tool_calls },
                }
            } else {
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
                match parse_tool_call(&block) {
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
                }
            };

            // On the structured path the model can only pick a published tool, but
            // guard anyway (an unknown name is impossible via tool_choice but the
            // dispatcher relies on it): an unknown tool surfaces as an error turn.
            if catalog.by_name(&request.tool_name).is_none() {
                return EngineReply {
                    text: format!(
                        "[terminal error] model selected unknown tool `{}`",
                        request.tool_name
                    ),
                    tool_calls,
                };
            }

            // T1: validate the model's args against the tool's PUBLISHED input_schema
            // BEFORE dispatch (the ACL seal — the loop validates the JSON schema, never
            // the supplier arg types). On a miss, run a BOUNDED repair turn: re-prompt
            // on the SAME dialogue_id naming the specific arg error (≤ MAX_REPAIR_RETRIES),
            // then give up with a clear non-dispatch turn (mirrors chat_with_repair).
            if let Some(spec) = catalog.by_name(&request.tool_name) {
                if let Err(arg_err) =
                    conversational_control::validate::validate_args(&spec.input_schema, &request.args)
                {
                    if repair_attempts >= MAX_REPAIR_RETRIES {
                        return EngineReply {
                            text: format!(
                                "[invalid tool arguments] `{}`: {arg_err}. Gave up after {} repair \
                                 attempts; no tool was dispatched.",
                                request.tool_name, MAX_REPAIR_RETRIES,
                            ),
                            tool_calls,
                        };
                    }
                    repair_attempts += 1;
                    next_user_message = format!(
                        "Tool call rejected: arguments for `{}` are invalid — {arg_err}. \
                         Re-emit the tool call as a single fenced ```json \
                         {{\"tool\":\"{}\",\"args\":{{…}}}} block with the corrected arguments.",
                        request.tool_name, request.tool_name,
                    );
                    continue;
                }
            }

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

/// Map the published tool catalog into the kernel `ChatToolDef`s the native
/// structured-output path offers the model (T2 Task 4). Each tool's
/// `input_schema` is T1's PUBLISHED schema — the SAME schema the loop validates
/// against before dispatch — so the native call is schema-valid by construction
/// and the validate-before-dispatch safety net still applies. Only the kernel
/// ChatToolDef crosses the ChatRunner trait (the ACL seal).
fn catalog_tool_defs(catalog: &ToolCatalog) -> Vec<llm_chat::chat::ChatToolDef> {
    catalog
        .specs()
        .iter()
        .map(|s| llm_chat::chat::ChatToolDef {
            name: s.name.clone(),
            description: s.description.clone(),
            input_schema: s.input_schema.clone(),
        })
        .collect()
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

/// OHS (G6): probe a model's availability with a 1-token call. Returns a
/// `ModelProbe { status, message }` — `ok` / `unavailable` (the runner
/// classified a model-not-found, surfaced as "pick another") / `error`. The
/// live subprocess path is structural-only (no headless claude here); the
/// default `ClaudeCliRunner` is used (authoring-time check, no team config).
/// Pure classification + the seam are unit-tested in `runners::probe`.
#[tauri::command(rename_all = "snake_case")]
async fn test_model(model: String) -> Result<runners::probe::ModelProbe, String> {
    let runner = runners::claude_cli::ClaudeCliRunner::new();
    Ok(runners::probe::run_probe(&runner, &model).await)
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
    target_repo: Option<String>,
) -> Result<Project, String> {
    // 1. HARD validate + serialize (shared gate; nothing is written when invalid).
    let (yaml_rel, yaml, prompts) = pipeline::draft::prepare_pipeline_write(&draft)?;
    let pipeline = draft.to_pipeline();

    // 2. Create the project row (Workspace; ~ already expanded inside).
    let home = std::env::var("HOME").unwrap_or_default();
    let expanded = workspace::api::expand_tilde(&root, &home);
    let mut project = Project::new(name, std::path::PathBuf::from(expanded), now_unix());
    // A5: target_repo carried through create, tilde-expanded like root.
    project.target_repo = target_repo
        .filter(|s| !s.trim().is_empty())
        .map(|s| workspace::api::expand_tilde(&s, &home));
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
/// After the project is created + its pipeline set active, trigger RUNTIME
/// activation (swap RuntimeState's active pipeline + spawn the new pipeline's
/// worker loops) so `/inject` targets the just-created project — the bug this fix
/// addresses (runtime activation was previously boot-only).
#[tauri::command(rename_all = "snake_case")]
async fn create_project_from_draft(
    state: tauri::State<'_, WorkspaceState>,
    activator: tauri::State<'_, Arc<pipeline_activator::PipelineActivator>>,
    name: String,
    root: String,
    draft: DraftPipeline,
    target_repo: Option<String>,
) -> Result<Project, String> {
    let project = create_project_from_draft_inner(&state, name, root, draft, target_repo).await?;
    activator.activate(&project.id.0).await?;
    Ok(project)
}

/// LF20: the frontend Stop button. A root-crate wrapper over the runtime brake
/// so the composition-root `ProcessRegistry` (which the runtime crate must not
/// know about) gets its `kill_all` on Stop. Replaces `runtime::api::brake_on` in
/// the invoke handler; sets the brake then kills every in-flight `claude` group.
#[tauri::command(rename_all = "snake_case")]
async fn brake_on(
    runtime: tauri::State<'_, Arc<RuntimeState>>,
    registry: tauri::State<'_, Arc<process_registry::ProcessRegistry>>,
    brake_store: tauri::State<'_, Arc<brake_persist::BrakeStore>>,
    reason: Option<String>,
) -> Result<runtime::brake::BrakeState, String> {
    let reason = reason.unwrap_or_else(|| "manual".into());
    runtime.brake.set_on(reason.as_str());
    // LH6: persist the brake row so an explicit Stop survives app exit/reboot.
    let _ = brake_store.save(true, Some(&reason), now_unix()).await;
    // LH5: Stop kills WORKERS only — the user's in-flight chat survives. Exit
    // (the RunEvent handler) still kill_all()s both. LH3: the bounded grace runs
    // on the blocking pool so it never stalls a Tokio worker; the user-facing
    // Stop awaits the kill so the UI is truthful.
    registry.kill_workers_blocking().await;
    Ok(runtime.brake.state())
}

/// LH6/LH8b: root `brake_off` (Resume) wrapper. Mirrors the root `brake_on`:
/// clears the runtime brake, persists the OFF row, and clears the registry kill
/// latch so resume re-enables spawning (the LH8a-left `Queued` rows are then
/// re-claimed by the live worker loops). Replaces `runtime::api::brake_off` in
/// the invoke handler so persistence + latch-clear stay consistent.
#[tauri::command(rename_all = "snake_case")]
async fn brake_off(
    runtime: tauri::State<'_, Arc<RuntimeState>>,
    registry: tauri::State<'_, Arc<process_registry::ProcessRegistry>>,
    brake_store: tauri::State<'_, Arc<brake_persist::BrakeStore>>,
) -> Result<runtime::brake::BrakeState, String> {
    runtime.brake.set_off();
    let _ = brake_store.save(false, None, now_unix()).await;
    registry.end_killing(); // LH8b: resume re-enables spawning
    Ok(runtime.brake.state())
}

/// OHS command: activate a project's runtime (swap the active pipeline + respawn
/// worker loops at a fresh generation). Called by the frontend when a project is
/// created or selected. Idempotent: re-activating the same project just bumps the
/// generation (old loops retire, new ones take over).
#[tauri::command(rename_all = "snake_case")]
async fn activate_project(
    activator: tauri::State<'_, Arc<pipeline_activator::PipelineActivator>>,
    project_id: String,
) -> Result<(), String> {
    activator.activate(&project_id).await
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

/// State holding the skill catalog seam (A4). Real impl = FsSkillScanner; tests
/// inject a FakeSkillCatalog. The composition root is the ONLY place that knows
/// both the Project type and the skills crate — it resolves a project's
/// `skill_sources` to plain `ClaudeRoot`s; the skills crate never learns Project.
pub struct SkillCatalogState {
    pub catalog: Arc<dyn skills::SkillCatalog>,
}

/// Resolve a project's skill roots: the always-on global `~/.claude` first, then
/// each configured project source (treated as a `.claude` root). Pure (home +
/// sources injected) so it is unit-testable. Skips an empty home for the global
/// root (no silent scan of a wrong dir).
pub fn resolve_skill_roots(home: &str, sources: &[String]) -> Vec<skills::ClaudeRoot> {
    let mut roots = Vec::new();
    if !home.is_empty() {
        roots.push(skills::ClaudeRoot {
            path: std::path::PathBuf::from(home).join(".claude"),
            source: skills::SkillSource::Global,
        });
    }
    for s in sources {
        roots.push(skills::ClaudeRoot {
            path: std::path::PathBuf::from(s),
            source: skills::SkillSource::Project,
        });
    }
    roots
}

/// Inner logic (testable without Tauri State): scan each root via the catalog,
/// tag by source, and merge with project-wins precedence. The catalog's `list`
/// is called per-root so each root's entries can be tagged + merged correctly.
pub fn list_skills_inner(
    catalog: &dyn skills::SkillCatalog,
    roots: &[skills::ClaudeRoot],
) -> Vec<skills::SkillEntry> {
    let per_root: Vec<(skills::SkillSource, Vec<skills::SkillEntry>)> = roots
        .iter()
        .map(|r| (r.source, catalog.list(std::slice::from_ref(r))))
        .collect();
    skills::merge_with_precedence(per_root)
}

/// OHS command (A4/G4): list the skills + slash commands available for
/// authoring-time autocomplete. `project_id` is OPTIONAL (G4): when present,
/// roots = global `~/.claude` + the project's configured `skill_sources` (project
/// wins); when absent (e.g. the new-project wizard, before a project exists),
/// roots = the global `~/.claude` ONLY — so the global catalog still loads during
/// creation. Skills are discovery sugar; an unknown project id falls back to the
/// global-only catalog rather than erroring (it must never break prompt authoring).
#[tauri::command(rename_all = "snake_case")]
async fn list_skills(
    ws: tauri::State<'_, WorkspaceState>,
    catalog: tauri::State<'_, SkillCatalogState>,
    project_id: Option<String>,
) -> Result<Vec<skills::SkillEntry>, String> {
    use agent_bus_core::ProjectId;
    // Project sources, merged in only when a project is supplied AND resolvable.
    let sources: Vec<String> = match project_id {
        Some(id) => match ws.store.get(&ProjectId(id)).await {
            Ok(project) => project.skill_sources,
            // No such project yet (mid-create) — global-only, never an error.
            Err(_) => Vec::new(),
        },
        None => Vec::new(),
    };
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_default();
    let roots = resolve_skill_roots(&home, &sources);
    Ok(list_skills_inner(catalog.catalog.as_ref(), &roots))
}

#[cfg(test)]
mod list_skills_tests {
    use super::{list_skills_inner, resolve_skill_roots};
    use skills::{FakeSkillCatalog, SkillEntry, SkillKind, SkillSource};

    fn entry(name: &str, ns: Option<&str>) -> SkillEntry {
        SkillEntry {
            name: name.into(),
            kind: SkillKind::Skill,
            namespace: ns.map(|s| s.into()),
            description: String::new(),
            verbs: vec![],
            source: SkillSource::Global,
            qualified: false,
        }
    }

    #[test]
    fn resolve_roots_puts_global_first_then_project_sources() {
        let roots = resolve_skill_roots("/home/tim", &["/proj/.claude".to_string()]);
        assert_eq!(roots.len(), 2);
        assert_eq!(roots[0].source, SkillSource::Global);
        assert_eq!(roots[0].path, std::path::PathBuf::from("/home/tim/.claude"));
        assert_eq!(roots[1].source, SkillSource::Project);
        assert_eq!(roots[1].path, std::path::PathBuf::from("/proj/.claude"));
    }

    #[test]
    fn resolve_roots_with_no_project_sources_yields_global_only() {
        // G4: list_skills with no project (None) resolves to global-only roots —
        // the catalog still loads during creation. Empty sources == global-only.
        let roots = resolve_skill_roots("/home/tim", &[]);
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].source, SkillSource::Global);
        assert_eq!(roots[0].path, std::path::PathBuf::from("/home/tim/.claude"));
    }

    #[test]
    fn list_skills_inner_returns_the_global_catalog_with_no_project_sources() {
        // G4: even with no project, the global catalog is scanned + returned.
        let fake = FakeSkillCatalog::new(vec![entry("ddd-council", None)]);
        let roots = resolve_skill_roots("/home/tim", &[]);
        let merged = list_skills_inner(&fake, &roots);
        assert!(merged.iter().any(|e| e.name == "ddd-council" && e.source == SkillSource::Global));
    }

    #[test]
    fn resolve_roots_skips_global_when_home_empty() {
        let roots = resolve_skill_roots("", &["/proj/.claude".to_string()]);
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].source, SkillSource::Project);
    }

    #[test]
    fn list_skills_inner_tags_by_source_and_applies_precedence() {
        // The Fake returns the same canned entry for every root; per-root scan +
        // merge stamps the right source and resolves the cross-source collision.
        let fake = FakeSkillCatalog::new(vec![entry("dup", Some("plug"))]);
        let roots = resolve_skill_roots("/home/tim", &["/proj/.claude".to_string()]);
        let merged = list_skills_inner(&fake, &roots);
        // Two "dup" entries: one global (qualified loser), one project (bare winner).
        let project = merged.iter().find(|e| e.source == SkillSource::Project).unwrap();
        let global = merged.iter().find(|e| e.source == SkillSource::Global).unwrap();
        assert!(!project.qualified, "project wins the bare name");
        assert!(global.qualified, "global loser offered qualified");
    }
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
        // T1: deserialize `args` into the SAME per-tool arg struct the supplier
        // derives its `input_schema` from (single source of truth). A bad-args
        // deserialization is reported as a tool error — though by the time we get
        // here the agentic loop / parser has already schema-validated the args.
        let a = &req.args;
        // Deserialize helper: maps a serde error into a uniform tool error.
        macro_rules! parse_args {
            ($t:ty) => {
                match serde_json::from_value::<$t>(a.clone()) {
                    Ok(v) => v,
                    Err(e) => return err(format!("invalid arguments for `{}`: {e}", req.tool_name)),
                }
            };
        }
        let result = match req.tool_name.as_str() {
            "inject_topic" => {
                // ④d: inject is now Start-a-run. It returns the created Run; emit
                // run-changed (a run started) + task-changed (the board refetches).
                let args = parse_args!(runtime::api::args::InjectTopicArgs);
                match runtime::api::inject_topic_inner(
                    &self.runtime, args.topic, args.target_repo,
                ).await {
                    Ok(run) => {
                        let _ = self.app.emit(crate::events::RUN_CHANGED, &run.id);
                        let _ = self.app.emit(crate::events::TASK_CHANGED, &run.id);
                        serde_json::to_value(run).map(ok).unwrap_or_else(err)
                    }
                    Err(e) => err(e),
                }
            }
            "approve_gate" | "reject_gate" | "revise_gate" => {
                let args = parse_args!(runtime::api::args::GateArgs);
                let verdict = match req.tool_name.as_str() {
                    "approve_gate" => agent_bus_core::Verdict::Approve,
                    "reject_gate" => agent_bus_core::Verdict::Reject,
                    _ => agent_bus_core::Verdict::Revise,
                };
                match runtime::api::apply_gate_verdict_inner(&self.runtime, &args.task_id, verdict).await {
                    Ok(task) => { let _ = self.app.emit(crate::events::TASK_CHANGED, &task.id.0); serde_json::to_value(task).map(ok).unwrap_or_else(err) }
                    Err(e) => err(e),
                }
            }
            "list_invocations" => {
                let args = parse_args!(runtime::api::args::TaskActionArgs);
                match runtime::api::list_invocations_inner(&self.runtime, &args.task_id).await {
                    Ok(rows) => serde_json::to_value(rows).map(ok).unwrap_or_else(err),
                    Err(e) => err(e),
                }
            }
            "retry_task" | "force_advance" | "abandon_task" | "accept_task" => {
                let args = parse_args!(runtime::api::args::TaskActionArgs);
                let res = match req.tool_name.as_str() {
                    "retry_task" => runtime::api::retry_task_inner(&self.runtime, &args.task_id).await,
                    "force_advance" => runtime::api::force_advance_inner(&self.runtime, &args.task_id).await,
                    "accept_task" => runtime::api::accept_task_inner(&self.runtime, &args.task_id).await,
                    _ => runtime::api::abandon_task_inner(&self.runtime, &args.task_id).await,
                };
                match res {
                    Ok(task) => { let _ = self.app.emit(crate::events::TASK_CHANGED, &task.id.0); serde_json::to_value(task).map(ok).unwrap_or_else(err) }
                    Err(e) => err(e),
                }
            }
            "brake_on" => {
                let args = parse_args!(runtime::api::args::BrakeOnArgs);
                let s = self.runtime.brake.clone();
                let reason = args.reason.unwrap_or_else(|| "manual".into());
                s.set_on(reason.as_str());
                // LH6: persist the brake row.
                let _ = self.brake_store.save(true, Some(&reason), now_unix()).await;
                // LH5: Stop kills WORKERS only; the in-flight chat survives.
                // LH3: offload the bounded grace off the Tokio worker thread.
                self.process_registry.kill_workers_blocking().await;
                let _ = self.app.emit(crate::events::USAGE_CHANGED, ());
                ok(serde_json::to_value(s.state()).unwrap())
            }
            "brake_off" => {
                self.runtime.brake.set_off();
                let _ = self.brake_store.save(false, None, now_unix()).await;
                self.process_registry.end_killing(); // LH8b: resume re-enables spawning
                let _ = self.app.emit(crate::events::USAGE_CHANGED, ());
                ok(serde_json::to_value(self.runtime.brake.state()).unwrap())
            }
            "scale_team" => {
                let args = parse_args!(runtime::api::args::ScaleTeamArgs);
                match runtime::api::scale_team_inner(&self.runtime, args.team_id) {
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
                let args = parse_args!(usage_telemetry::api::args::SetAutoMeterArgs);
                match usage_telemetry::api::set_auto_meter_inner(&self.usage.pool, args.enabled).await {
                    Ok(()) => { let _ = self.app.emit(crate::events::USAGE_CHANGED, ()); ok(serde_json::json!({ "auto_meter_enabled": args.enabled })) }
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

/// LH7: which brake reasons HARD-KILL in-flight work. A manual/reactive Stop
/// kills (the operator chose to halt now); an auto-meter brake is soft only —
/// block new claims, let in-flight finish, no kill, no re-run. Keyed off the
/// same discriminator as brake persistence (LH6) so the two cannot drift.
pub fn should_kill_for_reason(reason: &str) -> bool {
    crate::brake_persist::persists_across_reboot(Some(reason))
}

/// LH6: build the boot `Brake`, restoring a persisted MANUAL/reactive brake
/// authoritatively (come up braked, no auto-resume) but letting a persisted
/// AUTO_METER_REASON brake stay OFF — the auto-meter sweep re-derives it from
/// fresh telemetry on its first tick, so the run is not stranded.
async fn restore_brake_from(store: &crate::brake_persist::BrakeStore) -> Arc<Brake> {
    let brake = Arc::new(Brake::new());
    if let Ok(p) = store.load().await {
        if p.on && crate::brake_persist::persists_across_reboot(p.reason.as_deref()) {
            brake.set_on(p.reason.unwrap_or_else(|| "manual".into()));
        }
    }
    brake
}

/// Resolve `~/.claude/projects` (the Claude Code transcript tree this app
/// ingests for the window meter). Mirrors the existing HOME convention; on a
/// machine with no HOME it returns `.claude/projects` (relative) which simply
/// yields an empty walk.
fn cc_claude_projects_dir() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    std::path::PathBuf::from(home).join(".claude").join("projects")
}

/// Resolve the active project + pipeline at startup. v1: the newest project
/// (projects[0] in the frontend ordering) and its first pipeline file. Returns
/// an empty placeholder pipeline when none exists so the app still boots.
async fn load_active(
    project_store: &ProjectStore,
) -> (String, String, Option<String>, Pipeline) {
    let empty = Pipeline {
        id: String::new(), name: String::new(), description: String::new(),
        schema_version: pipeline::model::SCHEMA_VERSION,
        defaults: None,
        teams: vec![], gates: vec![], escalations: vec![],
        forks: vec![], joins: vec![],
    };
    let Ok(projects) = project_store.list().await else { return (String::new(), String::new(), None, empty); };
    let Some(project) = projects.into_iter().next() else { return (String::new(), String::new(), None, empty); };
    let root = project.root_path.to_string_lossy().into_owned();
    // A5: the project's target_repo is the ${target_repo} default for the worker
    // loop + inject. Read here at the composition root and handed in as a plain
    // string (Runtime/pool never see the Project type).
    let target_repo = project.target_repo.clone();
    let store = pipeline::store::PipelineStore::new(&root);
    let pipe = store
        .list_ids()
        .ok()
        .and_then(|ids| ids.into_iter().next())
        .and_then(|id| store.load(&id).ok())
        .unwrap_or(empty);
    (project.id.0, root, target_repo, pipe)
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
        Migration {
            version: 10,
            description: "project target repo — projects.target_repo column",
            sql: include_str!("../migrations/010_project_target_repo.sql"),
            kind: MigrationKind::Up,
        },
        Migration {
            version: 11,
            description: "project skill sources — projects.skill_sources JSON column (A4)",
            sql: include_str!("../migrations/011_skill_sources.sql"),
            kind: MigrationKind::Up,
        },
        Migration {
            version: 12,
            description: "runtime stores — runs/stores/generator_ledger tables + tasks.run_id/item_key (④a)",
            sql: include_str!("../migrations/012_runtime_stores.sql"),
            kind: MigrationKind::Up,
        },
        Migration {
            version: 13,
            description: "lifecycle hardening — live_processes (crash reap) + brake_state (persist) (LH4/LH6)",
            sql: include_str!("../migrations/013_lifecycle_hardening.sql"),
            kind: MigrationKind::Up,
        },
        Migration {
            version: 14,
            description: "task worktree_path — per-work-item git worktree isolation",
            sql: include_str!("../migrations/014_task_worktree.sql"),
            kind: MigrationKind::Up,
        },
    ];

    // Live child process-group registry (LF20): the killable spawners register
    // each `claude` group; the exit handler + brake triggers kill them all.
    // Built BEFORE the builder so a clone reaches the `run` exit callback.
    let process_registry = Arc::new(crate::process_registry::ProcessRegistry::new());
    let process_registry_for_exit = process_registry.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_sql::Builder::default()
                .add_migrations(DB_URL, migrations)
                .build(),
        )
        .setup(move |app| {
            let process_registry = process_registry.clone();
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

                // LH4: durable live-process store. Built now (pool ready) and
                // attached to the registry so the spawner persists a record per
                // spawned pgid; the boot reap (below, before recovery) clears it.
                let live_processes =
                    Arc::new(process_records::LiveProcessStore::new(pool.clone()));
                process_registry.set_live_store(live_processes.clone());

                // LH6: durable brake state. Built now (pool ready); persisted on
                // every set_on/set_off at the root and restored (reason-aware) on
                // boot. Managed so the root `brake_on`/`brake_off` commands resolve
                // it; cloned into the dispatcher + auto-meter sweep.
                let brake_store = Arc::new(brake_persist::BrakeStore::new(pool.clone()));
                handle.manage(brake_store.clone());

                // Workspace state (Plan 1).
                let project_store = Arc::new(ProjectStore::new(pool.clone()));
                handle.manage(WorkspaceState { store: project_store.clone() });

                // Worktree cleanup (S2): real git behind the WorktreeGit seam.
                handle.manage(workspace::worktree::WorktreeState {
                    store: project_store.clone(),
                    git: Arc::new(workspace::worktree::GitCli),
                });

                // Secrets / keychain (S1). Real OS keychain on macOS; an
                // in-memory fake elsewhere keeps the seam usable in any build.
                #[cfg(target_os = "macos")]
                let keychain: Arc<dyn secrets::KeychainStore> =
                    Arc::new(secrets::SecurityFrameworkKeychain::new());
                #[cfg(not(target_os = "macos"))]
                let keychain: Arc<dyn secrets::KeychainStore> =
                    Arc::new(secrets::FakeKeychain::new());
                handle.manage(secrets::api::KeychainState { store: keychain.clone() });

                // Skill catalog (A4): the real filesystem scanner behind the
                // SkillCatalog seam. list_skills resolves a project's sources to
                // roots at this composition root (the skills crate never learns
                // Project).
                handle.manage(SkillCatalogState {
                    catalog: Arc::new(skills::FsSkillScanner::new()),
                });

                // Git author config (S1).
                handle.manage(workspace::git_config::GitConfigState { pool: pool.clone() });

                // Runtime state (Plan 3). ONE instance, shared by the Tauri
                // commands, the terminal dispatcher, and the PipelineActivator
                // (no divergent copies — the active pipeline is now interior-
                // mutable and swapped on activation, so a single source of truth
                // is essential to the re-activation fix).
                let (project_id, project_root, project_target_repo, pipe) = load_active(&project_store).await;
                let tasks = Arc::new(TaskStore::new(pool.clone()));
                let invocation_audit = Arc::new(runtime::invocation_audit::InvocationAuditStore::new(pool.clone()));

                // Worktree isolation: the git-unaware seam, built at the root so
                // `runtime` never learns `git`. Threaded into each run's
                // EngineContext (via WorkerDeps) AND used for WT2 reset-on-resume
                // below. Reuses the same GitCli managed for the cleanup commands.
                let worktree_git: std::sync::Arc<dyn workspace::worktree::WorktreeGit> =
                    std::sync::Arc::new(workspace::worktree::GitCli);
                let worktree_provider: Option<std::sync::Arc<dyn runtime::engine::WorktreeProvider>> =
                    Some(std::sync::Arc::new(GitCliWorktreeProvider::new(
                        worktree_git.clone(),
                        project_root.clone(),
                    )));
                // LH6: restore a persisted MANUAL brake (come up braked); an
                // auto-meter brake stays OFF so the sweep re-derives it.
                let brake = restore_brake_from(&brake_store).await;

                // LH4: reap any crash-orphaned `claude` groups from a prior
                // session BEFORE re-queueing their tasks, so the re-run has no
                // surviving competitor. Best-effort; clears the records.
                process_records::reap_orphans(&live_processes).await;

                // F4 crash recovery + WT2 reset-on-resume: re-queue orphaned
                // running tasks (the prior session's `claude` groups were already
                // reaped above), then reset each re-queued implementer worktree to
                // its baseline before a worker can re-claim it (the kill may have
                // left a half-written tree). Runtime stays git-unaware: it reports
                // the re-queued rows; the root (which holds the provider) resets.
                // Read-only stages carry no worktree_path, so they are skipped; a
                // cleanly-committed worktree of a task that was NOT re-queued is
                // never touched.
                match tasks.release_orphaned_running(now_unix()).await {
                    Ok(requeued) => {
                        if let Some(wp) = worktree_provider.as_ref() {
                            for t in &requeued {
                                if let Some(path) = t.worktree_path.as_deref() {
                                    if let Err(e) = wp.reset(path) {
                                        eprintln!("app: WT2 worktree reset failed for {}: {e}", t.id.0);
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => eprintln!("app: release_orphaned_running failed: {e}"),
                }

                // Bounded-buffer engine aggregates (④a/④d). ONE instance each,
                // shared by RuntimeState (start_run + gate verdicts) and the
                // activator's worker loops — a single source of truth per aggregate.
                let stores = Arc::new(runtime::store::StoreRepo::new(pool.clone()));
                let runs = Arc::new(runtime::run_store::RunStore::new(pool.clone()));
                let ledger = Arc::new(runtime::generator_ledger::GeneratorLedger::new(pool.clone()));
                let fanout = Arc::new(runtime::fanout_store::FanOutStore::new(pool.clone()));

                // Resume reconciliation (LF20 occupancy leak): after orphaned
                // `running` rows were requeued above, rebuild each store's
                // occupancy from the resident work-items so a slot reserved before
                // a kill/crash (and never released) does not keep the resumed run
                // falsely backpressured. Per active run. Best-effort: a reconcile
                // failure must not block boot.
                if let Ok(Some(active)) = runs.latest_active_for_project(&project_id).await {
                    if let Err(e) = stores.reconcile_occupancy(&active.id).await {
                        eprintln!("app: boot reconcile_occupancy failed: {e}");
                    }
                }

                let revision_reader: Option<Arc<dyn runtime::revision::RevisionBundleReader>> =
                    Some(Arc::new(SqliteRevisionReader { pool: pool.clone() }));

                let runtime_state_arc = Arc::new(RuntimeState::new(
                    tasks.clone(),
                    brake.clone(),
                    stores.clone(),
                    runs.clone(),
                    ledger.clone(),
                    fanout.clone(),
                    revision_reader.clone(),
                    Some(invocation_audit.clone()),
                    runtime::api::ActivePipeline {
                        pipeline: Arc::new(pipe),
                        project_id: project_id.clone(),
                        project_root: project_root.clone(),
                        project_target_repo: project_target_repo.clone(),
                    },
                ));
                handle.manage(runtime_state_arc.clone());
                // LF20: the process registry as managed State so the root
                // `brake_on` command (and any other consumer) can resolve it.
                handle.manage(process_registry.clone());

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
                let ingestor = Arc::new(usage_telemetry::ingest::TranscriptIngestor::new(
                    cc_claude_projects_dir(),
                    cc_store.clone(),
                ));
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
                specs.extend(skills::tools());
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
                    process_registry: process_registry.clone(),
                    brake_store: brake_store.clone(),
                });

                // The terminal's free-form chat engine (Plan llm_chat). One
                // chat runner; the conversation's project_id is the stable
                // dialogue_id (D4). The system framing is the terminal's
                // operating prompt; model + budget are v1 defaults.
                // Per-runner selection (T2): an anthropic key resolvable via the
                // keychain / ANTHROPIC_API_KEY (S1) picks the structured
                // AnthropicApiChatRunner (native tool-use); else the CLI runner
                // (degrades via the trait default). The API idiom never crosses —
                // Runtime/CC hold an opaque Arc<dyn ChatRunner>.
                let chat_runner: Arc<dyn llm_chat::chat::ChatRunner> =
                    pipeline_activator::chat_runner_for(
                        &|| pipeline_activator::resolve_chat_key(&keychain),
                        &process_registry,
                    );
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

                // PipelineActivator — owns the activation lifecycle (swap the
                // active pipeline + (re)spawn the per-team worker loops at a fresh
                // generation). Built ONCE here at boot with all the loop
                // collaborators; held in Tauri state so the create/select paths
                // can re-activate. Boot activation goes through the SAME path.
                let activator = Arc::new(pipeline_activator::PipelineActivator::new(
                    handle.clone(),
                    runtime_state_arc.clone(),
                    project_store.clone(),
                    tasks.clone(),
                    brake.clone(),
                    pipeline_activator::WorkerDeps {
                        usage_sink: Some(usage_sink.clone()),
                        revision_reader: revision_reader.clone(),
                        pool: pool.clone(),
                        log_sink: Some(make_task_log_sink(handle.clone())),
                        audit: Some(invocation_audit.clone()),
                        keychain: Some(keychain.clone()),
                        stores: stores.clone(),
                        runs: runs.clone(),
                        ledger: ledger.clone(),
                        fanout: fanout.clone(),
                        process_registry: process_registry.clone(),
                        app_data: data_dir.clone(),
                        worktree_provider: worktree_provider.clone(),
                    },
                ));
                handle.manage(activator.clone());
                // Boot activation goes through the SAME path as runtime activation.
                if let Err(e) = activator.activate(&project_id).await {
                    eprintln!("app: boot activation failed: {e}");
                }

                // Boot backfill: prime cc_usage_log from transcripts modified
                // within the rolling window so the meter is correct on launch
                // instead of 0 until the first new transcript line. Emit only
                // when rows were inserted (avoids a needless refetch).
                {
                    let cfg = usage_telemetry::api::load_config(&pool).await;
                    let n = ingestor
                        .backfill_window(cfg.window_secs, now_unix())
                        .await
                        .unwrap_or(0);
                    if n > 0 {
                        let _ = handle.emit(crate::events::USAGE_CHANGED, ());
                    }
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
                    let brake_store = brake_store.clone();
                    let ingestor = ingestor.clone();
                    tauri::async_runtime::spawn(async move {
                        use usage_telemetry::api::load_config;
                        use usage_telemetry::brake_policy::{BrakeDecision, AUTO_METER_REASON};
                        use usage_telemetry::snapshot::{auto_brake_decision, compute_snapshot};
                        // High-water mark: start at boot (BEFORE the first sleep) so
                        // the first sweep only picks up files modified during/after
                        // boot — the boot backfill already covered the window.
                        let mut last_scan = now_unix();
                        loop {
                            tokio::time::sleep(std::time::Duration::from_secs(15)).await;
                            let cfg = load_config(&pool).await;
                            let now = now_unix();

                            // Always ingest changed transcripts (the meter must stay
                            // fresh even with the auto-brake disabled).
                            let new = ingestor.ingest_changed(last_scan).await.unwrap_or(0);
                            // Advance the watermark unconditionally, even past an errored
                            // pass: full re-parse + INSERT OR IGNORE makes re-ingest
                            // idempotent, and transcripts are append-only, so any line
                            // missed by a failed pass is recovered on the file's next append.
                            last_scan = now;
                            // Bound table growth: keep two full windows of margin.
                            let _ = cc.prune(now - 2 * cfg.window_secs).await;
                            if new > 0 {
                                let _ = handle.emit(crate::events::USAGE_CHANGED, ());
                            }

                            // Only the brake DECISION is gated on the enable flag.
                            if !cfg.auto_meter_enabled { continue; }
                            let auto_on = brake.state().reason.as_deref() == Some(AUTO_METER_REASON);
                            if let Ok(snap) = compute_snapshot(&cc, &worker, &cfg, brake.is_on(), now).await {
                                match auto_brake_decision(&snap, &cfg, auto_on) {
                                    BrakeDecision::SetOn(reason) => {
                                        brake.set_on(reason.as_str());
                                        // LH6: persist the brake row.
                                        let _ = brake_store.save(true, Some(&reason), now).await;
                                        // LH7: auto-meter is a SOFT brake — block new
                                        // claims via the brake gate, let in-flight
                                        // finish, NO kill, NO re-run. (No kill here.)
                                        let _ = handle.emit(crate::events::USAGE_CHANGED, ());
                                    }
                                    BrakeDecision::Release => {
                                        brake.set_off();
                                        let _ = brake_store.save(false, None, now).await;
                                        let _ = handle.emit(crate::events::USAGE_CHANGED, ());
                                    }
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
            workspace::api::workspace_set_target_repo,
            workspace::api::workspace_set_skill_sources,
            workspace::api::workspace_remove_project,
            list_skills,
            workspace::worktree::list_worktrees,
            workspace::worktree::remove_worktree,
            workspace::dir_listing::list_dir,
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
            test_model,
            design_session_turn_cmd,
            best_effort_validate_cmd,
            create_project_from_draft,
            activate_project,
            pipeline_to_draft_cmd,
            save_pipeline_edits,
            runtime::api::inject_topic,
            runtime::api::start_run,
            runtime::api::approve_gate,
            runtime::api::reject_gate,
            runtime::api::revise_gate,
            runtime::api::list_tasks,
            runtime::api::list_invocations,
            runtime::api::retry_task,
            runtime::api::force_advance,
            runtime::api::abandon_task,
            runtime::api::accept_task,
            runtime::api::list_runs,
            runtime::api::run_store_occupancy,
            brake_on,
            brake_off,
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
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(move |_app, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                // LF20: kill every in-flight `claude` process group on quit so no
                // orphan keeps mutating a worktree after the app is gone.
                process_registry_for_exit.kill_all();
            }
        });
}

#[cfg(test)]
mod task_log_tests {
    use super::TaskLogBuffer;
    use std::sync::{Arc, Mutex};

    #[test]
    fn per_kind_buffers_coalesce_independently_and_tag_their_kind() {
        use runners::output::{LogDelta, LogKind};
        // Two buffers keyed by kind; pushing into each and force-flushing yields
        // one emit per kind, each tagged.
        let mut out_buf = TaskLogBuffer::new("T-1".into());
        let mut think_buf = TaskLogBuffer::new("T-1".into());
        for d in [
            LogDelta { kind: LogKind::Thinking, text: "rea".into() },
            LogDelta { kind: LogKind::Thinking, text: "soning".into() },
            LogDelta { kind: LogKind::Output, text: "ans".into() },
        ] {
            match d.kind {
                LogKind::Output => out_buf.push(&d.text),
                LogKind::Thinking => think_buf.push(&d.text),
            }
        }
        let emitted = std::sync::Arc::new(std::sync::Mutex::new(Vec::<(String, String)>::new()));
        let e = emitted.clone();
        out_buf.force_flush(&mut |t, d| e.lock().unwrap().push((t.into(), d.into())));
        let e2 = emitted.clone();
        think_buf.force_flush(&mut |t, d| e2.lock().unwrap().push((t.into(), d.into())));
        assert_eq!(
            *emitted.lock().unwrap(),
            vec![("T-1".into(), "ans".into()), ("T-1".into(), "reasoning".into())]
        );
    }

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

        let target_repo_cols: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pragma_table_info('projects') WHERE name='target_repo'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(target_repo_cols, 1, "migration 010 column present exactly once");

        let skill_sources_cols: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pragma_table_info('projects') WHERE name='skill_sources'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(skill_sources_cols, 1, "migration 011 column present exactly once");

        // Migration 012 (④a): the new bounded-buffer tables + the work-item
        // identity columns on tasks.
        let new_tables: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM sqlite_master WHERE type='table' \
             AND name IN ('runs','stores','generator_ledger')",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(new_tables, 3, "migration 012 created runs/stores/generator_ledger");

        let run_id_cols: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pragma_table_info('tasks') WHERE name='run_id'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(run_id_cols, 1, "migration 012 tasks.run_id present exactly once");

        let item_key_cols: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pragma_table_info('tasks') WHERE name='item_key'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(item_key_cols, 1, "migration 012 tasks.item_key present exactly once");

        // Migration 013 (LH4/LH6): the two lifecycle-hardening tables.
        let lh_tables: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM sqlite_master WHERE type='table' \
             AND name IN ('live_processes','brake_state')",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(lh_tables, 2, "migration 013 created live_processes + brake_state");

        let worktree_path_cols: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pragma_table_info('tasks') WHERE name='worktree_path'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(worktree_path_cols, 1, "migration 014 tasks.worktree_path present exactly once");

        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(version, 14, "all fourteen migrations recorded");

        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn only_manual_reasons_trigger_a_kill() {
        use usage_telemetry::brake_policy::AUTO_METER_REASON;
        assert!(crate::should_kill_for_reason("manual"));
        assert!(crate::should_kill_for_reason("rate-limit"));
        assert!(!crate::should_kill_for_reason(AUTO_METER_REASON));
    }

    #[tokio::test]
    async fn boot_restore_brakes_for_manual_not_for_auto_meter() {
        use crate::brake_persist::{BrakeStore, PersistedBrake};
        use std::str::FromStr;
        let opts = SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .foreign_keys(false);
        let pool = SqlitePoolOptions::new().connect_with(opts).await.unwrap();
        sqlx::query(include_str!("../migrations/013_lifecycle_hardening.sql"))
            .execute(&pool)
            .await
            .unwrap();
        let store = BrakeStore::new(pool);

        // Manual Stop persisted -> the Brake comes up ON with the same reason.
        store.save(true, Some("manual"), 1).await.unwrap();
        let b = crate::restore_brake_from(&store).await;
        assert!(b.is_on());
        assert_eq!(b.state().reason.as_deref(), Some("manual"));

        // Auto-meter persisted -> the Brake comes up OFF (sweep re-derives it).
        store
            .save(true, Some(usage_telemetry::brake_policy::AUTO_METER_REASON), 2)
            .await
            .unwrap();
        let b2 = crate::restore_brake_from(&store).await;
        assert!(!b2.is_on(), "auto-meter brake must not strand the run across reboot");

        let _ = PersistedBrake { on: false, reason: None, ts: 0 }; // keep import used
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
        sqlx::query(include_str!("../migrations/010_project_target_repo.sql")).execute(&pool).await.unwrap();
        sqlx::query(include_str!("../migrations/011_skill_sources.sql")).execute(&pool).await.unwrap();
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
        let err = create_project_from_draft_inner(&ws, "Demo".into(), root.to_string_lossy().into(), d, None)
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
        let project = create_project_from_draft_inner(&ws, "Demo".into(), root.to_string_lossy().into(), complete_draft("demo"), None)
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
        let project = create_project_from_draft_inner(&ws, "Demo".into(), root.to_string_lossy().into(), complete_draft("demo"), None)
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
        let project = create_project_from_draft_inner(&ws, "Demo".into(), root.to_string_lossy().into(), complete_draft("demo"), None)
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
        let project = create_project_from_draft_inner(&ws, "Demo".into(), root.to_string_lossy().into(), complete_draft("demo"), None)
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

    /// A catalog whose `approve_gate` carries a REAL derived-style schema
    /// (task_id required, string) so the loop's validate-before-dispatch + bounded
    /// repair turn (T1 Task 4) are exercised.
    fn schema_catalog() -> ToolCatalog {
        ToolCatalog::new(vec![ToolSpec {
            name: "approve_gate".into(),
            description: "Approve a gated task".into(),
            input_schema: json!({
                "type": "object",
                "required": ["task_id"],
                "properties": { "task_id": { "type": "string" } }
            }),
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

    // T1 Task 4: the model emits args that fail schema validation -> a BOUNDED
    // repair turn re-prompts on the SAME dialogue_id naming the arg error; the
    // model's corrected re-emit then dispatches. A valid first emit makes no
    // extra call (covered by two_step_loop_... above).
    #[tokio::test]
    async fn invalid_args_trigger_a_repair_turn_then_a_valid_re_emit_dispatches() {
        let runner = Arc::new(FakeChatRunner::new(vec![
            // first emit: missing the required task_id -> rejected, repair re-prompt
            reply("Approving.\n```json\n{\"tool\":\"approve_gate\",\"args\":{}}\n```"),
            // repaired emit: includes task_id -> validates -> dispatches
            reply("Fixed.\n```json\n{\"tool\":\"approve_gate\",\"args\":{\"task_id\":\"T-7\"}}\n```"),
            // final answer after the tool result is fed back
            reply("Done — T-7 approved."),
        ]));
        let disp = Arc::new(
            FakeDispatcher::new().with("approve_gate", ToolCallResult::Ok { result: json!({"id":"T-7"}) }),
        );
        let eng = agentic(runner.clone(), disp.clone(), Arc::new(Brake::new()));

        let r = eng.respond("approve the gated task", &schema_catalog()).await;

        // the corrected call dispatched exactly once
        assert_eq!(disp.received.lock().unwrap().len(), 1);
        assert_eq!(r.tool_calls.len(), 1);
        assert_eq!(r.tool_calls[0].request.args, json!({"task_id":"T-7"}));
        assert_eq!(r.text, "Done — T-7 approved.");
        // exactly three model calls: bad emit + repair re-prompt + final answer
        let got = runner.received.lock().unwrap();
        assert_eq!(got.len(), 3);
        // the repair re-prompt was on the SAME dialogue_id and named the arg error
        assert_eq!(got[1].dialogue_id, "p");
        assert!(got[1].user_message.contains("task_id"), "repair must name the bad arg: {}", got[1].user_message);
        assert!(got[1].user_message.to_lowercase().contains("invalid")
            || got[1].user_message.to_lowercase().contains("required"));
    }

    // T1 Task 4: the model never produces valid args -> after MAX_REPAIR_RETRIES the
    // loop gives up with a clear non-dispatch turn (no tool ever dispatched).
    #[tokio::test]
    async fn exhausted_arg_repair_retries_give_up_without_dispatching() {
        use super::MAX_REPAIR_RETRIES;
        // every reply omits task_id; FakeChatRunner clamps to the last reply.
        let runner = Arc::new(FakeChatRunner::new(vec![
            reply("Approving.\n```json\n{\"tool\":\"approve_gate\",\"args\":{}}\n```"),
        ]));
        let disp = Arc::new(FakeDispatcher::new());
        let eng = agentic(runner.clone(), disp.clone(), Arc::new(Brake::new()));

        let r = eng.respond("approve it", &schema_catalog()).await;

        // nothing dispatched, no tool_calls recorded
        assert_eq!(disp.received.lock().unwrap().len(), 0);
        assert!(r.tool_calls.is_empty());
        // a clear non-dispatch turn naming the failure
        assert!(r.text.contains("invalid tool arguments") || r.text.to_lowercase().contains("invalid"));
        assert!(r.text.contains("task_id"));
        // initial bad emit + MAX_REPAIR_RETRIES repair re-prompts = 1 + 2 model calls
        assert_eq!(runner.received.lock().unwrap().len(), 1 + MAX_REPAIR_RETRIES);
    }

    // ---- T2 Task 4: native tool-use on a structured (API) runner ------------

    use llm_chat::chat::{ChatToolDef, StructuredReply};
    use llm_chat::fake::FakeStructuredChatRunner;

    fn structured_engine(runner: Arc<dyn ChatRunner>, disp: Arc<FakeDispatcher>) -> AgenticChatEngine {
        AgenticChatEngine::new(
            runner,
            disp as Arc<dyn ToolDispatcher>,
            Arc::new(Brake::new()),
            "p".into(),
            "You are the god terminal.".into(),
            "m".into(),
            8192,
        )
    }

    fn struct_reply(tool: &str, args: serde_json::Value) -> StructuredReply {
        StructuredReply { tool_name: tool.into(), args, usage: ChatUsage::default() }
    }

    // A native tool call dispatches WITHOUT any fenced-json parse or repair turn.
    #[tokio::test]
    async fn structured_runner_native_tool_call_dispatches_then_finishes() {
        // Step 1: structured tool call. Step 2: NoResult (prose finish) -> plain
        // chat returns the final answer.
        let runner = Arc::new(FakeStructuredChatRunner::with_chat(
            vec![struct_reply("inject_topic", json!({"topic": "03-scheduling"}))],
            vec![reply("Done — task T-9 was injected for 03-scheduling.")],
        ));
        let disp = Arc::new(
            FakeDispatcher::new().with("inject_topic", ToolCallResult::Ok { result: json!({"task_id":"T-9"}) }),
        );
        let eng = structured_engine(runner.clone(), disp.clone());

        let r = eng.respond("kick off research on 03-scheduling", &catalog()).await;

        // dispatched exactly once via the native call (no fenced-json parse)
        assert_eq!(disp.received.lock().unwrap().len(), 1);
        assert_eq!(r.tool_calls.len(), 1);
        assert_eq!(r.tool_calls[0].request.tool_name, "inject_topic");
        assert_eq!(r.tool_calls[0].request.args, json!({"topic": "03-scheduling"}));
        assert_eq!(r.text, "Done — task T-9 was injected for 03-scheduling.");
        // the structured seam was driven with the catalog tools, tool_choice auto
        let calls = runner.structured_calls.lock().unwrap();
        assert_eq!(calls[0].0, vec!["inject_topic".to_string()]);
        assert_eq!(calls[0].1, None); // None = auto (model may finish in prose)
    }

    // The structured path's catalog tool defs carry T1's published input_schema.
    #[test]
    fn catalog_tool_defs_carry_the_published_schema() {
        let defs: Vec<ChatToolDef> = super::catalog_tool_defs(&schema_catalog());
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "approve_gate");
        assert_eq!(defs[0].input_schema["required"][0], "task_id");
    }

    // T1's validate-before-dispatch is STILL the safety net on the structured
    // path: a native call with bad args triggers the bounded repair turn rather
    // than dispatching, even though the model picked a published tool natively.
    #[tokio::test]
    async fn structured_runner_invalid_args_still_repair_then_dispatch() {
        let runner = Arc::new(FakeStructuredChatRunner::with_chat(
            vec![
                struct_reply("approve_gate", json!({})),                 // missing task_id -> repair
                struct_reply("approve_gate", json!({"task_id": "T-7"})), // corrected -> dispatch
            ],
            vec![reply("Done — T-7 approved.")], // prose finish
        ));
        let disp = Arc::new(
            FakeDispatcher::new().with("approve_gate", ToolCallResult::Ok { result: json!({"id":"T-7"}) }),
        );
        let eng = structured_engine(runner.clone(), disp.clone());

        let r = eng.respond("approve the gated task", &schema_catalog()).await;

        // the corrected native call dispatched exactly once (T1 net held)
        assert_eq!(disp.received.lock().unwrap().len(), 1);
        assert_eq!(r.tool_calls[0].request.args, json!({"task_id":"T-7"}));
        assert_eq!(r.text, "Done — T-7 approved.");
        // the repair re-prompt rode the SAME dialogue_id and named the arg error
        let got = runner.received.lock().unwrap();
        assert!(got[1].user_message.contains("task_id"));
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
