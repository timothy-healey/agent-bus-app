//! Plugin resolution: turn a plugin name a team declares into the directory
//! `--plugin-dir` loads. Reads (never writes) the Claude config root's
//! `plugins/installed_plugins.json` and `plugins/known_marketplaces.json`. The
//! root is injected so tests never touch the user's own config.

use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// The Claude config root: `$CLAUDE_CONFIG_DIR`, else `~/.claude`.
pub fn claude_config_root() -> PathBuf {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    home.join(".claude")
}

/// A plugin the declared name could not be resolved to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginNotFound {
    pub name: String,
}

impl std::fmt::Display for PluginNotFound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "plugin '{}' was not found", self.name)
    }
}

impl std::error::Error for PluginNotFound {}

/// One plugin the editor can offer: its name, its marketplace and where it lives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PluginInfo {
    pub name: String,
    pub marketplace: String,
    pub path: String,
}

/// Resolves plugin names against one Claude config root.
#[derive(Debug, Clone)]
pub struct PluginResolver {
    root: PathBuf,
}

impl PluginResolver {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn read_json(&self, file: &str) -> Option<Value> {
        let text = std::fs::read_to_string(self.root.join("plugins").join(file)).ok()?;
        serde_json::from_str(&text).ok()
    }

    /// `(name, marketplace, installPath)` for every installed plugin, sorted.
    fn installed(&self) -> Vec<(String, String, PathBuf)> {
        let mut out = Vec::new();
        let Some(v) = self.read_json("installed_plugins.json") else { return out };
        let Some(plugins) = v.get("plugins").and_then(Value::as_object) else { return out };
        for (key, entries) in plugins {
            let (name, market) = split_key(key);
            let path = entries
                .as_array()
                .and_then(|a| a.iter().find_map(|e| e.get("installPath").and_then(Value::as_str)))
                .map(PathBuf::from);
            if let Some(path) = path {
                out.push((name.to_string(), market.to_string(), path));
            }
        }
        out.sort();
        out
    }

    /// `(marketplace, directory)` for every marketplace whose source is a local
    /// directory, sorted.
    fn directory_marketplaces(&self) -> Vec<(String, PathBuf)> {
        let mut out = Vec::new();
        let Some(v) = self.read_json("known_marketplaces.json") else { return out };
        let Some(markets) = v.as_object() else { return out };
        for (market, m) in markets {
            let source = m.get("source");
            if source.and_then(|s| s.get("source")).and_then(Value::as_str) != Some("directory") {
                continue;
            }
            if let Some(dir) = source.and_then(|s| s.get("path")).and_then(Value::as_str) {
                out.push((market.clone(), PathBuf::from(dir)));
            }
        }
        out.sort();
        out
    }

    /// The plugins a directory marketplace offers, as `(name, dir)`. Read from
    /// its `.claude-plugin/marketplace.json`; without one, the directory itself
    /// is a single plugin named after the marketplace.
    fn marketplace_plugins(market: &str, dir: &Path) -> Vec<(String, PathBuf)> {
        let manifest = dir.join(".claude-plugin").join("marketplace.json");
        let Some(v) = std::fs::read_to_string(&manifest).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) else {
            return vec![(market.to_string(), dir.to_path_buf())];
        };
        let mut out = Vec::new();
        for p in v.get("plugins").and_then(Value::as_array).into_iter().flatten() {
            let Some(name) = p.get("name").and_then(Value::as_str) else { continue };
            // A string source is a path relative to the marketplace directory;
            // other source kinds are remote and have no local directory here.
            let Some(src) = p.get("source").and_then(Value::as_str) else { continue };
            let path = if Path::new(src).is_absolute() { PathBuf::from(src) } else { dir.join(src) };
            out.push((name.to_string(), normalise(&path)));
        }
        out
    }

    /// Resolve `name` (a plugin name or `name@marketplace`) to its directory.
    /// The installed copy wins when its folder exists; otherwise a directory
    /// marketplace that offers the plugin is used.
    pub fn resolve(&self, declared: &str) -> Result<PathBuf, PluginNotFound> {
        let not_found = || PluginNotFound { name: declared.to_string() };
        let (name, market) = split_key(declared.trim());
        if name.is_empty() {
            return Err(not_found());
        }
        let market = (!market.is_empty()).then_some(market);
        let matches = |n: &str, m: &str| n == name && market.is_none_or(|want| want == m);

        for (n, m, path) in self.installed() {
            if matches(&n, &m) && path.is_dir() {
                return Ok(path);
            }
        }
        for (m, dir) in self.directory_marketplaces() {
            for (n, path) in Self::marketplace_plugins(&m, &dir) {
                if matches(&n, &m) && path.is_dir() {
                    return Ok(path);
                }
            }
        }
        Err(not_found())
    }

    /// Every plugin a team can declare: the installed ones whose folder exists
    /// plus those offered by directory marketplaces. Deduped by
    /// `name@marketplace`, sorted by name.
    pub fn list(&self) -> Vec<PluginInfo> {
        let mut out: Vec<PluginInfo> = Vec::new();
        let mut push = |name: String, marketplace: String, path: PathBuf| {
            if !path.is_dir() || out.iter().any(|p| p.name == name && p.marketplace == marketplace) {
                return;
            }
            out.push(PluginInfo { name, marketplace, path: path.to_string_lossy().into_owned() });
        };
        for (n, m, path) in self.installed() {
            push(n, m, path);
        }
        for (m, dir) in self.directory_marketplaces() {
            for (n, path) in Self::marketplace_plugins(&m, &dir) {
                push(n, m.clone(), path);
            }
        }
        out.sort_by(|a, b| (&a.name, &a.marketplace).cmp(&(&b.name, &b.marketplace)));
        out
    }
}

