//! Instance folders. Ported from `BaseInstance`, `MinecraftInstance` (paths) and `InstanceList`.

use crate::settings::Settings;
use crate::{fs, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Instance {
    /// The folder name inside the instances directory.
    pub id: String,
    pub root: PathBuf,
    pub settings: Settings,
}

impl Instance {
    pub fn load(root: &Path) -> Result<Self> {
        let id = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        Ok(Instance { id, root: root.to_path_buf(), settings: Settings::load(&root.join("instance.cfg"))? })
    }

    pub fn name(&self) -> String {
        self.settings.get("name").unwrap_or("Unnamed Instance").to_string()
    }

    /// `.minecraft` is only used when it exists and `minecraft` does not.
    pub fn game_root(&self) -> PathBuf {
        let (mc, dot_mc) = (self.root.join("minecraft"), self.root.join(".minecraft"));
        if dot_mc.exists() && !mc.exists() {
            dot_mc
        } else {
            mc
        }
    }

    pub fn natives_dir(&self) -> PathBuf {
        self.root.join("natives")
    }

    /// Where libraries with the `local` hint live.
    pub fn local_library_dir(&self) -> PathBuf {
        self.root.join("libraries")
    }

    pub fn mods_dir(&self) -> PathBuf {
        self.game_root().join("mods")
    }

    pub fn resources_dir(&self) -> PathBuf {
        self.game_root().join("resources")
    }
}

/// Every sub-folder that has an `instance.cfg`, sorted by name.
pub fn load_instances(dir: &Path) -> Result<Vec<Instance>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut instances = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(fs::io_err(dir))? {
        let path = entry.map_err(fs::io_err(dir))?.path();
        if path.join("instance.cfg").is_file() {
            match Instance::load(&path) {
                Ok(instance) => instances.push(instance),
                Err(e) => tracing::warn!("skipping instance {}: {e}", path.display()),
            }
        }
    }
    instances.sort_by_key(|i| i.name().to_lowercase());
    Ok(instances)
}

/// Creates the folder and `instance.cfg`. The caller writes `mmc-pack.json`.
pub fn create_instance(dir: &Path, name: &str) -> Result<Instance> {
    fs::create_dir_all(dir)?;
    let root = dir.join(fs::dir_name_from_string(name, dir)?);
    fs::create_dir_all(&root)?;
    let mut instance = Instance::load(&root)?;
    instance.settings.set("name", name);
    instance.settings.set("iconKey", "default");
    instance.settings.set("InstanceType", "OneSix");
    instance.settings.save()?;
    Ok(instance)
}

/// Instance id to group name, from `instgroups.json`. A missing or broken file means no groups.
pub fn load_groups(dir: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Ok(root) = fs::read_json::<serde_json::Value>(&dir.join("instgroups.json")) else { return out };
    let Some(groups) = root.get("groups").and_then(|g| g.as_object()) else { return out };
    for (group, body) in groups {
        let ids = body.get("instances").and_then(|i| i.as_array());
        for id in ids.into_iter().flatten().filter_map(|id| id.as_str()) {
            out.insert(id.to_string(), group.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_then_list() {
        let dir = tempfile::tempdir().expect("tempdir");
        let first = create_instance(dir.path(), "My: World").expect("create");
        let second = create_instance(dir.path(), "My: World").expect("create");
        assert_eq!(first.id, "My- World");
        assert_eq!(second.id, "My- World(1)");
        std::fs::create_dir(dir.path().join("not-an-instance")).expect("mkdir");

        let listed = load_instances(dir.path()).expect("list");
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].name(), "My: World");
        assert_eq!(listed[0].settings.get("InstanceType"), Some("OneSix"));
    }

    #[test]
    fn game_root_prefers_minecraft_unless_only_dot_minecraft_exists() {
        let dir = tempfile::tempdir().expect("tempdir");
        let instance = create_instance(dir.path(), "a").expect("create");
        assert!(instance.game_root().ends_with("minecraft"));
        std::fs::create_dir(instance.root.join(".minecraft")).expect("mkdir");
        assert!(instance.game_root().ends_with(".minecraft"));
    }

    #[test]
    fn groups_map_instances() {
        let dir = tempfile::tempdir().expect("tempdir");
        let json = r#"{"formatVersion":"1","groups":{"Modded":{"hidden":false,"instances":["a","b"]}}}"#;
        std::fs::write(dir.path().join("instgroups.json"), json).expect("write");
        let groups = load_groups(dir.path());
        assert_eq!(groups.get("a").map(String::as_str), Some("Modded"));
        assert_eq!(groups.len(), 2);
    }
}
