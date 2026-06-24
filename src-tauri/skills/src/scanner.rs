//! `FsSkillScanner` (Task 2) — the real `SkillCatalog`. Knows the `.claude`
//! internal layout and walks it; returns `SkillEntry`s. Everything filesystem /
//! `SKILL.md`-shaped is sealed here.
//!
//! Per root it scans:
//!   - Plugin skills:   `<root>/plugins/cache/<mkt>/<plugin>/<version>/skills/<name>/SKILL.md`
//!   - Direct skills:   `<root>/skills/<name>/SKILL.md`
//!   - Plugin commands: `<root>/plugins/cache/.../<version>/commands/*.md`
//!   - Direct commands: `<root>/commands/*.md`
//!
//! Robustness: malformed/missing frontmatter → skip that entry (never crash);
//! a missing root → contributes nothing. Newest plugin version (by semver) wins.

use crate::model::{ClaudeRoot, SkillCatalog, SkillEntry, SkillKind, SkillSource};
use crate::verbs::verbs_from_router_table;
use std::path::Path;

/// The real filesystem-walking catalog.
#[derive(Default)]
pub struct FsSkillScanner;

impl FsSkillScanner {
    pub fn new() -> Self {
        Self
    }
}

impl SkillCatalog for FsSkillScanner {
    fn list(&self, roots: &[ClaudeRoot]) -> Vec<SkillEntry> {
        let mut out = Vec::new();
        for root in roots {
            scan_root(&root.path, root.source, &mut out);
        }
        out
    }
}

fn scan_root(root: &Path, source: SkillSource, out: &mut Vec<SkillEntry>) {
    if !root.is_dir() {
        return; // missing root contributes nothing, no error
    }
    scan_plugins(root, source, out);
    scan_direct_skills(&root.join("skills"), None, source, out);
    scan_command_dir(&root.join("commands"), None, source, out);
}

/// `<root>/plugins/cache/<marketplace>/<plugin>/<version>/{skills,commands}`.
fn scan_plugins(root: &Path, source: SkillSource, out: &mut Vec<SkillEntry>) {
    let cache = root.join("plugins").join("cache");
    let Ok(markets) = std::fs::read_dir(&cache) else {
        return;
    };
    for market in markets.flatten() {
        let Ok(plugins) = std::fs::read_dir(market.path()) else {
            continue;
        };
        for plugin in plugins.flatten() {
            let plugin_name = plugin.file_name().to_string_lossy().into_owned();
            // Pick the newest version dir for this plugin.
            let Some(version_dir) = newest_version_dir(&plugin.path()) else {
                continue;
            };
            scan_direct_skills(
                &version_dir.join("skills"),
                Some(&plugin_name),
                source,
                out,
            );
            scan_command_dir(&version_dir.join("commands"), Some(&plugin_name), source, out);
        }
    }
}

/// Return the path of the newest version subdir under `plugin_dir`, by semver.
/// Falls back to lexical order when a name isn't semver. None if no subdirs.
fn newest_version_dir(plugin_dir: &Path) -> Option<std::path::PathBuf> {
    let entries = std::fs::read_dir(plugin_dir).ok()?;
    let mut best: Option<(SemverKey, std::path::PathBuf)> = None;
    for e in entries.flatten() {
        if !e.path().is_dir() {
            continue;
        }
        let name = e.file_name().to_string_lossy().into_owned();
        let key = SemverKey::parse(&name);
        match &best {
            Some((bk, _)) if *bk >= key => {}
            _ => best = Some((key, e.path())),
        }
    }
    best.map(|(_, p)| p)
}

/// Scan a `skills/` dir: each `<name>/SKILL.md` → a Skill entry. `namespace` is
/// the plugin name (None for a direct/project-local skills dir).
fn scan_direct_skills(
    skills_dir: &Path,
    namespace: Option<&str>,
    source: SkillSource,
    out: &mut Vec<SkillEntry>,
) {
    let Ok(entries) = std::fs::read_dir(skills_dir) else {
        return;
    };
    for e in entries.flatten() {
        let skill_md = e.path().join("SKILL.md");
        if !skill_md.is_file() {
            continue;
        }
        let Ok(body) = std::fs::read_to_string(&skill_md) else {
            continue;
        };
        let Some((name, description)) = parse_frontmatter(&body) else {
            continue; // malformed/missing frontmatter → skip, never crash
        };
        // The frontmatter name is authoritative; fall back to the dir name.
        let name = if name.trim().is_empty() {
            e.file_name().to_string_lossy().into_owned()
        } else {
            name
        };
        let verbs = verbs_from_router_table(&body);
        out.push(SkillEntry {
            name,
            kind: SkillKind::Skill,
            namespace: namespace.map(|s| s.to_string()),
            description,
            verbs,
            source,
            qualified: false,
        });
    }
}

