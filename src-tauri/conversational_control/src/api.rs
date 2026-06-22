//! Conversational Control OHS. The terminal's Tauri commands (send_message,
//! get_conversation) + the (empty, D8) tools() catalog entry. State holds the
//! catalog (read-only union), the engine, the dispatcher, the store, and the
//! active project_id. The send flow: load (or create) conversation -> append
//! user turn -> engine.respond -> append assistant turn (with embedded
//! tool-call results) -> save -> return the assistant turn.

use crate::catalog::ToolCatalog;
use crate::conversation::Conversation;
use crate::engine::ConversationEngine;
use crate::store::ConversationStore;
use crate::turn::Turn;
use agent_bus_core::ToolSpec;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

/// Managed state for the Conversational Control context.
pub struct TerminalState {
    pub catalog: Arc<ToolCatalog>,
    pub engine: Arc<dyn ConversationEngine>,
    pub store: Arc<ConversationStore>,
    /// The active project (one open at a time in v1). Empty string => no project.
    pub project_id: String,
}

fn now_unix() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64
}

/// Core send logic, pure of Tauri so it is directly testable. Loads/creates the
/// conversation, runs the engine, persists, returns the full conversation.
pub async fn send_message_inner(
    state_project_id: &str,
    catalog: &ToolCatalog,
    engine: &dyn ConversationEngine,
    store: &ConversationStore,
    input: &str,
    now: i64,
) -> Result<Conversation, String> {
    let mut convo = store
        .load(state_project_id)
        .await
        .map_err(|e| e.to_string())?
        .unwrap_or_else(|| Conversation::new(state_project_id, Uuid::new_v4().to_string(), now));

    convo.append(Turn::user(input, now)).map_err(|e| e.to_string())?;
    let reply = engine.respond(input, catalog).await;
    convo
        .append(Turn::assistant(reply.text, reply.tool_calls, now))
        .map_err(|e| e.to_string())?;
    store.save(&convo).await.map_err(|e| e.to_string())?;
    Ok(convo)
}

#[tauri::command]
pub async fn send_message(
    state: tauri::State<'_, TerminalState>,
    input: String,
) -> Result<Conversation, String> {
    if state.project_id.is_empty() {
        return Err("no active project".into());
    }
    send_message_inner(
        &state.project_id,
        &state.catalog,
        state.engine.as_ref(),
        &state.store,
        &input,
        now_unix(),
    )
    .await
}

#[tauri::command]
pub async fn get_conversation(
    state: tauri::State<'_, TerminalState>,
) -> Result<Option<Conversation>, String> {
    if state.project_id.is_empty() {
        return Ok(None);
    }
    state.store.load(&state.project_id).await.map_err(|e| e.to_string())
}

/// OHS contract. The terminal context is a pure consumer; it publishes no
/// app-tools of its own in v1 (D8). The empty Vec lets the composition root
/// treat all seven contexts uniformly when building the catalog union.
pub fn tools() -> Vec<ToolSpec> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatch::FakeDispatcher;
    use crate::engine::CommandEngine;
    use agent_bus_core::ToolCallResult;
    use serde_json::json;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn fixture() -> (ToolCatalog, Arc<dyn ConversationEngine>, ConversationStore) {
        let pool = SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        sqlx::query(include_str!("../../app/migrations/001_initial.sql")).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO projects (id,name,root_path,created_at,updated_at) VALUES ('p','n','/p',0,0)")
            .execute(&pool).await.unwrap();
        let catalog = ToolCatalog::new(vec![ToolSpec {
            name: "inject_topic".into(), description: "d".into(),
            input_schema: json!({"type":"object"}), supplier_context: "runtime".into(),
        }]);
        let disp = Arc::new(FakeDispatcher::new()
            .with("inject_topic", ToolCallResult::Ok { result: json!({"task_id":"T-3"}) }));
        let engine: Arc<dyn ConversationEngine> = Arc::new(CommandEngine::new(disp));
        (catalog, engine, ConversationStore::new(pool))
    }

    #[tokio::test]
    async fn send_message_inner_appends_user_then_assistant_and_persists() {
        let (catalog, engine, store) = fixture().await;
        let convo = send_message_inner("p", &catalog, engine.as_ref(), &store, "/inject 03-x", 500)
            .await
            .unwrap();
        assert_eq!(convo.turns.len(), 2);
        assert_eq!(convo.turns[0].text, "/inject 03-x");
        assert!(convo.turns[1].text.contains("Done"));
        assert_eq!(convo.turns[1].tool_calls.len(), 1);
        // reload proves it persisted
        let reloaded = store.load("p").await.unwrap().unwrap();
        assert_eq!(reloaded.turns.len(), 2);
    }

    #[tokio::test]
    async fn second_send_continues_the_same_conversation() {
        let (catalog, engine, store) = fixture().await;
        send_message_inner("p", &catalog, engine.as_ref(), &store, "/inject a", 1).await.unwrap();
        let convo = send_message_inner("p", &catalog, engine.as_ref(), &store, "/inject b", 2).await.unwrap();
        assert_eq!(convo.turns.len(), 4); // user/assistant x2, alternation held
    }

    #[test]
    fn tools_is_empty_in_v1() {
        assert!(tools().is_empty());
    }
}
