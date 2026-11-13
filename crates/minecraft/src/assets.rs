//! Asset index and objects. Ported from `AssetsUtils` and `AssetUpdateTask`.

use crate::version_file::AssetIndexInfo;
use crate::{io_err, Error, Result};
use prism_net::{Client, Download};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const DEFAULT_RESOURCE_BASE: &str = "https://resources.download.minecraft.net/";

#[derive(Deserialize)]
struct AssetObject {
    hash: String,
    size: u64,
}

#[derive(Deserialize)]
struct AssetsIndex {
    #[serde(default, rename = "virtual")]
    is_virtual: bool,
    #[serde(default)]
    map_to_resources: bool,
    objects: BTreeMap<String, AssetObject>,
}

/// The id comes from metadata and becomes a file name.
fn safe_id(id: &str) -> String {
    prism_core::fs::remove_invalid_filename_chars(id)
}

fn index_path(data_dir: &Path, id: &str) -> PathBuf {
    data_dir.join("assets").join("indexes").join(format!("{}.json", safe_id(id)))
}

fn load_index(data_dir: &Path, id: &str) -> Result<AssetsIndex> {
    let path = index_path(data_dir, id);
    let index: AssetsIndex = prism_core::fs::read_json(&path)?;
    // Hashes become file paths and URLs.
    match index.objects.values().find(|o| o.hash.len() < 2 || !o.hash.chars().all(|c| c.is_ascii_hexdigit())) {
        Some(bad) => Err(Error::Launch(format!("{}: invalid asset hash '{}'", path.display(), bad.hash))),
        None => Ok(index),
    }
}

fn object_path(data_dir: &Path, hash: &str) -> PathBuf {
    data_dir.join("assets").join("objects").join(&hash[..2]).join(hash)
}

/// Downloads the asset index (when it changed) and every missing or wrongly sized object.
/// `resource_url` is the `ResourceURLOverride` setting; empty means Mojang's server.
pub async fn update(
    client: &Client,
    data_dir: &Path,
    info: &AssetIndexInfo,
    resource_url: &str,
    concurrency: usize,
    progress: impl Fn(usize, usize),
) -> Result<()> {
    let path = index_path(data_dir, &info.id);
    let current = std::fs::read(&path).is_ok_and(|data| !info.sha1.is_empty() && prism_net::sha1_hex(&data) == info.sha1);
    if !current && !info.url.is_empty() {
        let sha1 = Some(info.sha1.clone()).filter(|s| !s.is_empty());
        client.download(&Download { url: info.url.clone(), path, sha1 }).await?;
    }
    let index = load_index(data_dir, &info.id).map_err(|e| Error::Launch(format!("Failed to read the assets index! {e}")))?;

    let base = if resource_url.is_empty() { DEFAULT_RESOURCE_BASE } else { resource_url };
    let mut downloads: BTreeMap<&str, Download> = BTreeMap::new();
    for object in index.objects.values() {
        let path = object_path(data_dir, &object.hash);
        if !std::fs::metadata(&path).is_ok_and(|m| m.is_file() && m.len() == object.size) {
            let url = format!("{base}{}/{}", &object.hash[..2], object.hash);
            downloads.insert(&object.hash, Download { url, path, sha1: Some(object.hash.clone()) });
        }
    }
    let downloads: Vec<Download> = downloads.into_values().collect();
    Ok(client.download_all(&downloads, concurrency, progress).await?)
}

/// The folder `${game_assets}` points at. Old versions read assets by file name
/// from a "virtual" folder or from the instance's `resources` folder.
pub fn assets_dir(data_dir: &Path, id: &str, resources_dir: &Path) -> PathBuf {
    match load_index(data_dir, id) {
        Ok(index) if index.map_to_resources && !index.is_virtual => resources_dir.to_path_buf(),
        _ => data_dir.join("assets").join("virtual").join(safe_id(id)),
    }
}

/// Copies objects to their real names when the index asks for it.
pub fn reconstruct(data_dir: &Path, id: &str, resources_dir: &Path) -> Result<()> {
    let Ok(index) = load_index(data_dir, id) else { return Ok(()) };
    if !index.is_virtual && !index.map_to_resources {
        return Ok(());
    }
    let target = assets_dir(data_dir, id, resources_dir);
    for (name, object) in &index.objects {
        // Names come from the index; keep them inside the target folder.
        if Path::new(name).components().any(|c| !matches!(c, std::path::Component::Normal(_))) {
            return Err(Error::Launch(format!("asset name '{name}' leaves the assets folder")));
        }
        let (from, to) = (object_path(data_dir, &object.hash), target.join(name));
        if from.is_file() && !to.exists() {
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent).map_err(io_err(parent))?;
            }
            std::fs::copy(&from, &to).map_err(io_err(&to))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_index(data_dir: &Path, id: &str, body: &str) {
        let path = index_path(data_dir, id);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, body).expect("write");
    }

    #[test]
    fn virtual_index_is_rebuilt_by_name() {
        let dir = tempfile::tempdir().expect("tempdir");
        let hash = "a9993e364706816aba3e25717850c26c9cd0d89d";
        write_index(dir.path(), "legacy", &format!(r#"{{"virtual":true,"objects":{{"sounds/a.ogg":{{"hash":"{hash}","size":3}}}}}}"#));
        let object = object_path(dir.path(), hash);
        std::fs::create_dir_all(object.parent().expect("parent")).expect("mkdir");
        std::fs::write(&object, b"abc").expect("write");

        let resources = dir.path().join("resources");
        let target = assets_dir(dir.path(), "legacy", &resources);
        assert!(target.ends_with("assets/virtual/legacy"));
        reconstruct(dir.path(), "legacy", &resources).expect("reconstruct");
        assert_eq!(std::fs::read(target.join("sounds/a.ogg")).expect("read"), b"abc");
    }

    #[test]
    fn modern_index_needs_no_rebuild_and_bad_entries_are_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let resources = dir.path().join("resources");
        write_index(dir.path(), "34", r#"{"objects":{"a":{"hash":"a9993e364706816aba3e25717850c26c9cd0d89d","size":3}}}"#);
        reconstruct(dir.path(), "34", &resources).expect("nothing to do");
        assert!(!dir.path().join("assets/virtual").exists());

        write_index(dir.path(), "evil", r#"{"virtual":true,"objects":{"a":{"hash":"../../../x","size":3}}}"#);
        assert!(load_index(dir.path(), "evil").is_err());
        write_index(dir.path(), "evil2", r#"{"virtual":true,"objects":{"../../x":{"hash":"a9993e364706816aba3e25717850c26c9cd0d89d","size":3}}}"#);
        assert!(reconstruct(dir.path(), "evil2", &resources).is_err());
    }
}
