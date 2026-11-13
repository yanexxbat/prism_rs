//! Component ("version") files and libraries.
//! Ported from `OneSixVersionFormat`, `MojangVersionFormat`, `Library`, `Rule`,
//! `GradleSpecifier`, `RuntimeContext`, `VersionFile` and `LaunchProfile`.

use crate::{Error, Result};
use prism_core::version::Version;
use prism_net::Download;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub const LIBRARY_BASE: &str = "https://libraries.minecraft.net/";

/// The system the game will run on, as far as library selection cares.
#[derive(Clone, Debug, PartialEq)]
pub struct RuntimeContext {
    /// `32` or `64`.
    pub java_architecture: String,
    /// `os.arch` as Java reports it, e.g. `amd64`.
    pub java_real_architecture: String,
    /// `windows`, `linux` or `osx`.
    pub system: String,
}

impl RuntimeContext {
    pub fn current_system() -> &'static str {
        match std::env::consts::OS {
            "windows" => "windows",
            "macos" => "osx",
            other => {
                if other == "linux" {
                    "linux"
                } else {
                    "unknown"
                }
            }
        }
    }

    fn mapped_arch(&self) -> &str {
        match self.java_real_architecture.as_str() {
            "amd64" => "x86_64",
            "i386" | "i686" => "x86",
            "aarch64" => "arm64",
            "arm" | "armhf" => "arm32",
            other => other,
        }
    }

    pub fn classifier(&self) -> String {
        format!("{}-{}", self.system, self.mapped_arch())
    }

    /// Mojang's metadata assumes these two when it names only the OS.
    fn is_legacy_arch(&self) -> bool {
        matches!(self.mapped_arch(), "x86_64" | "x86")
    }

    pub fn classifier_matches(&self, target: &str) -> bool {
        target == self.classifier() || (self.is_legacy_arch() && target == self.system)
    }
}

/// `group:artifact:version[:classifier][@extension]`
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GradleSpecifier {
    pub group: String,
    pub artifact: String,
    pub version: String,
    pub classifier: String,
    pub extension: Option<String>,
    pub valid: bool,
}

impl GradleSpecifier {
    pub fn parse(value: &str) -> Self {
        let (coords, extension) = match value.split_once('@') {
            Some((coords, ext)) => (coords, Some(ext)),
            None => (value, None),
        };
        let parts: Vec<&str> = coords.split(':').collect();
        let ext_ok = extension.is_none_or(|e| !e.is_empty() && !e.contains([':', '@']));
        if !(3..=4).contains(&parts.len()) || parts.iter().any(|p| p.is_empty()) || !ext_ok {
            return Self::default();
        }
        GradleSpecifier {
            group: parts[0].into(),
            artifact: parts[1].into(),
            version: parts[2].into(),
            classifier: parts.get(3).copied().unwrap_or_default().into(),
            extension: extension.map(str::to_string),
            valid: true,
        }
    }

    pub fn file_name(&self) -> String {
        if !self.valid {
            return String::new();
        }
        let classifier = if self.classifier.is_empty() { String::new() } else { format!("-{}", self.classifier) };
        format!("{}-{}{}.{}", self.artifact, self.version, classifier, self.extension.as_deref().unwrap_or("jar"))
    }

    pub fn to_path(&self, filename_override: Option<&str>) -> String {
        if !self.valid {
            return String::new();
        }
        let file = filename_override.map(str::to_string).unwrap_or_else(|| self.file_name());
        format!("{}/{}/{}/{}", self.group.replace('.', "/"), self.artifact, self.version, file)
    }

