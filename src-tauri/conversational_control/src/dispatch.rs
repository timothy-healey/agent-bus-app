//! The customer→supplier seam (D1). conversational_control depends on NO
//! supplier crate; instead it calls them through this trait. The concrete impl
//! (RootDispatcher) lives at the composition root, the only place that imports
//! every context. Tests use FakeDispatcher.

use agent_bus_core::{ToolCallRequest, ToolCallResult};
use async_trait::async_trait;
use std::sync::Mutex;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DispatchError {
    #[error("unknown tool: {0}")]
    UnknownTool(String),
    #[error("supplier rejected the call: {0}")]
    Supplier(String),
}

/// Routes a ToolCallRequest to the owning supplier's command and returns the
/// supplier's result already translated into ToolCallResult. Object-safe so the
/// root can hold it as `Arc<dyn ToolDispatcher>`.
#[async_trait]
pub trait ToolDispatcher: Send + Sync {
    async fn dispatch(&self, req: &ToolCallRequest) -> ToolCallResult;
}

/// A test double: maps tool_name -> canned ToolCallResult, and records every
/// request it received (mirrors runners::fake::FakeRunner).
pub struct FakeDispatcher {
    responses: std::collections::HashMap<String, ToolCallResult>,
    pub received: Mutex<Vec<ToolCallRequest>>,
}

impl FakeDispatcher {
    pub fn new() -> Self {
        Self { responses: std::collections::HashMap::new(), received: Mutex::new(vec![]) }
    }
    /// Seed a canned result for a tool name (builder style).
    pub fn with(mut self, tool: &str, result: ToolCallResult) -> Self {
        self.responses.insert(tool.to_string(), result);
        self
    }
}

impl Default for FakeDispatcher {
    fn default() -> Self { Self::new() }
}

#[async_trait]
impl ToolDispatcher for FakeDispatcher {
    async fn dispatch(&self, req: &ToolCallRequest) -> ToolCallResult {
        self.received.lock().unwrap().push(req.clone());
        match self.responses.get(&req.tool_name) {
            Some(r) => r.clone(),
            None => ToolCallResult::Err { error: format!("unknown tool: {}", req.tool_name) },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn fake_returns_canned_result_and_records_request() {
        let d = FakeDispatcher::new()
            .with("inject_topic", ToolCallResult::Ok { result: json!({"task_id": "T-7"}) });
        let req = ToolCallRequest { tool_name: "inject_topic".into(), args: json!({"topic": "x"}) };
        let res = d.dispatch(&req).await;
        assert_eq!(res, ToolCallResult::Ok { result: json!({"task_id": "T-7"}) });
        assert_eq!(d.received.lock().unwrap().len(), 1);
        assert_eq!(d.received.lock().unwrap()[0].tool_name, "inject_topic");
    }

    #[tokio::test]
    async fn fake_errs_on_unseeded_tool() {
        let d = FakeDispatcher::new();
        let req = ToolCallRequest { tool_name: "mystery".into(), args: json!({}) };
        assert_eq!(d.dispatch(&req).await, ToolCallResult::Err { error: "unknown tool: mystery".into() });
    }
}
