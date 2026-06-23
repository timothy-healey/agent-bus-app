//! claude CLI argument construction for a chat turn. Pure: takes a ChatRequest
//! (+ optional resume session id) and returns the argv vector. Simpler than the
//! worker command line (no settings / add-dir / permission-mode): chat is a
//! plain multi-turn --print invocation. The --resume flag is the ONLY place a
//! claude session id is named, and it never leaves this crate (F3).

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
