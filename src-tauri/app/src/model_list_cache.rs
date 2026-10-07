//! The model list's disk cache and refresh rule. The list belongs to the
//! installed CLI, not a project, so the cache lives in the app data dir as
//! `model-list.json`. Boot serves the cache (or the built-in list) at once; a
//! live fetch replaces it when it returns. A failed fetch keeps whatever list
//! is current and never blocks editing or running.

use agent_bus_core::{ModelList, ModelListSource, ModelSource};
use std::path::Path;
use std::sync::RwLock;

pub const CACHE_FILE: &str = "model-list.json";

/// Read the cache. Missing, unreadable or wrongly shaped files give `None`.
pub fn load(path: &Path) -> Option<ModelList> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut list: ModelList = serde_json::from_str(&text).ok()?;
    list.source = ModelListSource::Cached;
    Some(list)
}

/// Write the cache through a temp file so a crash never leaves half a file.
pub fn save(path: &Path, list: &ModelList) -> std::io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(list)?)?;
    std::fs::rename(&tmp, path)
}

/// The list to serve at boot: a non-empty cache, else the built-in list.
pub fn boot_list(cached: Option<ModelList>) -> ModelList {
    match cached {
        Some(l) if !l.models.is_empty() => l,
        _ => runners::curated_models::curated(),
    }
}

/// Fetch live; on a non-empty list, replace `current` and write the cache.
/// Returns whether the list changed. Blocking: run off the async runtime.
pub fn refresh_from(source: &dyn ModelSource, current: &RwLock<ModelList>, cache_path: &Path) -> bool {
    match source.fetch() {
        Ok(list) if !list.models.is_empty() => {
            if let Err(e) = save(cache_path, &list) {
                eprintln!("app: model list cache write failed: {e}");
            }
            if let Ok(mut cur) = current.write() {
                *cur = list;
            }
            true
        }
        Ok(_) => {
            eprintln!("app: model list query returned no models; keeping the current list");
            false
        }
        Err(e) => {
            eprintln!("app: model list query failed: {e}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_bus_core::{ModelList, ModelListSource};

    fn tempdir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("abp-model-cache-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn save_then_load_marks_the_list_cached() {
        let dir = tempdir();
        let path = dir.join("model-list.json");
        let mut live = runners::curated_models::curated();
        live.source = ModelListSource::Live;
        live.cli_version = Some("2.1.292".into());
        save(&path, &live).unwrap();
        let back = load(&path).unwrap();
        assert_eq!(back.source, ModelListSource::Cached);
        assert_eq!(back.cli_version.as_deref(), Some("2.1.292"));
        assert_eq!(back.models, live.models);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_missing_or_corrupt_cache_is_ignored() {
        let dir = tempdir();
        assert!(load(&dir.join("absent.json")).is_none());
        std::fs::write(dir.join("bad.json"), "{\"models\": [tru").unwrap();
        assert!(load(&dir.join("bad.json")).is_none());
        std::fs::write(dir.join("old.json"), "{\"models\": 3}").unwrap();
        assert!(load(&dir.join("old.json")).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn boot_list_prefers_the_cache_then_the_curated_list() {
        let mut cached = runners::curated_models::curated();
        cached.source = ModelListSource::Cached;
        cached.models.truncate(2);
        assert_eq!(boot_list(Some(cached.clone())), cached);
        assert_eq!(boot_list(None).source, ModelListSource::Curated);
    }

    #[test]
    fn an_empty_cached_list_falls_back_to_curated() {
        let empty = ModelList { models: vec![], source: ModelListSource::Cached, cli_version: None };
        assert_eq!(boot_list(Some(empty)).source, ModelListSource::Curated);
    }

    struct Fixed(Result<ModelList, String>);
    impl agent_bus_core::ModelSource for Fixed {
        fn fetch(&self) -> Result<ModelList, String> {
            self.0.clone()
        }
    }

    #[test]
    fn a_live_fetch_replaces_the_list_and_writes_the_cache() {
        let dir = tempdir();
        let path = dir.join("model-list.json");
        let current = std::sync::RwLock::new(runners::curated_models::curated());
        let mut live = runners::curated_models::curated();
        live.source = ModelListSource::Live;
        live.models.truncate(3);
        assert!(refresh_from(&Fixed(Ok(live.clone())), &current, &path));
        assert_eq!(*current.read().unwrap(), live);
        assert_eq!(load(&path).unwrap().models, live.models);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_failed_or_empty_fetch_keeps_the_current_list() {
        let dir = tempdir();
        let path = dir.join("model-list.json");
        let current = std::sync::RwLock::new(runners::curated_models::curated());
        assert!(!refresh_from(&Fixed(Err("model list query timed out".into())), &current, &path));
        let empty = ModelList { models: vec![], source: ModelListSource::Live, cli_version: None };
        assert!(!refresh_from(&Fixed(Ok(empty)), &current, &path));
        assert_eq!(current.read().unwrap().source, ModelListSource::Curated);
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}
