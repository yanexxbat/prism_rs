//! `mmc-pack.json`: the ordered component list of an instance.
//! Ported from `PackProfile`, `Component` and `ComponentUpdateTask`.

use crate::meta::Meta;
use crate::version_file::{valid_uid, LaunchProfile, Require, RuntimeContext, VersionFile};
use crate::{Error, Result};
use prism_core::version::Version;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn is_false(value: &bool) -> bool {
    !*value
}

/// Field order is alphabetical, matching what the original writes.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Component {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cached_conflicts: Vec<Require>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub cached_name: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cached_requires: Vec<Require>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub cached_version: String,
    #[serde(skip_serializing_if = "is_false")]
    pub cached_volatile: bool,
    /// Added to satisfy another component, not chosen by the user.
    #[serde(skip_serializing_if = "is_false")]
    pub dependency_only: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub disabled: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub important: bool,
    pub uid: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub version: String,

    /// Loaded by `resolve`.
    #[serde(skip)]
    pub file: Option<VersionFile>,
    /// Comes from `patches/<uid>.json` instead of the metadata server.
    #[serde(skip)]
    pub custom: bool,
}

impl Component {
    pub fn name(&self) -> &str {
        if self.cached_name.is_empty() {
            &self.uid
        } else {
            &self.cached_name
        }
    }

