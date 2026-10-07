//! claude CLI argument construction. Pure: takes an InvocationRequest, returns
//! the argv vector. Kept separate from spawning so it is asserted exactly in
//! tests with no subprocess. Mirrors the spec's Worker model command line.

use crate::output::InvocationRequest;

/// The binary name. The composition root may override via PATH; v1 assumes
/// `claude` is resolvable.
pub const CLAUDE_BIN: &str = "claude";

/// Build the argv (excluding the program name) for one claude --print invocation.
///
/// The worker is isolated from the user's Claude Code setup
/// (`--setting-sources=` drops user and project settings, hooks and plugins;
/// `--strict-mcp-config` drops MCP servers) and has no one to answer a
/// permission prompt (`--permission-prompts none`). Every variadic flag uses
/// the `--flag=value` form as one element so it can never swallow the
/// positional prompt, which is always last. `--tools` is never passed: it
/// would remove `Skill`.
pub fn build_args(req: &InvocationRequest) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "--print".into(),
        "--output-format".into(),
        "stream-json".into(),
        // `claude --print --output-format stream-json` REQUIRES --verbose, or it
        // exits non-zero with empty stdout.
        "--verbose".into(),
        "--append-system-prompt".into(),
        req.system_prompt.clone(),
        "--settings".into(),
        req.settings_path.clone(),
        "--setting-sources=".into(),
        "--strict-mcp-config".into(),
        "--permission-prompts".into(),
        "none".into(),
        "--permission-mode".into(),
        req.permission_mode.as_cli().into(),
    ];
    for dir in &req.add_dirs {
        args.push(format!("--add-dir={dir}"));
    }
    for dir in &req.plugin_dirs {
        args.push(format!("--plugin-dir={dir}"));
    }
    if !req.disallowed_tools.is_empty() {
        args.push(format!("--disallowed-tools={}", req.disallowed_tools.join(",")));
    }
    args.push("--model".into());
    args.push(req.model.clone());
    if let Some(level) = req.effort.level() {
        args.push("--effort".into());
        args.push(level.to_string());
    }
    // The Structured output schema for this stage kind.
    args.push(format!(
        "--json-schema={}",
        serde_json::to_string(&req.output_kind.schema()).expect("a schema serialises")
    ));
    // The user message is the final positional argument.
    args.push(req.user_message.clone());
    args
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_bus_core::{Effort, OutputKind, PermissionMode};

    /// Flags whose value list is variadic in the CLI: given as a separate
    /// element, they would swallow the positional prompt.
    const VARIADIC: &[&str] = &["--add-dir", "--disallowed-tools", "--disallowedTools", "--allowed-tools", "--allowedTools", "--tools", "--mcp-config", "--plugin-dir"];

    fn req() -> InvocationRequest {
        InvocationRequest {
            task_id: "T-1".into(),
            team_id: "research".into(),
            model: "claude-opus-4-7".into(),
            effort: Effort::Level("high".into()),
            system_prompt: "You are research.".into(),
            user_message: "Investigate topic X".into(),
            settings_path: "/p/.agent-bus/runtime/T-1-research-1700.settings.json".into(),
            add_dirs: vec!["/repo".into(), "/p/artifacts/analyses".into()],
            permission_mode: PermissionMode::Auto,
            disallowed_tools: vec!["Bash".into(), "Agent".into(), "WebFetch".into()],
            plugin_dirs: vec!["/plugins/superpowers".into(), "/plugins/ddd council".into()],
            working_dir: None,
            output_kind: OutputKind::Reviewer,
        }
    }

    #[test]
    fn json_schema_is_one_equals_form_element_carrying_the_kind_schema() {
        let r = req();
        let args = build_args(&r);
        let flags: Vec<&String> = args.iter().filter(|a| a.starts_with("--json-schema")).collect();
        assert_eq!(flags.len(), 1, "exactly one --json-schema element");
        let tail = flags[0].strip_prefix("--json-schema=").expect("the = form, one argv element");
        assert!(!tail.contains('\n'), "compact JSON");
        let schema: serde_json::Value = serde_json::from_str(tail).unwrap();
        assert_eq!(schema, r.output_kind.schema());
        // the positional prompt survives
        assert_eq!(args.last().unwrap(), "Investigate topic X");
    }

    #[test]
    fn each_output_kind_passes_its_own_schema() {
        for kind in [OutputKind::Generator, OutputKind::Producer, OutputKind::Reviewer] {
            let mut r = req();
            r.output_kind = kind;
            let args = build_args(&r);
            let tail = args.iter().find_map(|a| a.strip_prefix("--json-schema=")).unwrap();
            assert_eq!(serde_json::from_str::<serde_json::Value>(tail).unwrap(), kind.schema());
        }
    }

    #[test]
    fn builds_the_spec_command_line() {
        let args = build_args(&req());
        assert_eq!(args[0], "--print");
        assert_eq!(args[1], "--output-format");
        assert_eq!(args[2], "stream-json");
        // --verbose is required for --print stream-json
        assert!(args.iter().any(|a| a == "--verbose"));
        // model + effort present
        let model_i = args.iter().position(|a| a == "--model").unwrap();
        assert_eq!(args[model_i + 1], "claude-opus-4-7");
        let effort_i = args.iter().position(|a| a == "--effort").unwrap();
        assert_eq!(args[effort_i + 1], "high");
        assert!(!args.iter().any(|a| a == "--max-thinking-tokens"));
        // user message is last
        assert_eq!(args.last().unwrap(), "Investigate topic X");
    }

    #[test]
    fn default_effort_passes_no_effort_flag() {
        let mut r = req();
        r.effort = Effort::Default;
        let args = build_args(&r);
        assert!(!args.iter().any(|a| a == "--effort"));
        assert!(!args.iter().any(|a| a == "--max-thinking-tokens"));
        assert_eq!(args.last().unwrap(), "Investigate topic X");
    }

    #[test]
    fn one_equals_form_add_dir_per_directory() {
        let args = build_args(&req());
        let dirs: Vec<&String> = args.iter().filter(|a| a.starts_with("--add-dir")).collect();
        assert_eq!(dirs, vec!["--add-dir=/repo", "--add-dir=/p/artifacts/analyses"]);
    }

    #[test]
    fn every_variadic_flag_uses_the_equals_form_and_the_prompt_survives() {
        let args = build_args(&req());
        for a in &args {
            assert!(!VARIADIC.contains(&a.as_str()), "bare variadic flag {a} in {args:?}");
        }
        assert_eq!(args.last().unwrap(), "Investigate topic X");
        assert_eq!(args.iter().filter(|a| *a == "Investigate topic X").count(), 1);
    }

    #[test]
    fn the_worker_is_isolated_and_has_no_prompt_surface() {
        let args = build_args(&req());
        assert!(args.iter().any(|a| a == "--setting-sources="));
        assert!(args.iter().any(|a| a == "--strict-mcp-config"));
        let i = args.iter().position(|a| a == "--permission-prompts").unwrap();
        assert_eq!(args[i + 1], "none");
        assert!(!args.iter().any(|a| a == "--bare" || a.starts_with("--tools")));
    }

    #[test]
    fn the_permission_mode_follows_the_request() {
        let mut r = req();
        let args = build_args(&r);
        let i = args.iter().position(|a| a == "--permission-mode").unwrap();
        assert_eq!(args[i + 1], "auto");
        r.permission_mode = PermissionMode::AcceptEdits;
        let args = build_args(&r);
        let i = args.iter().position(|a| a == "--permission-mode").unwrap();
        assert_eq!(args[i + 1], "acceptEdits");
    }

    #[test]
    fn disallowed_tools_is_one_element_listing_exactly_the_request() {
        let args = build_args(&req());
        let d: Vec<&String> = args.iter().filter(|a| a.starts_with("--disallowed-tools")).collect();
        assert_eq!(d, vec!["--disallowed-tools=Bash,Agent,WebFetch"]);
        let mut r = req();
        r.disallowed_tools = vec![];
        assert!(!build_args(&r).iter().any(|a| a.starts_with("--disallowed-tools")));
    }

    #[test]
    fn one_plugin_dir_per_plugin() {
        let args = build_args(&req());
        let p: Vec<&String> = args.iter().filter(|a| a.starts_with("--plugin-dir")).collect();
        assert_eq!(p, vec!["--plugin-dir=/plugins/superpowers", "--plugin-dir=/plugins/ddd council"]);
    }

    #[test]
    fn settings_path_present() {
        let args = build_args(&req());
        let s_i = args.iter().position(|a| a == "--settings").unwrap();
        assert!(args[s_i + 1].ends_with("T-1-research-1700.settings.json"));
    }

    #[test]
    fn append_system_prompt_carries_the_prompt() {
        let args = build_args(&req());
        let i = args.iter().position(|a| a == "--append-system-prompt").unwrap();
        assert_eq!(args[i + 1], "You are research.");
    }
}