/// Split `name@marketplace`; a bare name has an empty marketplace.
fn split_key(key: &str) -> (&str, &str) {
    key.split_once('@').unwrap_or((key, ""))
}

fn normalise(p: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn root() -> PathBuf {
        let r = std::env::temp_dir().join(format!("abp-plugins-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(r.join("plugins")).unwrap();
        r
    }

    fn write(path: &Path, v: Value) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, serde_json::to_string(&v).unwrap()).unwrap();
    }

    /// A config root with superpowers installed (folder present), a ddd-council
    /// install whose cache folder is missing, and ddd-council's directory
    /// marketplace.
    fn fixture() -> (PathBuf, PathBuf, PathBuf) {
        let r = root();
        let sp = r.join("plugins/cache/official/superpowers/6.4.1");
        std::fs::create_dir_all(&sp).unwrap();
        let council = r.join("src/ddd-council");
        std::fs::create_dir_all(council.join("skills")).unwrap();
        write(
            &council.join(".claude-plugin/marketplace.json"),
            json!({"name": "ddd-council", "plugins": [{"name": "ddd-council", "source": "./"}]}),
        );
        write(
            &r.join("plugins/installed_plugins.json"),
            json!({"version": 2, "plugins": {
                "superpowers@official": [{"scope": "user", "installPath": sp.to_string_lossy(), "version": "6.4.1"}],
                "ddd-council@ddd-council": [{"scope": "user", "installPath": r.join("plugins/cache/ddd-council/ddd-council/0.1.0").to_string_lossy(), "version": "0.1.0"}]
            }}),
        );
        write(
            &r.join("plugins/known_marketplaces.json"),
            json!({
                "official": {"source": {"source": "github", "repo": "example/official"}, "installLocation": r.join("plugins/marketplaces/official").to_string_lossy()},
                "ddd-council": {"source": {"source": "directory", "path": council.to_string_lossy()}, "installLocation": council.to_string_lossy()}
            }),
        );
        (r, sp, council)
    }

    #[test]
    fn a_plugin_resolves_to_its_install_path() {
        let (r, sp, _) = fixture();
        let res = PluginResolver::new(&r);
        assert_eq!(res.resolve("superpowers").unwrap(), sp);
        assert_eq!(res.resolve("superpowers@official").unwrap(), sp);
    }

    #[test]
    fn a_missing_cache_folder_falls_back_to_the_directory_marketplace() {
        let (r, _, council) = fixture();
        let res = PluginResolver::new(&r);
        assert_eq!(res.resolve("ddd-council").unwrap(), council);
        assert_eq!(res.resolve("ddd-council@ddd-council").unwrap(), council);
    }

    #[test]
    fn a_directory_marketplace_without_a_manifest_is_one_plugin() {
        let r = root();
        let dir = r.join("solo");
        std::fs::create_dir_all(&dir).unwrap();
        write(
            &r.join("plugins/known_marketplaces.json"),
            json!({"solo": {"source": {"source": "directory", "path": dir.to_string_lossy()}}}),
        );
        assert_eq!(PluginResolver::new(&r).resolve("solo").unwrap(), dir);
    }

    #[test]
    fn an_unknown_plugin_or_the_wrong_marketplace_is_not_found() {
        let (r, _, _) = fixture();
        let res = PluginResolver::new(&r);
        assert_eq!(res.resolve("nope"), Err(PluginNotFound { name: "nope".into() }));
        assert!(res.resolve("superpowers@elsewhere").is_err());
        assert!(res.resolve("").is_err());
    }

    #[test]
    fn a_root_without_plugin_files_resolves_nothing() {
        let r = root();
        let res = PluginResolver::new(&r);
        assert!(res.resolve("superpowers").is_err());
        assert!(res.list().is_empty());
    }

    #[test]
    fn list_merges_installed_and_directory_plugins() {
        let (r, sp, council) = fixture();
        let l = PluginResolver::new(&r).list();
        assert_eq!(
            l,
            vec![
                PluginInfo { name: "ddd-council".into(), marketplace: "ddd-council".into(), path: council.to_string_lossy().into_owned() },
                PluginInfo { name: "superpowers".into(), marketplace: "official".into(), path: sp.to_string_lossy().into_owned() },
            ]
        );
    }

    #[test]
    fn the_config_root_defaults_under_home() {
        // Only the shape is checked; the environment is not modified.
        let r = claude_config_root();
        if std::env::var_os("CLAUDE_CONFIG_DIR").is_none() {
            assert!(r.ends_with(".claude"), "{r:?}");
        }
    }
}
