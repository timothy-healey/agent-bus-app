//! The `initialize` control request: the models the installed `claude` CLI
//! offers and the effort levels each supports, read without calling a model.
//! The only code that knows the request and response shape.

use crate::control_request::{self, ControlError};
use agent_bus_core::{ModelList, ModelListSource, ModelOption, ModelSource};
use serde_json::Value;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The control request line written to `claude`'s stdin.
pub const INITIALIZE_REQUEST: &str =
    "{\"type\":\"control_request\",\"request_id\":\"m1\",\"request\":{\"subtype\":\"initialize\"}}\n";

fn opt_str(v: &Value) -> Option<String> {
    v.as_str().map(str::to_string)
}

/// Parse `claude`'s stdout for the `initialize` control response's models. An
/// entry without a `value` or `displayName` is skipped; a missing
/// `supportedEffortLevels` means the model takes no effort level.
pub fn parse_initialize(stdout: &str) -> Result<Vec<ModelOption>, String> {
    for line in stdout.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        if v["type"] != "control_response" {
            continue;
        }
        let resp = &v["response"];
        if resp["subtype"] == "error" {
            return Err(resp["error"].as_str().unwrap_or("initialize request failed").to_string());
        }
        let models = resp["response"]["models"].as_array().ok_or("initialize response has no models")?;
        return Ok(models
            .iter()
            .filter_map(|m| {
                Some(ModelOption {
                    value: m["value"].as_str()?.to_string(),
                    resolved_model: opt_str(&m["resolvedModel"]),
                    display_name: m["displayName"].as_str()?.to_string(),
                    description: opt_str(&m["description"]),
                    effort_levels: m["supportedEffortLevels"]
                        .as_array()
                        .map(|a| a.iter().filter_map(opt_str).collect())
                        .unwrap_or_default(),
                    supports_auto_mode: m["supportsAutoMode"].as_bool().unwrap_or(false),
                })
            })
            .collect());
    }
    Err("no initialize response from claude".into())
}

/// `claude --version` prints e.g. `2.1.292 (Claude Code)`; the first token is
/// the version.
pub fn parse_cli_version(stdout: &str) -> Option<String> {
    stdout.lines().next()?.split_whitespace().next().map(str::to_string)
}

/// The installed CLI's version, or `None` if it cannot be read in time.
pub fn cli_version(bin: &str, timeout: Duration) -> Option<String> {
    let mut child = Command::new(bin)
        .arg("--version")
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => return None,
        }
    }
    let mut out = String::new();
    use std::io::Read;
    child.stdout.take()?.read_to_string(&mut out).ok()?;
    parse_cli_version(&out)
}

/// Production source: asks the installed `claude` for its model list.
pub struct ClaudeCliModelSource {
    bin: String,
    timeout: Duration,
}

impl ClaudeCliModelSource {
    pub fn new() -> Self {
        Self::with_bin(crate::command::CLAUDE_BIN, Duration::from_secs(10))
    }

    pub fn with_bin(bin: impl Into<String>, timeout: Duration) -> Self {
        Self { bin: bin.into(), timeout }
    }
}

impl Default for ClaudeCliModelSource {
    fn default() -> Self {
        Self::new()
    }
}