/// Scan a `commands/` dir: each `*.md` → a Command entry whose name is the file
/// stem. When `namespace` is Some (a plugin's commands), those command names are
/// ALSO surfaced as the namespacing plugin's verb set — but here we emit each as
/// its own Command entry (verb extraction priority #1 is handled at the plugin
/// level by the skill's sibling commands; a command itself has no sub-verbs).
fn scan_command_dir(
    commands_dir: &Path,
    namespace: Option<&str>,
    source: SkillSource,
    out: &mut Vec<SkillEntry>,
) {
    let Ok(entries) = std::fs::read_dir(commands_dir) else {
        return;
    };
    for e in entries.flatten() {
        let path = e.path();
        if path.extension().and_then(|x| x.to_str()) != Some("md") {
            continue;
        }
        let Some(stem) = path.file_stem().map(|s| s.to_string_lossy().into_owned()) else {
            continue;
        };
        // Frontmatter is optional for commands; description best-effort.
        let description = std::fs::read_to_string(&path)
            .ok()
            .and_then(|b| parse_frontmatter(&b).map(|(_, d)| d))
            .unwrap_or_default();
        out.push(SkillEntry {
            name: stem,
            kind: SkillKind::Command,
            namespace: namespace.map(|s| s.to_string()),
            description,
            verbs: vec![],
            source,
            qualified: false,
        });
    }
}

/// Parse a leading YAML frontmatter block (`---\n...\n---`) and pull `name` +
/// `description`. Returns None when there is no frontmatter or it doesn't parse
/// — the caller treats that as "skip this entry". A frontmatter block that
/// parses but lacks the keys yields empty strings (still a usable entry; the
/// caller falls back to the dir name).
fn parse_frontmatter(body: &str) -> Option<(String, String)> {
    let rest = body.strip_prefix("---")?;
    // The first newline after the opening fence, then up to the closing fence.
    let rest = rest.trim_start_matches(['\r', '\n']);
    let end = rest.find("\n---")?;
    let yaml = &rest[..end];
    let value: serde_yaml::Value = serde_yaml::from_str(yaml).ok()?;
    let name = value
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let description = value
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    Some((name, description))
}

/// A comparable semver key. Non-semver names sort below any semver (so a real
/// version always wins over a stray dir); ties broken lexically by the raw name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct SemverKey {
    parts: (u64, u64, u64),
    raw: String,
}

