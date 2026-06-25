//! Directory listing for the Workspace context (G7). Workspace owns the project
//! filesystem layout, so it owns the in-app file/folder browser's backing read.
//! The `std::fs` directory-read idiom is sealed behind the `list_dir` OHS
//! command — only the `DirEntry` DTO crosses the boundary (mirrors the
//! `WorktreeEntry` seam in `worktree.rs` and the `SpawnFn`/`KeychainStore`
//! seams). Tolerant by contract: an unreadable directory surfaces a `String`
//! error, never a panic.

use crate::api::expand_tilde;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// One filesystem entry crossing the OHS. No `std::fs`/`PathBuf`/`DirEntry`
/// idiom leaks past this DTO. Owned solely by Workspace and published through
/// Workspace's own OHS (like `Project`/`WorktreeEntry`), so it is not a
/// cross-context kernel and does not belong in `agent_bus_core`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirEntry {
    /// The entry's base name (no path), e.g. "src".
    pub name: String,
    /// The entry's absolute path, e.g. "/Users/tim/proj/src".
    pub path: String,
    /// True when the entry is a directory (lazily expandable in the tree).
    pub is_dir: bool,
}

/// List the immediate children of `path` (tilde-expanded with the same
/// discipline as the rest of the Workspace OHS). Sorted dirs-first, then
/// case-insensitively by name. Tolerant: a missing/unreadable directory yields
/// `Err(String)`, never a panic; unreadable individual entries are skipped.
/// `home` is injected for testability (pure tilde expansion); the read is the
/// only IO. Sealed behind the OHS — this is the single place the fs read-dir
/// idiom executes.
pub fn list_dir_inner(path: &str, home: &str) -> Result<Vec<DirEntry>, String> {
    let expanded = expand_tilde(path, home);
    let dir = Path::new(&expanded);
    let read = std::fs::read_dir(dir).map_err(|e| format!("cannot read {expanded}: {e}"))?;

    let mut out = Vec::new();
    for entry in read.flatten() {
        // file_type() can fail on a broken symlink — skip it, don't abort the
        // whole listing (tolerant contract).
        let is_dir = match entry.file_type() {
            Ok(ft) => ft.is_dir(),
            Err(_) => continue,
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        let abs = entry.path().to_string_lossy().into_owned();
        out.push(DirEntry { name, path: abs, is_dir });
    }

    // Dirs first, then case-insensitive by name (stable browsing order).
    out.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(out)
}

fn home_dir() -> String {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_default()
}

/// OHS command (G7): list a directory's immediate children for the in-app
/// `FileTreePicker`. Tilde-expanded; dirs-first; tolerant (error, not panic).
/// The fs read-dir idiom never crosses out — only `DirEntry` does.
#[tauri::command(rename_all = "snake_case")]
pub async fn list_dir(path: String) -> Result<Vec<DirEntry>, String> {
    list_dir_inner(&path, &home_dir())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::fs;

    fn tmp() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("abp-ld-{}", uuid::Uuid::new_v4()))
    }

    #[test]
    fn lists_children_dirs_first_then_alpha() {
        let root = tmp();
        fs::create_dir_all(root.join("zeta")).unwrap();
        fs::create_dir_all(root.join("Alpha")).unwrap();
        fs::write(root.join("b.txt"), "x").unwrap();
        fs::write(root.join("A.txt"), "x").unwrap();

        let got = list_dir_inner(root.to_str().unwrap(), "").unwrap();
        let order: Vec<&str> = got.iter().map(|e| e.name.as_str()).collect();
        // dirs first (case-insensitive alpha), then files (case-insensitive alpha)
        assert_eq!(order, vec!["Alpha", "zeta", "A.txt", "b.txt"]);
        // is_dir is correct + path is absolute
        let alpha = got.iter().find(|e| e.name == "Alpha").unwrap();
        assert!(alpha.is_dir);
        assert!(alpha.path.ends_with("Alpha"));
        assert!(Path::new(&alpha.path).is_absolute());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn expands_tilde_against_injected_home() {
        let home = tmp();
        fs::create_dir_all(home.join("child")).unwrap();
        let got = list_dir_inner("~/child", home.to_str().unwrap()).unwrap();
        // "~/child" resolved under home and listed it (empty dir -> no entries,
        // but no error proves the expansion + read happened).
        assert!(got.is_empty());
        // and "~" itself lists the home dir's children
        let top = list_dir_inner("~", home.to_str().unwrap()).unwrap();
        assert!(top.iter().any(|e| e.name == "child" && e.is_dir));
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn unreadable_dir_is_an_error_not_a_panic() {
        let missing = tmp().join("does-not-exist");
        let err = list_dir_inner(missing.to_str().unwrap(), "").unwrap_err();
        assert!(err.contains("cannot read"));
    }

    #[test]
    fn dir_entry_wire_contract_matches_ts() {
        let e = DirEntry { name: "src".into(), path: "/p/src".into(), is_dir: true };
        let v = serde_json::to_value(&e).unwrap();
        let keys: BTreeSet<String> = v.as_object().unwrap().keys().cloned().collect();
        let want: BTreeSet<String> =
            ["name", "path", "is_dir"].iter().map(|s| s.to_string()).collect();
        assert_eq!(keys, want);
        assert!(v["name"].is_string());
        assert!(v["path"].is_string());
        assert!(v["is_dir"].is_boolean());
    }
}