    /// Minecraft itself and dependencies cannot be switched off.
    fn is_enabled(&self) -> bool {
        let can_be_disabled = self.uid != "net.minecraft" && !self.dependency_only;
        !can_be_disabled || !self.disabled
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PackFile {
    components: Vec<Component>,
    format_version: i64,
}

struct Requirement {
    equals: String,
    suggests: String,
    first_dependee: usize,
}

pub struct PackProfile {
    instance_root: PathBuf,
    pub components: Vec<Component>,
}

impl PackProfile {
    /// For a new instance.
    pub fn new(instance_root: &Path) -> Self {
        PackProfile { instance_root: instance_root.to_path_buf(), components: Vec::new() }
    }

    pub fn load(instance_root: &Path) -> Result<Self> {
        let path = instance_root.join("mmc-pack.json");
        let file: PackFile = prism_core::fs::read_json(&path)?;
        if file.format_version != 1 {
            return Err(Error::Dependencies(format!("{}: invalid component file version, expected 1", path.display())));
        }
        let mut components: Vec<Component> = Vec::new();
        for component in file.components {
            if !valid_uid(&component.uid) {
                return Err(Error::BadComponent(component.uid));
            }
            if components.iter().any(|c| c.uid == component.uid) {
                tracing::warn!("ignoring duplicate component entry {}", component.uid);
                continue;
            }
            components.push(component);
        }
        Ok(PackProfile { instance_root: instance_root.to_path_buf(), components })
    }

    pub fn save(&self) -> Result<()> {
        let path = self.instance_root.join("mmc-pack.json");
        let file = PackFile { components: self.components.clone(), format_version: 1 };
        // Four spaces, like Qt's indented JSON.
        let mut out = Vec::new();
        let formatter = serde_json::ser::PrettyFormatter::with_indent(b"    ");
        let mut serializer = serde_json::Serializer::with_formatter(&mut out, formatter);
        file.serialize(&mut serializer).map_err(|source| prism_core::Error::Json { path: path.clone(), source })?;
        out.push(b'\n');
        Ok(prism_core::fs::write_atomic(&path, &out)?)
    }

    pub fn component(&self, uid: &str) -> Option<&Component> {
        self.components.iter().find(|c| c.uid == uid)
    }

    /// Sets the version of a component, adding it at the end when it is new.
    /// Call `resolve` afterwards.
    pub fn set_component_version(&mut self, uid: &str, version: &str, important: bool) {
        match self.components.iter_mut().find(|c| c.uid == uid) {
            Some(existing) => {
                existing.version = version.to_string();
                existing.important = important;
                existing.file = None;
            }
            None => self.components.push(Component {
                uid: uid.to_string(),
                version: version.to_string(),
                important,
                ..Component::default()
            }),
        }
    }

    pub fn remove(&mut self, uid: &str) {
        self.components.retain(|c| c.uid != uid);
    }

    async fn load_components(&mut self, meta: &Meta, offline: bool) -> Result<()> {
        for component in self.components.iter_mut().filter(|c| c.file.is_none()) {
            let patch = self.instance_root.join("patches").join(format!("{}.json", component.uid));
            let mut file = if patch.is_file() {
                component.custom = true;
                let data = std::fs::read(&patch).map_err(crate::io_err(&patch))?;
                VersionFile::from_json(&data, &patch.to_string_lossy())?
            } else {
                meta.version(&component.uid, &component.version, None, offline).await?
            };
            file.uid = component.uid.clone();
            component.cached_name = file.name.clone();
            component.cached_version = file.version.clone();
            component.cached_volatile = file.volatile;
            component.cached_requires = file.requires.clone();
            component.cached_conflicts = file.conflicts.clone();
            component.file = Some(file);
        }
        Ok(())
    }

    /// Collects what the components require of each other. Two different exact versions
    /// for the same component are an error.
    fn gather_requirements(&self) -> Result<BTreeMap<String, Requirement>> {
        let mut all: BTreeMap<String, Requirement> = BTreeMap::new();
        for (index, component) in self.components.iter().enumerate() {
            for require in &component.cached_requires {
                let new = Requirement {
                    equals: require.equals.clone(),
                    suggests: require.suggests.clone(),
                    first_dependee: index,
                };
                let merged = match all.remove(&require.uid) {
                    None => new,
                    Some(old) => {
                        if !old.equals.is_empty() && !new.equals.is_empty() && old.equals != new.equals {
                            return Err(Error::Dependencies(format!(
                                "Conflicting requirements detected during dependency checking! {} must be {} and {}",
                                require.uid, old.equals, new.equals
                            )));
                        }
                        let newer = Version::new(&old.suggests) < Version::new(&new.suggests);
                        Requirement {
                            equals: if old.equals.is_empty() { new.equals } else { old.equals },
                            suggests: if old.suggests.is_empty() || (!new.suggests.is_empty() && newer) {
                                new.suggests
                            } else {
                                old.suggests
                            },
                            first_dependee: old.first_dependee.min(index),
                        }
                    }
                };
                all.insert(require.uid.clone(), merged);
            }
        }
        Ok(all)
    }

    /// Loads every component and makes the list consistent.
    ///
    /// With `check_only` (the launch path) a missing or mismatching dependency is an
    /// error. Without it, missing dependencies are inserted before their first dependee
    /// and dependency-only components follow the version their dependee demands.
    pub async fn resolve(&mut self, meta: &Meta, offline: bool, check_only: bool) -> Result<()> {
        loop {
            self.load_components(meta, offline).await?;

            // Drop auto-added volatile components nothing asks for any more.
            let requirements = loop {
                let requirements = self.gather_requirements()?;
                let before = self.components.len();
                self.components
                    .retain(|c| !(c.dependency_only && c.cached_volatile && !requirements.contains_key(&c.uid)));
                if self.components.len() == before {
                    break requirements;
                }
            };

            let mut to_add = Vec::new();
            let mut to_change = Vec::new();
            for (uid, requirement) in requirements {
                match self.components.iter().position(|c| c.uid == uid) {
                    None => to_add.push((uid, requirement)),
                    Some(_) if requirement.equals.is_empty() => {}
                    Some(index) if self.components[index].version == requirement.equals => {}
                    Some(index) if self.components[index].dependency_only && !self.components[index].custom => {
                        to_change.push((index, requirement.equals));
                    }
                    Some(_) => return Err(Error::Dependencies("Instance has conflicting dependencies.".into())),
                }
            }

            if to_add.is_empty() && to_change.is_empty() {
                return Ok(());
            }
            if check_only {
                return Err(Error::Dependencies(
                    "Instance has unresolved dependencies while loading/checking for launch.".into(),
                ));
            }

            let minecraft_version = self.component("net.minecraft").map(|c| c.version.clone()).unwrap_or_default();
            for (index, version) in to_change {
                self.components[index].version = version;
                self.components[index].file = None;
            }
            for (uid, requirement) in to_add {
                let version = if !requirement.equals.is_empty() {
                    requirement.equals
                } else if !requirement.suggests.is_empty() {
                    requirement.suggests
                } else {
                    match uid.as_str() {
                        "org.lwjgl" => "2.9.1".to_string(),
                        "org.lwjgl3" => "3.1.2".to_string(),
                        "net.fabricmc.intermediary" | "org.quiltmc.hashed" => minecraft_version.clone(),
                        _ => String::new(),
                    }
                };
                let component = Component { uid, version, dependency_only: true, ..Component::default() };
                let position = requirement.first_dependee.min(self.components.len());
                self.components.insert(position, component);
            }
        }
    }

    /// The merged profile. Components that `resolve` has not loaded contribute nothing.
    pub fn launch_profile(&self, ctx: &RuntimeContext) -> LaunchProfile {
        let mut profile = LaunchProfile::default();
        for file in self.components.iter().filter(|c| c.is_enabled()).filter_map(|c| c.file.as_ref()) {
            profile.apply(file, ctx);
        }
        profile
    }
}

/// Creates an instance folder with Minecraft and, optionally, a mod loader component.
/// Ported from `VanillaCreationTask`. Nothing is left behind when the components do not resolve.
pub async fn create_instance(
    meta: &Meta,
    instances_dir: &Path,
    name: &str,
    minecraft_version: &str,
    loader: Option<(&str, &str)>,
) -> Result<prism_core::instance::Instance> {
    let instance = prism_core::instance::create_instance(instances_dir, name)?;
    let mut pack = PackProfile::new(&instance.root);
    pack.set_component_version("net.minecraft", minecraft_version, true);
    if let Some((uid, version)) = loader {
        pack.set_component_version(uid, version, false);
    }
    let resolved = pack.resolve(meta, false, false).await.and_then(|()| pack.save());
    if let Err(e) = resolved {
        let _ = std::fs::remove_dir_all(&instance.root);
        return Err(e);
    }
    Ok(instance)
}

#[cfg(test)]
mod tests {
    use super::*;
    use prism_net::Client;

    /// A metadata cache on disk; every request is offline.
    fn meta_with(dir: &Path, files: &[(&str, &str, &str)]) -> Meta {
        for (uid, version, body) in files {
            let path = dir.join("meta").join(uid).join(format!("{version}.json"));
            std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
            std::fs::write(path, body).expect("write");
        }
        Meta::new(Client::new().expect("client"), dir, "http://127.0.0.1:9/")
    }

    const FILES: &[(&str, &str, &str)] = &[
        ("net.minecraft", "26.3", r#"{"uid":"net.minecraft","version":"26.3","name":"Minecraft","mainJar":{"name":"com.mojang:minecraft:26.3:client"},"requires":[{"uid":"org.lwjgl3","suggests":"3.4.3"}]}"#),
        ("net.minecraft", "26.2", r#"{"uid":"net.minecraft","version":"26.2","name":"Minecraft","mainJar":{"name":"com.mojang:minecraft:26.2:client"},"requires":[{"uid":"org.lwjgl3","suggests":"3.4.1"}]}"#),
        ("org.lwjgl3", "3.4.3", r#"{"uid":"org.lwjgl3","version":"3.4.3","name":"LWJGL 3","volatile":true}"#),
        ("net.fabricmc.fabric-loader", "0.20.1", r#"{"uid":"net.fabricmc.fabric-loader","version":"0.20.1","name":"Fabric Loader","requires":[{"uid":"net.fabricmc.intermediary"}]}"#),
        ("net.fabricmc.intermediary", "26.3", r#"{"uid":"net.fabricmc.intermediary","version":"26.3","name":"Intermediary Mappings","volatile":true,"requires":[{"uid":"net.minecraft","equals":"26.3"}]}"#),
        ("net.fabricmc.intermediary", "26.2", r#"{"uid":"net.fabricmc.intermediary","version":"26.2","name":"Intermediary Mappings","volatile":true,"requires":[{"uid":"net.minecraft","equals":"26.2"}]}"#),
    ];

    fn uids(profile: &PackProfile) -> Vec<String> {
        profile.components.iter().map(|c| format!("{} {}", c.uid, c.version)).collect()
    }

    #[tokio::test]
    async fn fabric_instance_resolves_like_the_original() {
        let dir = tempfile::tempdir().expect("tempdir");
        let meta = meta_with(dir.path(), FILES);
        let root = dir.path().join("inst");

        let mut profile = PackProfile::new(&root);
        profile.set_component_version("net.minecraft", "26.3", true);
        profile.set_component_version("net.fabricmc.fabric-loader", "0.20.1", false);
        profile.resolve(&meta, true, false).await.expect("resolve");
        assert_eq!(
            uids(&profile),
            ["org.lwjgl3 3.4.3", "net.minecraft 26.3", "net.fabricmc.intermediary 26.3", "net.fabricmc.fabric-loader 0.20.1"]
        );
        assert!(profile.components[0].dependency_only && profile.components[0].cached_volatile);
        assert!(profile.components[1].important && !profile.components[1].dependency_only);

        // The file on disk has the original's shape and loads back.
        profile.save().expect("save");
        let text = std::fs::read_to_string(root.join("mmc-pack.json")).expect("read");
        assert!(text.contains("\"formatVersion\": 1"));
        assert!(text.contains("            \"cachedName\": \"LWJGL 3\""));
        let mut reloaded = PackProfile::load(&root).expect("load");
        reloaded.resolve(&meta, true, true).await.expect("launch check");

        // Removing the loader drops the dependency that only it needed.
        reloaded.remove("net.fabricmc.fabric-loader");
        reloaded.resolve(&meta, true, false).await.expect("resolve");
        assert_eq!(uids(&reloaded), ["org.lwjgl3 3.4.3", "net.minecraft 26.3"]);
    }

    #[tokio::test]
    async fn launch_check_refuses_unresolved_and_conflicting_lists() {
        let dir = tempfile::tempdir().expect("tempdir");
        let meta = meta_with(dir.path(), FILES);

        let mut missing = PackProfile::new(dir.path());
        missing.set_component_version("net.minecraft", "26.3", true);
        assert!(matches!(missing.resolve(&meta, true, true).await, Err(Error::Dependencies(_))));

        // Intermediary pinned by the user to 26.3 while Minecraft is 26.2.
        let mut conflict = PackProfile::new(dir.path());
        conflict.set_component_version("net.minecraft", "26.2", true);
        conflict.set_component_version("net.fabricmc.intermediary", "26.3", false);
        assert!(matches!(conflict.resolve(&meta, true, false).await, Err(Error::Dependencies(_))));
    }

    #[tokio::test]
    async fn local_patch_replaces_metadata() {
        let dir = tempfile::tempdir().expect("tempdir");
        let meta = meta_with(dir.path(), FILES);
        let root = dir.path().join("inst");
        std::fs::create_dir_all(root.join("patches")).expect("mkdir");
        std::fs::write(
            root.join("patches/org.lwjgl3.json"),
            r#"{"uid":"wrong.uid","version":"custom","name":"My LWJGL","libraries":[{"name":"a:b:1"}]}"#,
        )
        .expect("write");

        let mut profile = PackProfile::new(&root);
        profile.set_component_version("org.lwjgl3", "3.4.3", false);
        profile.set_component_version("net.minecraft", "26.3", true);
        profile.resolve(&meta, true, false).await.expect("resolve");
        let lwjgl = profile.component("org.lwjgl3").expect("component");
        assert!(lwjgl.custom);
        assert_eq!(lwjgl.cached_name, "My LWJGL");
        assert_eq!(lwjgl.file.as_ref().expect("file").uid, "org.lwjgl3");

        let ctx = RuntimeContext { java_architecture: "64".into(), java_real_architecture: "amd64".into(), system: "windows".into() };
        let launch = profile.launch_profile(&ctx);
        assert_eq!(launch.libraries.len(), 1);
        assert!(launch.main_jar.is_some());
    }

    #[test]
    fn bad_uid_in_pack_file_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("mmc-pack.json"), r#"{"components":[{"uid":"../../x"}],"formatVersion":1}"#).expect("write");
        assert!(matches!(PackProfile::load(dir.path()), Err(Error::BadComponent(_))));
    }
}