impl SemverKey {
    fn parse(name: &str) -> Self {
        let trimmed = name.trim_start_matches('v');
        let mut nums = trimmed.split(['.', '-', '+']).filter_map(|p| p.parse::<u64>().ok());
        let major = nums.next();
        let minor = nums.next().unwrap_or(0);
        let patch = nums.next().unwrap_or(0);
        // A name with no leading number sorts to the very bottom.
        let parts = match major {
            Some(m) => (m, minor, patch),
            None => (0, 0, 0),
        };
        SemverKey { parts, raw: name.to_string() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    struct TempRoot(PathBuf);
    impl TempRoot {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!("skills-fix-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&p).unwrap();
            TempRoot(p)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn plugin_skill(root: &Path, market: &str, plugin: &str, version: &str, name: &str, fm: &str) {
        let dir = root
            .join("plugins/cache")
            .join(market)
            .join(plugin)
            .join(version)
            .join("skills")
            .join(name);
        write(&dir.join("SKILL.md"), fm);
    }

    fn list(root: &Path, source: SkillSource) -> Vec<SkillEntry> {
        FsSkillScanner::new().list(&[ClaudeRoot { path: root.to_path_buf(), source }])
    }

    #[test]
    fn missing_root_contributes_nothing() {
        let got = FsSkillScanner::new().list(&[ClaudeRoot {
            path: PathBuf::from("/no/such/root/here"),
            source: SkillSource::Global,
        }]);
        assert!(got.is_empty());
    }

    #[test]
    fn scans_a_plugin_skill_with_frontmatter() {
        let root = TempRoot::new();
        plugin_skill(
            root.path(),
            "mkt",
            "superpowers",
            "1.0.0",
            "brainstorming",
            "---\nname: brainstorming\ndescription: Explore intent\n---\nbody\n",
        );
        let got = list(root.path(), SkillSource::Global);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name, "brainstorming");
        assert_eq!(got[0].namespace.as_deref(), Some("superpowers"));
        assert_eq!(got[0].description, "Explore intent");
        assert_eq!(got[0].kind, SkillKind::Skill);
        assert!(got[0].verbs.is_empty());
    }

    #[test]
    fn newest_plugin_version_wins() {
        let root = TempRoot::new();
        plugin_skill(root.path(), "mkt", "p", "1.0.0", "s", "---\nname: s\ndescription: old\n---\n");
        plugin_skill(root.path(), "mkt", "p", "1.2.0", "s", "---\nname: s\ndescription: new\n---\n");
        plugin_skill(root.path(), "mkt", "p", "1.10.0", "s", "---\nname: s\ndescription: newest\n---\n");
        let got = list(root.path(), SkillSource::Global);
        assert_eq!(got.len(), 1, "only newest version listed");
        assert_eq!(got[0].description, "newest");
    }

    #[test]
    fn scans_direct_skills_without_namespace() {
        let root = TempRoot::new();
        write(
            &root.path().join("skills/local-skill/SKILL.md"),
            "---\nname: local-skill\ndescription: project local\n---\n",
        );
        let got = list(root.path(), SkillSource::Project);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name, "local-skill");
        assert_eq!(got[0].namespace, None);
        assert_eq!(got[0].source, SkillSource::Project);
    }

    #[test]
    fn scans_user_commands_and_plugin_commands() {
        let root = TempRoot::new();
        write(&root.path().join("commands/init-session.md"), "do the thing\n");
        write(
            &root
                .path()
                .join("plugins/cache/mkt/tools/1.0.0/commands/deploy.md"),
            "---\ndescription: ship it\n---\n",
        );
        let mut got = list(root.path(), SkillSource::Global);
        got.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].name, "deploy");
        assert_eq!(got[0].kind, SkillKind::Command);
        assert_eq!(got[0].namespace.as_deref(), Some("tools"));
        assert_eq!(got[0].description, "ship it");
        assert_eq!(got[1].name, "init-session");
        assert_eq!(got[1].kind, SkillKind::Command);
        assert_eq!(got[1].namespace, None);
    }

    #[test]
    fn malformed_frontmatter_is_skipped_not_fatal() {
        let root = TempRoot::new();
        // good skill
        plugin_skill(root.path(), "mkt", "p", "1.0.0", "good", "---\nname: good\ndescription: ok\n---\n");
        // no frontmatter at all
        write(&root.path().join("skills/bad/SKILL.md"), "just text, no frontmatter\n");
        // broken yaml
        write(&root.path().join("skills/broken/SKILL.md"), "---\nname: [unterminated\n---\n");
        let got = list(root.path(), SkillSource::Global);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name, "good");
    }

    #[test]
    fn extracts_verbs_from_a_router_table_skill() {
        let root = TempRoot::new();
        plugin_skill(
            root.path(),
            "mkt",
            "ddd-council",
            "0.1.0",
            "ddd-council",
            "---\nname: ddd-council\ndescription: council\n---\n\n| Verb | Reference |\n| --- | --- |\n| vet | a.md |\n| critique | b.md |\n",
        );
        let got = list(root.path(), SkillSource::Global);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].verbs, vec!["vet".to_string(), "critique".into()]);
    }

    #[test]
    fn semver_key_orders_correctly() {
        assert!(SemverKey::parse("1.10.0") > SemverKey::parse("1.2.0"));
        assert!(SemverKey::parse("2.0.0") > SemverKey::parse("1.99.99"));
        assert!(SemverKey::parse("v1.0.0") > SemverKey::parse("not-semver"));
    }
}
