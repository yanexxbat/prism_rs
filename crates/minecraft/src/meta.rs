//! Client for the metadata server. Ported from `meta/Index`, `meta/VersionList`,
//! `meta/Version`, `meta/BaseEntity` and `meta/JsonFormat`.

use crate::version_file::{valid_uid, Require, VersionFile};
use crate::{Error, Result};
use prism_net::Client;
use serde::Deserialize;
use std::path::{Path, PathBuf};

pub const DEFAULT_META_URL: &str = "https://meta.prismlauncher.org/v1/";

#[derive(Clone, Debug, Deserialize)]
pub struct IndexEntry {
    pub uid: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Index {
    pub packages: Vec<IndexEntry>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionEntry {
    pub version: String,
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub release_time: String,
    #[serde(default)]
    pub recommended: bool,
    #[serde(default)]
    pub requires: Vec<Require>,
    #[serde(default)]
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct VersionList {
    pub uid: String,
    #[serde(default)]
    pub name: String,
    pub versions: Vec<VersionEntry>,
}

#[derive(Clone)]
pub struct Meta {
    client: Client,
    base_url: String,
    dir: PathBuf,
}

impl Meta {
    /// `url_override` is the `MetaURLOverride` setting; empty means the default server.
    pub fn new(client: Client, data_dir: &Path, url_override: &str) -> Self {
        let mut base_url = if url_override.is_empty() { DEFAULT_META_URL } else { url_override }.to_string();
        if !base_url.ends_with('/') {
            base_url.push('/');
        }
        Meta { client, base_url, dir: data_dir.join("meta") }
    }

    /// Returns the file from the local cache when its hash is the expected one,
    /// otherwise from the server. Without an expected hash the local copy is only
    /// the fallback for a failed or offline request.
    async fn fetch(&self, relative: &str, sha256: Option<&str>, offline: bool) -> Result<Vec<u8>> {
        let path = self.dir.join(relative);
        let local = std::fs::read(&path).ok();
        if let (Some(data), Some(expected)) = (&local, sha256) {
            if prism_net::sha256_hex(data) == expected {
                return Ok(data.clone());
            }
        }
        if offline {
            return local.filter(|_| sha256.is_none()).ok_or_else(|| Error::MetaOffline(relative.to_string()));
        }
        let url = format!("{}{relative}", self.base_url);
        match (self.client.get_bytes(&url).await, sha256) {
            (Ok(data), Some(expected)) if prism_net::sha256_hex(&data) != expected => Err(Error::MetaChecksum {
                file: relative.to_string(),
                expected: expected.to_string(),
                actual: prism_net::sha256_hex(&data),
            }),
            (Ok(data), _) => {
                prism_core::fs::write_atomic(&path, &data)?;
                Ok(data)
            }
            (Err(e), None) if local.is_some() => {
                tracing::warn!("using cached {relative}: {e}");
                Ok(local.unwrap_or_default())
            }
            (Err(e), _) => Err(e.into()),
        }
    }

    fn parse<T: serde::de::DeserializeOwned>(data: &[u8], file: &str) -> Result<T> {
        serde_json::from_slice(data).map_err(|e| Error::BadVersionFile { file: file.to_string(), reason: e.to_string() })
    }

    pub async fn index(&self, offline: bool) -> Result<Index> {
        Self::parse(&self.fetch("index.json", None, offline).await?, "index.json")
    }

    /// The versions of one component, newest first as the server lists them.
    pub async fn version_list(&self, uid: &str, offline: bool) -> Result<VersionList> {
        if !valid_uid(uid) {
            return Err(Error::BadComponent(uid.to_string()));
        }
        let file = format!("{uid}/index.json");
        Self::parse(&self.fetch(&file, None, offline).await?, &file)
    }

    pub async fn version(&self, uid: &str, version: &str, sha256: Option<&str>, offline: bool) -> Result<VersionFile> {
        // Both parts become a file path and a URL.
        let version_ok = !version.is_empty() && !version.contains(['/', '\\']) && !version.contains("..");
        if !valid_uid(uid) || !version_ok {
            return Err(Error::BadComponent(format!("{uid} {version}")));
        }
        let file = format!("{uid}/{version}.json");
        VersionFile::from_json(&self.fetch(&file, sha256, offline).await?, &file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offline_meta(dir: &Path) -> Meta {
        // The port is never contacted in these tests: every call is offline or cached.
        Meta::new(Client::new().expect("client"), dir, "http://127.0.0.1:9/")
    }

    #[tokio::test]
    async fn cached_file_with_matching_hash_needs_no_network() {
        let dir = tempfile::tempdir().expect("tempdir");
        let body = br#"{"formatVersion":1,"uid":"org.lwjgl3","version":"3.4.3","name":"LWJGL 3","volatile":true}"#;
        let path = dir.path().join("meta/org.lwjgl3/3.4.3.json");
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, body).expect("write");

        let meta = offline_meta(dir.path());
        let hash = prism_net::sha256_hex(body);
        let file = meta.version("org.lwjgl3", "3.4.3", Some(&hash), false).await.expect("cached");
        assert!(file.volatile);
        assert_eq!(file.name, "LWJGL 3");

        // Offline, a file without a known hash is taken as is; a wrong hash is not.
        assert!(meta.version("org.lwjgl3", "3.4.3", None, true).await.is_ok());
        assert!(matches!(meta.version("org.lwjgl3", "3.4.3", Some("00"), true).await, Err(Error::MetaOffline(_))));
        assert!(matches!(meta.version("org.lwjgl3", "9.9.9", None, true).await, Err(Error::MetaOffline(_))));
    }

    #[tokio::test]
    async fn path_tricks_are_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let meta = offline_meta(dir.path());
        for (uid, version) in [("../x", "1"), ("net.minecraft", "../../x"), ("net.minecraft", "a/b"), ("net.minecraft", "")] {
            assert!(matches!(meta.version(uid, version, None, true).await, Err(Error::BadComponent(_))), "{uid} {version}");
        }
    }
}