    fn matches_name(&self, other: &GradleSpecifier) -> bool {
        self.artifact == other.artifact && self.group == other.group && self.classifier == other.classifier
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Require {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub equals: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub suggests: String,
    pub uid: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct DownloadInfo {
    pub sha1: String,
    pub size: u64,
    pub url: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AssetIndexInfo {
    pub id: String,
    pub sha1: String,
    pub size: u64,
    pub total_size: u64,
    pub url: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct LibraryDownloads {
    pub artifact: Option<DownloadInfo>,
    pub classifiers: BTreeMap<String, DownloadInfo>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct RuleOs {
    pub name: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Rule {
    pub action: String,
    pub os: Option<RuleOs>,
}

impl Rule {
    /// `None` means the rule does not apply to this system.
    fn apply(&self, ctx: &RuntimeContext) -> Option<bool> {
        if let Some(name) = self.os.as_ref().and_then(|os| os.name.as_deref()) {
            if !ctx.classifier_matches(name) {
                return None;
            }
        }
        match self.action.as_str() {
            "allow" => Some(true),
            "disallow" => Some(false),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Library {
    pub name: String,
    /// Maven repository base URL.
    pub url: Option<String>,
    pub natives: BTreeMap<String, String>,
    pub rules: Option<Vec<Rule>>,
    pub downloads: Option<LibraryDownloads>,
    #[serde(rename = "MMC-hint")]
    pub hint: Option<String>,
    #[serde(rename = "MMC-absoluteUrl", alias = "MMC-absulute_url")]
    pub absolute_url: Option<String>,
    #[serde(rename = "MMC-filename")]
    pub filename: Option<String>,
}

/// The files of one library on this system.
#[derive(Debug, Default, PartialEq)]
pub struct LibraryFiles {
    pub jar: Option<PathBuf>,
    pub native: Option<PathBuf>,
}

impl Library {
    pub fn spec(&self) -> GradleSpecifier {
        GradleSpecifier::parse(&self.name)
    }

    pub fn is_native(&self) -> bool {
        !self.natives.is_empty()
    }

    /// A file the user put into the instance's `libraries` folder.
    pub fn is_local(&self) -> bool {
        self.hint.as_deref() == Some("local")
    }

    fn compatible_native(&self, ctx: &RuntimeContext) -> Option<&str> {
        let mut entry = self.natives.get(&ctx.classifier());
        if entry.is_none() && ctx.is_legacy_arch() {
            entry = self.natives.get(&ctx.system);
        }
        entry.map(String::as_str)
    }

    pub fn is_active(&self, ctx: &RuntimeContext) -> bool {
        let allowed = match self.rules.as_deref() {
            None | Some([]) => true,
            // The last rule that applies decides; with none applying the library is off.
            Some(rules) => rules.iter().filter_map(|r| r.apply(ctx)).last().unwrap_or(false),
        };
        allowed && (!self.is_native() || self.compatible_native(ctx).is_some())
    }

    /// Path below the library store, with `${arch}` filled in.
    pub fn storage_suffix(&self, ctx: &RuntimeContext) -> String {
        let mut spec = self.spec();
        if self.is_native() {
            spec.classifier = self.compatible_native(ctx).unwrap_or("INVALID").to_string();
        }
        spec.to_path(self.filename.as_deref()).replace("${arch}", &ctx.java_architecture)
    }

    fn actual_path(&self, ctx: &RuntimeContext, libraries_dir: &Path, local_dir: &Path) -> PathBuf {
        let relative = prism_core::fs::remove_invalid_path_chars(&self.storage_suffix(ctx));
        if self.is_local() {
            let file = Path::new(&relative).file_name().unwrap_or_default();
            local_dir.join(file)
        } else {
            libraries_dir.join(relative)
        }
    }

    pub fn files(&self, ctx: &RuntimeContext, libraries_dir: &Path, local_dir: &Path) -> LibraryFiles {
        let path = Some(self.actual_path(ctx, libraries_dir, local_dir));
        if self.is_native() {
            LibraryFiles { jar: None, native: path }
        } else {
            LibraryFiles { jar: path, native: None }
        }
    }

    /// The download for this library, or `None` when the file is already there
    /// or the metadata names no file for this system.
    pub fn download(&self, ctx: &RuntimeContext, libraries_dir: &Path, local_dir: &Path) -> Result<Option<Download>> {
        let path = self.actual_path(ctx, libraries_dir, local_dir);
        if self.is_local() {
            return if path.is_file() { Ok(None) } else { Err(Error::MissingLocalLibrary(path)) };
        }
        let always_stale = self.hint.as_deref() == Some("always-stale");
        if path.is_file() && !always_stale {
            return Ok(None);
        }
        let (url, sha1) = match &self.downloads {
            Some(downloads) => {
                let info = if self.is_native() {
                    self.compatible_native(ctx)
                        .map(|c| c.replace("${arch}", &ctx.java_architecture))
                        .and_then(|c| downloads.classifiers.get(&c))
                } else {
                    downloads.artifact.as_ref()
                };
                match info {
                    Some(info) => (info.url.clone(), Some(info.sha1.clone()).filter(|s| !s.is_empty())),
                    None => return Ok(None),
                }
            }
            None => {
                let suffix = self.storage_suffix(ctx);
                let url = match (&self.absolute_url, &self.url) {
                    (Some(absolute), _) if !absolute.is_empty() => absolute.clone(),
                    (_, Some(repo)) if !repo.is_empty() => format!("{}/{suffix}", repo.trim_end_matches('/')),
                    _ => format!("{LIBRARY_BASE}{suffix}"),
                };
                (url, None)
            }
        };
        Ok(Some(Download { url, path, sha1 }))
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Agent {
    #[serde(flatten)]
    pub library: Library,
    #[serde(default)]
    pub argument: Option<String>,
}

/// Only letters, digits, `-` and `_` in dot-separated parts. Uids end up in file paths and URLs.
pub fn valid_uid(uid: &str) -> bool {
    !uid.is_empty()
        && uid.split('.').all(|part| {
            !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        })
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct VersionFile {
    #[serde(rename = "formatVersion")]
    pub format_version: Option<i64>,
    pub name: String,
    #[serde(alias = "fileId")]
    pub uid: String,
    pub version: String,
    /// `id` in Mojang's format.
    #[serde(rename = "id")]
    pub minecraft_version: String,
    #[serde(rename = "mainClass")]
    pub main_class: String,
    #[serde(rename = "appletClass")]
    pub applet_class: String,
    #[serde(rename = "minecraftArguments")]
    pub minecraft_arguments: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub assets: Option<String>,
    #[serde(rename = "assetIndex")]
    pub asset_index: Option<AssetIndexInfo>,
    #[serde(rename = "compatibleJavaMajors")]
    pub compatible_java_majors: Vec<u32>,
    #[serde(rename = "compatibleJavaName")]
    pub compatible_java_name: String,
    pub downloads: BTreeMap<String, DownloadInfo>,
    #[serde(rename = "+tweakers")]
    pub tweakers: Vec<String>,
    #[serde(rename = "+traits")]
    pub traits: BTreeSet<String>,
    #[serde(rename = "+jvmArgs")]
    pub jvm_args: Vec<String>,
    #[serde(rename = "jarMods", alias = "+jarMods")]
    pub jar_mods: Vec<serde_json::Value>,
    pub libraries: Vec<Library>,
    #[serde(rename = "+libraries")]
    pub plus_libraries: Vec<Library>,
    #[serde(rename = "mavenFiles")]
    pub maven_files: Vec<Library>,
    #[serde(rename = "+agents")]
    pub agents: Vec<Agent>,
    #[serde(rename = "mainJar")]
    pub main_jar: Option<Library>,
    pub requires: Vec<Require>,
    pub conflicts: Vec<Require>,
    pub volatile: bool,
    #[serde(rename = "mcVersion")]
    pub mc_version: String,
}

impl VersionFile {
    /// Parses and applies the fix-ups `OneSixVersionFormat::versionFileFromJson` does.
    pub fn from_json(data: &[u8], origin: &str) -> Result<Self> {
        let bad = |reason: String| Error::BadVersionFile { file: origin.to_string(), reason };
        let mut file: VersionFile = serde_json::from_slice(data).map_err(|e| bad(e.to_string()))?;
        if !matches!(file.format_version, None | Some(0) | Some(1)) {
            return Err(bad("format version is not supported".into()));
        }
        file.libraries.append(&mut file.plus_libraries);
        if file.main_jar.is_none() && !file.minecraft_version.is_empty() {
            let client = file.downloads.get("client").cloned().ok_or_else(|| {
                bad("URL for the main jar could not be determined".into())
            })?;
            file.main_jar = Some(Library {
                name: format!("com.mojang:minecraft:{}:client", file.minecraft_version),
                downloads: Some(LibraryDownloads { artifact: Some(client), classifiers: BTreeMap::new() }),
                ..Library::default()
            });
        }
        if !file.mc_version.is_empty() {
            file.requires.push(Require { uid: "net.minecraft".into(), equals: file.mc_version.clone(), ..Require::default() });
        }
        if file.asset_index.is_none() {
            file.asset_index = file.assets.clone().map(|id| AssetIndexInfo { id, ..AssetIndexInfo::default() });
        }
        Ok(file)
    }
}

/// Everything the components add up to. Built by applying each enabled component in order.
#[derive(Clone, Debug, Default)]
pub struct LaunchProfile {
    pub minecraft_version: String,
    pub minecraft_version_type: String,
    pub assets: Option<AssetIndexInfo>,
    pub minecraft_arguments: String,
    pub main_class: String,
    pub applet_class: String,
    pub jvm_args: Vec<String>,
    pub tweakers: Vec<String>,
    pub traits: BTreeSet<String>,
    pub libraries: Vec<Library>,
    pub native_libraries: Vec<Library>,
    pub maven_files: Vec<Library>,
    pub agents: Vec<Agent>,
    pub has_jar_mods: bool,
    pub main_jar: Option<Library>,
    pub compatible_java_majors: Vec<u32>,
    pub compatible_java_name: String,
}

fn apply_string(from: &str, to: &mut String) {
    if !from.is_empty() {
        *to = from.to_string();
    }
}

impl LaunchProfile {
    pub fn apply(&mut self, file: &VersionFile, ctx: &RuntimeContext) {
        // Only the real Minecraft component may set these.
        if file.uid == "net.minecraft" {
            apply_string(&file.version, &mut self.minecraft_version);
            apply_string(&file.kind, &mut self.minecraft_version_type);
            if file.asset_index.is_some() {
                self.assets = file.asset_index.clone();
            }
        }
        if file.main_jar.is_some() {
            self.main_jar = file.main_jar.clone();
        }
        apply_string(&file.main_class, &mut self.main_class);
        apply_string(&file.applet_class, &mut self.applet_class);
        apply_string(&file.minecraft_arguments, &mut self.minecraft_arguments);
        self.jvm_args.extend(file.jvm_args.iter().cloned());
        self.tweakers.retain(|t| !file.tweakers.contains(t));
        self.tweakers.extend(file.tweakers.iter().cloned());
        self.has_jar_mods |= !file.jar_mods.is_empty();
        self.traits.extend(file.traits.iter().cloned());
        self.compatible_java_majors.extend(&file.compatible_java_majors);
        apply_string(&file.compatible_java_name, &mut self.compatible_java_name);

        for library in file.libraries.iter().filter(|l| l.is_active(ctx)) {
            let list = if library.is_native() { &mut self.native_libraries } else { &mut self.libraries };
            let spec = library.spec();
            let mut same_name = list.iter_mut().filter(|l| l.spec().matches_name(&spec));
            match (same_name.next(), same_name.next()) {
                // Exactly one library of that name: the newer version wins.
                (Some(existing), None) => {
                    if Version::new(&spec.version) > Version::new(&existing.spec().version) {
                        *existing = library.clone();
                    }
                }
                _ => list.push(library.clone()),
            }
        }
        let usable = |l: &Library| l.is_active(ctx) && !l.is_native();
        self.maven_files.extend(file.maven_files.iter().filter(|l| usable(l)).cloned());
        self.agents.extend(file.agents.iter().filter(|a| usable(&a.library)).cloned());
    }

    pub fn assets(&self) -> AssetIndexInfo {
        self.assets.clone().unwrap_or_else(|| AssetIndexInfo { id: "legacy".into(), ..AssetIndexInfo::default() })
    }

    /// Classpath jars (libraries, then the main jar) and native jars to extract.
    pub fn library_files(&self, ctx: &RuntimeContext, libraries_dir: &Path, local_dir: &Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
        let (mut jars, mut natives) = (Vec::new(), Vec::new());
        let ordered = self.libraries.iter().chain(&self.main_jar).chain(&self.native_libraries);
        for library in ordered {
            let files = library.files(ctx, libraries_dir, local_dir);
            jars.extend(files.jar);
            natives.extend(files.native);
        }
        (jars, natives)
    }

    /// Every file a launch needs from the library store.
    pub fn all_libraries(&self) -> impl Iterator<Item = &Library> {
        self.libraries.iter().chain(&self.native_libraries).chain(&self.maven_files).chain(&self.main_jar)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(system: &str, arch: &str) -> RuntimeContext {
        RuntimeContext { java_architecture: "64".into(), java_real_architecture: arch.into(), system: system.into() }
    }

    fn library(json: &str) -> Library {
        serde_json::from_str(json).expect("library json")
    }

    #[test]
    fn gradle_specifier_paths() {
        let plain = GradleSpecifier::parse("org.lwjgl:lwjgl:3.4.3");
        assert_eq!(plain.to_path(None), "org/lwjgl/lwjgl/3.4.3/lwjgl-3.4.3.jar");
        let full = GradleSpecifier::parse("org.gradle.test.classifiers:service:1.0:jdk15@zip");
        assert_eq!(full.file_name(), "service-1.0-jdk15.zip");
        assert!(!GradleSpecifier::parse("broken").valid);
        assert!(!GradleSpecifier::parse("a:b:c:d:e").valid);
        assert_eq!(GradleSpecifier::parse("a::c").to_path(None), "");
    }

    #[test]
    fn rules_pick_the_right_system() {
        let osx_only = library(
            r#"{"name":"ca.weblite:java-objc-bridge:1.1","rules":[{"action":"allow","os":{"name":"osx"}},{"action":"allow","os":{"name":"osx-arm64"}}]}"#,
        );
        assert!(!osx_only.is_active(&ctx("windows", "amd64")));
        assert!(osx_only.is_active(&ctx("osx", "amd64")));
        assert!(osx_only.is_active(&ctx("osx", "aarch64")));

        // "windows" alone means x86/x86_64; ARM has its own classifier.
        let win_x86 = library(r#"{"name":"a:b-natives-windows:1","rules":[{"action":"allow","os":{"name":"windows"}}]}"#);
        assert!(win_x86.is_active(&ctx("windows", "amd64")));
        assert!(!win_x86.is_active(&ctx("windows", "aarch64")));

        let not_osx = library(r#"{"name":"a:b:1","rules":[{"action":"allow"},{"action":"disallow","os":{"name":"osx"}}]}"#);
        assert!(not_osx.is_active(&ctx("linux", "amd64")));
        assert!(!not_osx.is_active(&ctx("osx", "amd64")));
    }

    #[test]
    fn old_style_natives_use_classifiers() {
        let lwjgl2 = library(
            r#"{"name":"org.lwjgl.lwjgl:lwjgl-platform:2.9.4","natives":{"windows":"natives-windows","linux":"natives-linux"},
                "downloads":{"classifiers":{"natives-windows":{"sha1":"aa","size":1,"url":"https://example.invalid/win.jar"}}}}"#,
        );
        let windows = ctx("windows", "amd64");
        assert!(lwjgl2.is_active(&windows));
        assert!(!lwjgl2.is_active(&ctx("osx", "amd64")));
        assert_eq!(
            lwjgl2.storage_suffix(&windows),
            "org/lwjgl/lwjgl/lwjgl-platform/2.9.4/lwjgl-platform-2.9.4-natives-windows.jar"
        );
        let dir = tempfile::tempdir().expect("tempdir");
        let download = lwjgl2.download(&windows, dir.path(), dir.path()).expect("ok").expect("needed");
        assert_eq!(download.url, "https://example.invalid/win.jar");
        assert_eq!(download.sha1.as_deref(), Some("aa"));
        let files = lwjgl2.files(&windows, dir.path(), dir.path());
        assert!(files.jar.is_none() && files.native.is_some());
    }

    #[test]
    fn maven_url_is_built_from_the_repository() {
        let dir = tempfile::tempdir().expect("tempdir");
        let c = ctx("windows", "amd64");
        let fabric = library(r#"{"name":"net.fabricmc:fabric-loader:0.20.1","url":"https://maven.fabricmc.net"}"#);
        let download = fabric.download(&c, dir.path(), dir.path()).expect("ok").expect("needed");
        assert_eq!(download.url, "https://maven.fabricmc.net/net/fabricmc/fabric-loader/0.20.1/fabric-loader-0.20.1.jar");
        assert_eq!(download.sha1, None);

        let mojang = library(r#"{"name":"com.mojang:brigadier:1.0"}"#);
        let download = mojang.download(&c, dir.path(), dir.path()).expect("ok").expect("needed");
        assert!(download.url.starts_with(LIBRARY_BASE));

        let local = library(r#"{"name":"custom:thing:1","MMC-hint":"local"}"#);
        assert!(matches!(local.download(&c, dir.path(), dir.path()), Err(Error::MissingLocalLibrary(_))));
    }

    #[test]
    fn profile_merges_components_in_order() {
        let c = ctx("windows", "amd64");
        let minecraft = VersionFile::from_json(
            br#"{"uid":"net.minecraft","version":"26.3","type":"release","mainClass":"net.minecraft.client.main.Main",
                "minecraftArguments":"--username ${auth_player_name}","assetIndex":{"id":"34","sha1":"x","size":1,"totalSize":2,"url":"u"},
                "mainJar":{"name":"com.mojang:minecraft:26.3:client"},"+traits":["XR:Initial"],"compatibleJavaMajors":[25],
                "libraries":[{"name":"org.ow2.asm:asm:9.6"},{"name":"com.google.guava:guava:33.0"}]}"#,
            "test",
        )
        .expect("minecraft");
        let fabric = VersionFile::from_json(
            br#"{"uid":"net.fabricmc.fabric-loader","version":"0.20.1","mainClass":"net.fabricmc.loader.impl.launch.knot.KnotClient",
                "assetIndex":{"id":"evil"},"+tweakers":["t"],
                "libraries":[{"name":"org.ow2.asm:asm:9.8"},{"name":"com.google.guava:guava:31.0"},{"name":"net.fabricmc:fabric-loader:0.20.1"}]}"#,
            "test",
        )
        .expect("fabric");

        let mut profile = LaunchProfile::default();
        profile.apply(&minecraft, &c);
        profile.apply(&fabric, &c);

        assert_eq!(profile.main_class, "net.fabricmc.loader.impl.launch.knot.KnotClient");
        assert_eq!(profile.minecraft_version, "26.3");
        assert_eq!(profile.assets().id, "34", "only net.minecraft sets the asset index");
        let names: Vec<&str> = profile.libraries.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["org.ow2.asm:asm:9.8", "com.google.guava:guava:33.0", "net.fabricmc:fabric-loader:0.20.1"]);

        let (jars, natives) = profile.library_files(&c, Path::new("libs"), Path::new("local"));
        assert!(natives.is_empty());
        assert!(jars.last().expect("main jar").ends_with("minecraft-26.3-client.jar"));
    }

    #[test]
    fn uids_are_checked() {
        assert!(valid_uid("net.fabricmc.fabric-loader"));
        assert!(valid_uid("org.lwjgl3"));
        for bad in ["", "../etc", "a..b", "a/b", "a b", ".a"] {
            assert!(!valid_uid(bad), "{bad}");
        }
    }

    #[test]
    fn unsupported_format_version_is_refused() {
        assert!(VersionFile::from_json(br#"{"formatVersion":2,"uid":"a"}"#, "t").is_err());
        assert!(VersionFile::from_json(br#"{"id":"1.0","uid":"net.minecraft"}"#, "t").is_err(), "no main jar URL");
    }
}