impl ModelSource for ClaudeCliModelSource {
    fn fetch(&self) -> Result<ModelList, String> {
        let cli_version = cli_version(&self.bin, self.timeout);
        let out = control_request::run(&self.bin, INITIALIZE_REQUEST, self.timeout).map_err(|e| match e {
            ControlError::TimedOut => "model list query timed out".to_string(),
            ControlError::Spawn(e) => format!("could not start claude: {e}"),
        })?;
        let models = parse_initialize(&out)?;
        Ok(ModelList { models, source: ModelListSource::Live, cli_version })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_bus_core::{ModelListSource, ModelOption, ModelSource};
    use std::time::Duration;

    const SAMPLE: &str = include_str!("fixtures/initialize-sample.jsonl");

    #[test]
    fn fixture_gives_twelve_models_with_their_levels() {
        let models = parse_initialize(SAMPLE).unwrap();
        assert_eq!(models.len(), 12);
        let by = |v: &str| models.iter().find(|m| m.value == v).unwrap().clone();
        assert_eq!(by("default").resolved_model.as_deref(), Some("claude-opus-5-5"));
        assert_eq!(by("default").effort_levels, ["low", "medium", "high", "xhigh", "max"]);
        assert!(by("haiku").effort_levels.is_empty());
        assert_eq!(by("claude-opus-4-6").effort_levels, ["low", "medium", "high", "max"]);
        assert_eq!(by("opus").display_name, "Opus 5.5");
        assert!(by("sonnet").description.is_some());
        assert!(by("sonnet").supports_auto_mode);
        assert!(by("default").supports_auto_mode);
        assert!(!by("haiku").supports_auto_mode);
    }

    #[test]
    fn the_curated_list_agrees_with_the_capture_on_auto_mode() {
        let live = parse_initialize(SAMPLE).unwrap();
        let curated = crate::curated_models::curated();
        for m in &live {
            let c = curated.find(&m.value).unwrap_or_else(|| panic!("curated lacks {}", m.value));
            assert_eq!(c.supports_auto_mode, m.supports_auto_mode, "{}", m.value);
        }
    }

    #[test]
    fn fixture_is_redacted() {
        assert!(SAMPLE.contains("\"email\":\"user@example.com\""));
        assert!(SAMPLE.contains("\"organization\":\"example-org\""));
        assert!(!SAMPLE.contains("/Users/"));
    }

    #[test]
    fn an_error_response_is_rejected() {
        let s = r#"{"type":"control_response","response":{"subtype":"error","request_id":"m1","error":"Unsupported control request subtype: initialize"}}"#;
        assert_eq!(parse_initialize(s).unwrap_err(), "Unsupported control request subtype: initialize");
    }

    #[test]
    fn a_response_without_models_is_rejected() {
        let s = r#"{"type":"control_response","response":{"subtype":"success","request_id":"m1","response":{"commands":[]}}}"#;
        assert_eq!(parse_initialize(s).unwrap_err(), "initialize response has no models");
    }

    #[test]
    fn an_entry_without_a_value_is_skipped() {
        let s = r#"{"type":"control_response","response":{"subtype":"success","request_id":"m1","response":{"models":[{"displayName":"x"},{"value":"haiku","displayName":"Haiku"}]}}}"#;
        let models = parse_initialize(s).unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].value, "haiku");
        assert_eq!(models[0].resolved_model, None);
        assert!(models[0].effort_levels.is_empty());
    }

    #[test]
    fn no_control_response_is_an_error() {
        assert_eq!(parse_initialize("").unwrap_err(), "no initialize response from claude");
    }

    #[test]
    fn version_line_is_parsed() {
        assert_eq!(parse_cli_version("2.1.292 (Claude Code)\n").as_deref(), Some("2.1.292"));
        assert_eq!(parse_cli_version(""), None);
    }

    #[test]
    fn curated_list_matches_the_fixture() {
        let live = parse_initialize(SAMPLE).unwrap();
        let curated = crate::curated_models::curated();
        assert_eq!(curated.source, ModelListSource::Curated);
        assert_eq!(curated.cli_version, None);
        let values = |ms: &[ModelOption]| {
            ms.iter().map(|m| (m.value.clone(), m.resolved_model.clone(), m.effort_levels.clone())).collect::<Vec<_>>()
        };
        assert_eq!(values(&curated.models), values(&live));
    }

    #[cfg(unix)]
    fn script(body: &str) -> (std::path::PathBuf, String) {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("abp-models-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fake-claude");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        (dir, path.to_string_lossy().into_owned())
    }

    #[cfg(unix)]
    fn fixture_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fixtures/initialize-sample.jsonl")
    }

    #[cfg(unix)]
    #[test]
    fn cli_source_reads_models_and_version_from_the_binary() {
        let (dir, bin) = script(&format!(
            "if [ \"$1\" = \"--version\" ]; then echo '9.9.9 (Claude Code)'; exit 0; fi\ncat > /dev/null\ncat '{}'",
            fixture_path().display()
        ));
        let list = ClaudeCliModelSource::with_bin(bin, Duration::from_secs(5)).fetch().unwrap();
        assert_eq!(list.source, ModelListSource::Live);
        assert_eq!(list.cli_version.as_deref(), Some("9.9.9"));
        assert_eq!(list.models.len(), 12);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn cli_source_returns_on_the_control_response_without_waiting_for_exit() {
        let (dir, bin) = script(&format!(
            "if [ \"$1\" = \"--version\" ]; then echo '9.9.9'; exit 0; fi\ncat '{}'\nsleep 30",
            fixture_path().display()
        ));
        let started = std::time::Instant::now();
        let list = ClaudeCliModelSource::with_bin(bin, Duration::from_secs(10)).fetch().unwrap();
        assert_eq!(list.models.len(), 12);
        assert!(started.elapsed() < Duration::from_secs(3), "took {:?}", started.elapsed());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn cli_source_times_out_a_hung_binary() {
        let (dir, bin) = script("if [ \"$1\" = \"--version\" ]; then echo '9.9.9'; exit 0; fi\nsleep 30");
        let started = std::time::Instant::now();
        let err = ClaudeCliModelSource::with_bin(bin, Duration::from_millis(300)).fetch().unwrap_err();
        assert_eq!(err, "model list query timed out");
        assert!(started.elapsed() < Duration::from_secs(5));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_hung_version_query_gives_no_version() {
        let (dir, bin) = script("sleep 30");
        let started = std::time::Instant::now();
        assert_eq!(cli_version(&bin, Duration::from_millis(300)), None);
        assert!(started.elapsed() < Duration::from_secs(5));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn cli_source_sends_the_initialize_request_with_the_control_args() {
        let (dir, bin) = script("");
        let record = dir.join("record");
        std::fs::write(
            &bin,
            format!(
                "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo '9.9.9'; exit 0; fi\nfor a in \"$@\"; do echo \"arg:$a\"; done > '{r}'\necho \"cwd:$(pwd -P)\" >> '{r}'\nhead -n 1 >> '{r}'\ncat '{f}'\n",
                r = record.display(),
                f = fixture_path().display()
            ),
        )
        .unwrap();
        ClaudeCliModelSource::with_bin(bin, Duration::from_secs(5)).fetch().unwrap();
        let rec = std::fs::read_to_string(&record).unwrap();
        let args: Vec<&str> = rec.lines().filter_map(|l| l.strip_prefix("arg:")).collect();
        assert_eq!(args, crate::control_request::CONTROL_ARGS);
        let cwd = rec.lines().find_map(|l| l.strip_prefix("cwd:")).unwrap();
        assert_eq!(std::path::Path::new(cwd), std::env::temp_dir().canonicalize().unwrap());
        assert!(rec.contains(r#"{"type":"control_request","request_id":"m1","request":{"subtype":"initialize"}}"#));
        let _ = std::fs::remove_dir_all(dir);
    }
}
