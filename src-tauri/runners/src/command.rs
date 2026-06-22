//! claude CLI argument construction. Pure: takes an InvocationRequest, returns
//! the argv vector. Kept separate from spawning so it is asserted exactly in
//! tests with no subprocess. Mirrors the spec's Worker model command line.

use crate::output::InvocationRequest;

/// The binary name. The composition root may override via PATH; v1 assumes
/// `claude` is resolvable.
pub const CLAUDE_BIN: &str = "claude";

/// Build the argv (excluding the program name) for one claude --print invocation.
pub fn build_args(req: &InvocationRequest) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "--print".into(),
        "--output-format".into(),
        "stream-json".into(),
        "--append-system-prompt".into(),
        req.system_prompt.clone(),
        "--settings".into(),
        req.settings_path.clone(),
    ];
    for dir in &req.add_dirs {
        args.push("--add-dir".into());
        args.push(dir.clone());
    }
    args.push("--permission-mode".into());
    args.push("acceptEdits".into());
    args.push("--model".into());
    args.push(req.model.clone());
    args.push("--max-thinking-tokens".into());
    args.push(req.thinking_budget.to_string());
    // The user message is the final positional argument.
    args.push(req.user_message.clone());
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req() -> InvocationRequest {
        InvocationRequest {
            task_id: "T-1".into(),
            team_id: "research".into(),
            model: "claude-opus-4-7".into(),
            thinking_budget: 32000,
            system_prompt: "You are research.".into(),
            user_message: "Investigate topic X".into(),
            settings_path: "/p/.agent-bus/runtime/T-1-research-1700.settings.json".into(),
            add_dirs: vec!["/repo".into(), "/p/artifacts/analyses".into()],
        }
    }

    #[test]
    fn builds_the_spec_command_line() {
        let args = build_args(&req());
        assert_eq!(args[0], "--print");
        assert_eq!(args[1], "--output-format");
        assert_eq!(args[2], "stream-json");
        // model + budget present
        let model_i = args.iter().position(|a| a == "--model").unwrap();
        assert_eq!(args[model_i + 1], "claude-opus-4-7");
        let budget_i = args.iter().position(|a| a == "--max-thinking-tokens").unwrap();
        assert_eq!(args[budget_i + 1], "32000");
        // user message is last
        assert_eq!(args.last().unwrap(), "Investigate topic X");
    }

    #[test]
    fn one_add_dir_flag_per_directory() {
        let args = build_args(&req());
        let count = args.iter().filter(|a| *a == "--add-dir").count();
        assert_eq!(count, 2);
        assert!(args.windows(2).any(|w| w[0] == "--add-dir" && w[1] == "/repo"));
    }

    #[test]
    fn settings_and_permission_mode_present() {
        let args = build_args(&req());
        let s_i = args.iter().position(|a| a == "--settings").unwrap();
        assert!(args[s_i + 1].ends_with("T-1-research-1700.settings.json"));
        let p_i = args.iter().position(|a| a == "--permission-mode").unwrap();
        assert_eq!(args[p_i + 1], "acceptEdits");
    }

    #[test]
    fn append_system_prompt_carries_the_prompt() {
        let args = build_args(&req());
        let i = args.iter().position(|a| a == "--append-system-prompt").unwrap();
        assert_eq!(args[i + 1], "You are research.");
    }
}
