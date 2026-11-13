//! Mod loaders as instance components. Ported from the loader handling in
//! `PackProfile` (`getModLoaders`, `getSupportedModLoaders`) and `ResourceAPI`.

use crate::meta::{Meta, VersionEntry};
use crate::pack_profile::PackProfile;
use crate::Result;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Loader {
    Fabric,
    Quilt,
    Forge,
    NeoForge,
}

impl Loader {
    pub const ALL: [Loader; 4] = [Loader::Fabric, Loader::Quilt, Loader::Forge, Loader::NeoForge];

    /// The component uid on the metadata server.
    pub fn uid(self) -> &'static str {
        match self {
            Loader::Fabric => "net.fabricmc.fabric-loader",
            Loader::Quilt => "org.quiltmc.quilt-loader",
            Loader::Forge => "net.minecraftforge",
            Loader::NeoForge => "net.neoforged",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Loader::Fabric => "Fabric",
            Loader::Quilt => "Quilt",
            Loader::Forge => "Forge",
            Loader::NeoForge => "NeoForge",
        }
    }

    /// Modrinth's loader category.
    pub fn modrinth(self) -> &'static str {
        match self {
            Loader::Fabric => "fabric",
            Loader::Quilt => "quilt",
            Loader::Forge => "forge",
            Loader::NeoForge => "neoforge",
        }
    }

    /// CurseForge's `ModLoaderType`.
    pub fn curseforge(self) -> u32 {
        match self {
            Loader::Forge => 1,
            Loader::Fabric => 4,
            Loader::Quilt => 5,
            Loader::NeoForge => 6,
        }
    }

    /// The loader an instance has, if any.
    pub fn of(pack: &PackProfile) -> Option<Loader> {
        Loader::ALL.into_iter().find(|loader| pack.component(loader.uid()).is_some())
    }

    /// Loaders whose mods run on an instance with this loader: Quilt also loads Fabric
    /// mods, and NeoForge for 1.20.1 still loads Forge mods.
    pub fn accepted(self, minecraft_version: &str) -> Vec<Loader> {
        match self {
            Loader::Quilt => vec![Loader::Quilt, Loader::Fabric],
            Loader::NeoForge if minecraft_version == "1.20.1" => vec![Loader::NeoForge, Loader::Forge],
            other => vec![other],
        }
    }
}

/// Keeps the versions that can go with this Minecraft version: those that require
/// exactly it, and those that do not name a Minecraft version at all (Fabric, Quilt).
fn compatible(versions: Vec<VersionEntry>, minecraft_version: &str) -> Vec<VersionEntry> {
    let fits = |entry: &VersionEntry| {
        entry.requires.iter().filter(|r| r.uid == "net.minecraft").all(|r| r.equals.is_empty() || r.equals == minecraft_version)
    };
    versions.into_iter().filter(fits).collect()
}

/// The loader versions that can be installed for a Minecraft version, as the server orders them.
pub async fn loader_versions(meta: &Meta, loader: Loader, minecraft_version: &str) -> Result<Vec<VersionEntry>> {
    // Fabric and Quilt run on intermediary mappings, which exist per Minecraft version.
    if matches!(loader, Loader::Fabric | Loader::Quilt) {
        let mappings = meta.version_list("net.fabricmc.intermediary", false).await?;
        if !mappings.versions.iter().any(|v| v.version == minecraft_version) {
            return Ok(Vec::new());
        }
    }
    Ok(compatible(meta.version_list(loader.uid(), false).await?.versions, minecraft_version))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_filtered_by_minecraft_requirement() {
        let entries: Vec<VersionEntry> = serde_json::from_str(
            r#"[{"version":"64.0.1","requires":[{"uid":"net.minecraft","equals":"26.3"}]},
                {"version":"47.2.0","requires":[{"uid":"net.minecraft","equals":"1.20.1"}]},
                {"version":"0.20.1","requires":[{"uid":"net.fabricmc.intermediary"}]}]"#,
        )
        .expect("entries");
        let names = |mc: &str| compatible(entries.clone(), mc).into_iter().map(|e| e.version).collect::<Vec<_>>();
        assert_eq!(names("26.3"), ["64.0.1", "0.20.1"]);
        assert_eq!(names("1.20.1"), ["47.2.0", "0.20.1"]);
    }

    #[test]
    fn instance_loader_and_accepted_mods() {
        let mut pack = PackProfile::new(std::path::Path::new("unused"));
        pack.set_component_version("net.minecraft", "1.20.1", true);
        assert_eq!(Loader::of(&pack), None);
        pack.set_component_version("net.neoforged", "47.1.0", false);
        assert_eq!(Loader::of(&pack), Some(Loader::NeoForge));
        assert_eq!(Loader::NeoForge.accepted("1.20.1"), [Loader::NeoForge, Loader::Forge]);
        assert_eq!(Loader::NeoForge.accepted("26.3"), [Loader::NeoForge]);
        assert_eq!(Loader::Quilt.accepted("26.3"), [Loader::Quilt, Loader::Fabric]);
    }
}
