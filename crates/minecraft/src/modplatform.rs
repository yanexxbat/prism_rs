//! Mod search and download on Modrinth and CurseForge.
//! Ported from `modplatform/modrinth/ModrinthAPI`, `ModrinthPackIndex`,
//! `modplatform/flame/FlameAPI`, `FlameModIndex` and `ResourceDownloadTask`.

use crate::loaders::Loader;
use crate::{Error, Result};
use prism_net::{Client, Download};
use serde_json::Value;
use std::path::{Path, PathBuf};

const MODRINTH: &str = "https://api.modrinth.com/v2";
const CURSEFORGE: &str = "https://api.curseforge.com/v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    Modrinth,
    CurseForge,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Project {
    pub provider: Provider,
    pub id: String,
    pub title: String,
    pub description: String,
    pub author: String,
    pub downloads: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModVersion {
    /// What the user picks from, e.g. `mc26.3-0.7.0-fabric`.
    pub name: String,
    /// `release`, `beta` or `alpha`.
    pub release_type: String,
    pub file_name: String,
    /// `None` when the author does not allow downloads through the API (CurseForge).
    pub url: Option<String>,
    pub sha1: Option<String>,
}

/// What to look for. An empty `text` lists the most popular mods.
#[derive(Clone, Debug, Default)]
pub struct Query {
    pub text: String,
    pub minecraft_version: String,
    pub loaders: Vec<Loader>,
    pub offset: u32,
}

/// The CurseForge API key: the `PRISM_RS_CURSEFORGE_API_KEY` environment variable,
/// else the `FlameKeyOverride` setting. None is built in.
pub fn curseforge_api_key(setting: &str) -> Option<String> {
    std::env::var("PRISM_RS_CURSEFORGE_API_KEY")
        .ok()
        .or_else(|| Some(setting.to_string()))
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty())
}

fn text(value: &Value, key: &str) -> String {
    value.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn list<'a>(value: &'a Value, key: &str) -> impl Iterator<Item = &'a Value> {
    value.get(key).and_then(Value::as_array).into_iter().flatten()
}

fn modrinth_facets(query: &Query) -> String {
    let mut facets = Vec::new();
    if !query.loaders.is_empty() {
        let loaders: Vec<String> = query.loaders.iter().map(|l| format!("\"categories:{}\"", l.modrinth())).collect();
        facets.push(format!("[{}]", loaders.join(",")));
    }
    if !query.minecraft_version.is_empty() {
        facets.push(format!("[\"versions:{}\"]", query.minecraft_version));
    }
    facets.push("[\"project_type:mod\"]".to_string());
    format!("[{}]", facets.join(","))
}

fn modrinth_search_params(query: &Query) -> Vec<(&'static str, String)> {
    let mut params = vec![("offset", query.offset.to_string()), ("limit", "25".to_string())];
    if !query.text.is_empty() {
        params.push(("query", query.text.clone()));
    }
    params.extend([("index", "relevance".to_string()), ("facets", modrinth_facets(query))]);
    params
}

fn modrinth_version_params(minecraft_version: &str, loaders: &[Loader]) -> Vec<(&'static str, String)> {
    let loaders: Vec<String> = loaders.iter().map(|l| format!("\"{}\"", l.modrinth())).collect();
    vec![
        ("game_versions", format!("[\"{minecraft_version}\"]")),
        ("loaders", format!("[{}]", loaders.join(","))),
        ("include_changelog", "false".to_string()),
    ]
}

fn parse_modrinth_projects(body: &Value) -> Vec<Project> {
    list(body, "hits")
        .map(|hit| Project {
            provider: Provider::Modrinth,
            id: text(hit, "project_id"),
            title: text(hit, "title"),
            description: text(hit, "description"),
            author: text(hit, "author"),
            downloads: hit.get("downloads").and_then(Value::as_u64).unwrap_or(0),
        })
        .filter(|project| !project.id.is_empty())
        .collect()
}

fn parse_modrinth_versions(body: &Value) -> Vec<ModVersion> {
    body.as_array()
        .into_iter()
        .flatten()
        .filter_map(|version| {
            // The primary file, or the first one when none is marked.
            let files: Vec<&Value> = list(version, "files").collect();
            let file = files.iter().find(|f| f.get("primary").and_then(Value::as_bool) == Some(true)).or(files.first())?;
            Some(ModVersion {
                name: text(version, "version_number"),
                release_type: text(version, "version_type"),
                file_name: text(file, "filename"),
                url: Some(text(file, "url")),
                sha1: file.get("hashes").map(|h| text(h, "sha1")).filter(|h| !h.is_empty()),
            })
        })
        .collect()
}

fn curseforge_search_params(query: &Query) -> Vec<(&'static str, String)> {
    let mut params =
        vec![("gameId", "432".to_string()), ("classId", "6".to_string()), ("index", query.offset.to_string()), ("pageSize", "25".to_string())];
    if !query.text.is_empty() {
        params.push(("searchFilter", query.text.clone()));
    }
    // sortField 2 is popularity.
    params.extend([("sortField", "2".to_string()), ("sortOrder", "desc".to_string())]);
    if !query.loaders.is_empty() {
        let loaders: Vec<String> = query.loaders.iter().map(|l| l.curseforge().to_string()).collect();
        params.push(("modLoaderTypes", format!("[{}]", loaders.join(","))));
    }
    if !query.minecraft_version.is_empty() {
        params.push(("gameVersion", query.minecraft_version.clone()));
    }
    params
}

fn curseforge_file_params(minecraft_version: &str, loaders: &[Loader]) -> Vec<(&'static str, String)> {
    let mut params = vec![("pageSize", "10000".to_string()), ("gameVersion", minecraft_version.to_string())];
    // The API filters by one loader only.
    if let [single] = loaders {
        params.push(("modLoaderType", single.curseforge().to_string()));
    }
    params
}

fn parse_curseforge_projects(body: &Value) -> Vec<Project> {
    list(body, "data")
        .map(|hit| Project {
            provider: Provider::CurseForge,
            id: hit.get("id").and_then(Value::as_u64).map(|id| id.to_string()).unwrap_or_default(),
            title: text(hit, "name"),
            description: text(hit, "summary"),
            author: list(hit, "authors").next().map(|author| text(author, "name")).unwrap_or_default(),
            downloads: hit.get("downloadCount").and_then(Value::as_f64).unwrap_or(0.0) as u64,
        })
        .filter(|project| !project.id.is_empty())
        .collect()
}

fn parse_curseforge_files(body: &Value) -> Vec<ModVersion> {
    list(body, "data")
        .map(|file| ModVersion {
            name: text(file, "displayName"),
            release_type: match file.get("releaseType").and_then(Value::as_u64) {
                Some(2) => "beta",
                Some(3) => "alpha",
                _ => "release",
            }
            .to_string(),
            file_name: text(file, "fileName"),
            url: Some(text(file, "downloadUrl")).filter(|url| !url.is_empty()),
            // algo 1 is SHA-1, 2 is MD5.
            sha1: list(file, "hashes").find(|h| h.get("algo").and_then(Value::as_u64) == Some(1)).map(|h| text(h, "value")),
        })
        .collect()
}

async fn get(
    client: &Client,
    url: &str,
    params: &[(&'static str, String)],
    api_key: Option<&str>,
) -> Result<Value> {
    let failed = |e: reqwest::Error| Error::ModPlatform(format!("request failed: {}", e.without_url()));
    let mut request = client.http().get(url).query(params).header("Accept", "application/json");
    if let Some(key) = api_key {
        request = request.header("x-api-key", key);
    }
    let response = request.send().await.and_then(|r| r.error_for_status()).map_err(failed)?;
    response.json().await.map_err(failed)
}

fn need_key(api_key: Option<&str>) -> Result<&str> {
    api_key.ok_or_else(|| Error::ModPlatform("CurseForge needs an API key. See the README for how to set one.".into()))
}

pub async fn search(client: &Client, provider: Provider, api_key: Option<&str>, query: &Query) -> Result<Vec<Project>> {
    match provider {
        Provider::Modrinth => {
            let body = get(client, &format!("{MODRINTH}/search"), &modrinth_search_params(query), None).await?;
            Ok(parse_modrinth_projects(&body))
        }
        Provider::CurseForge => {
            let key = need_key(api_key)?;
            let body = get(client, &format!("{CURSEFORGE}/mods/search"), &curseforge_search_params(query), Some(key)).await?;
            Ok(parse_curseforge_projects(&body))
        }
    }
}

/// The files of a project that fit the instance, newest first.
pub async fn versions(
    client: &Client,
    project: &Project,
    api_key: Option<&str>,
    minecraft_version: &str,
    loaders: &[Loader],
) -> Result<Vec<ModVersion>> {
    // The id becomes part of the URL path.
    if !project.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(Error::ModPlatform(format!("unexpected project id '{}'", project.id)));
    }
    match project.provider {
        Provider::Modrinth => {
            let url = format!("{MODRINTH}/project/{}/version", project.id);
            Ok(parse_modrinth_versions(&get(client, &url, &modrinth_version_params(minecraft_version, loaders), None).await?))
        }
        Provider::CurseForge => {
            let key = need_key(api_key)?;
            let url = format!("{CURSEFORGE}/mods/{}/files", project.id);
            Ok(parse_curseforge_files(&get(client, &url, &curseforge_file_params(minecraft_version, loaders), Some(key)).await?))
        }
    }
}

/// Downloads the file into the mods folder and returns its path.
pub async fn install(client: &Client, version: &ModVersion, mods_dir: &Path) -> Result<PathBuf> {
    let url = version.url.as_deref().ok_or_else(|| {
        Error::ModPlatform(format!("The author of '{}' does not allow downloads outside the CurseForge app.", version.name))
    })?;
    if !url.starts_with("https://") {
        return Err(Error::ModPlatform(format!("refusing download from a non-HTTPS address for '{}'", version.name)));
    }
    // The name comes from the server; keep it a plain file name.
    let file_name = prism_core::fs::remove_invalid_filename_chars(&version.file_name);
    if file_name.trim_matches('.').is_empty() {
        return Err(Error::ModPlatform("the mod has no usable file name".into()));
    }
    let path = mods_dir.join(file_name);
    client.download(&Download { url: url.to_string(), path: path.clone(), sha1: version.sha1.clone() }).await?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query() -> Query {
        Query { text: "sodium".into(), minecraft_version: "26.3".into(), loaders: vec![Loader::Quilt, Loader::Fabric], offset: 25 }
    }

    #[test]
    fn modrinth_requests() {
        let params = modrinth_search_params(&query());
        assert_eq!(params[0], ("offset", "25".to_string()));
        assert_eq!(
            params.last().expect("facets").1,
            r#"[["categories:quilt","categories:fabric"],["versions:26.3"],["project_type:mod"]]"#
        );
        assert_eq!(modrinth_facets(&Query::default()), r#"[["project_type:mod"]]"#);
        assert_eq!(modrinth_version_params("26.3", &[Loader::Fabric])[..2], [
            ("game_versions", r#"["26.3"]"#.to_string()),
            ("loaders", r#"["fabric"]"#.to_string()),
        ]);
    }

    #[test]
    fn modrinth_responses() {
        let hits: Value = serde_json::from_str(
            r#"{"hits":[{"project_id":"AANobbMI","slug":"sodium","title":"Sodium","description":"Fast","author":"jellysquid3","downloads":90000000}],"total_hits":1}"#,
        )
        .expect("json");
        let projects = parse_modrinth_projects(&hits);
        assert_eq!(projects.len(), 1);
        assert_eq!((projects[0].id.as_str(), projects[0].title.as_str(), projects[0].downloads), ("AANobbMI", "Sodium", 90000000));

        let versions: Value = serde_json::from_str(
            r#"[{"version_number":"mc26.3-0.7.0-fabric","version_type":"release","files":[
                   {"url":"https://cdn.modrinth.com/a/sources.jar","filename":"sodium-sources.jar","primary":false,"hashes":{"sha1":"aa","sha512":"x"}},
                   {"url":"https://cdn.modrinth.com/a/sodium.jar","filename":"sodium.jar","primary":true,"hashes":{"sha1":"bb","sha512":"y"}}]},
                {"version_number":"empty","version_type":"beta","files":[]}]"#,
        )
        .expect("json");
        let parsed = parse_modrinth_versions(&versions);
        assert_eq!(parsed.len(), 1, "a version without files is skipped");
        assert_eq!((parsed[0].file_name.as_str(), parsed[0].sha1.as_deref()), ("sodium.jar", Some("bb")));
    }

    #[test]
    fn curseforge_requests_and_responses() {
        let params = curseforge_search_params(&query());
        assert!(params.contains(&("modLoaderTypes", "[5,4]".to_string())));
        assert!(params.contains(&("searchFilter", "sodium".to_string())) && params.contains(&("gameVersion", "26.3".to_string())));
        assert!(curseforge_file_params("26.3", &[Loader::Forge]).contains(&("modLoaderType", "1".to_string())));
        assert!(!curseforge_file_params("26.3", &[Loader::Quilt, Loader::Fabric]).iter().any(|(k, _)| *k == "modLoaderType"));

        let hits: Value = serde_json::from_str(
            r#"{"data":[{"id":238222,"name":"Just Enough Items (JEI)","summary":"View Items","authors":[{"name":"mezz"}],"downloadCount":4.0e8}]}"#,
        )
        .expect("json");
        let projects = parse_curseforge_projects(&hits);
        assert_eq!((projects[0].id.as_str(), projects[0].author.as_str(), projects[0].downloads), ("238222", "mezz", 400000000));

        let files: Value = serde_json::from_str(
            r#"{"data":[{"displayName":"jei-1.jar","fileName":"jei-1.jar","downloadUrl":"https://edge.forgecdn.net/files/1/jei-1.jar","releaseType":2,
                         "hashes":[{"value":"md5md5","algo":2},{"value":"sha1sha1","algo":1}]},
                        {"displayName":"blocked","fileName":"b.jar","downloadUrl":null,"releaseType":1,"hashes":[]}]}"#,
        )
        .expect("json");
        let parsed = parse_curseforge_files(&files);
        assert_eq!((parsed[0].release_type.as_str(), parsed[0].sha1.as_deref()), ("beta", Some("sha1sha1")));
        assert_eq!(parsed[1].url, None);
    }

    #[tokio::test]
    async fn install_refuses_what_it_cannot_trust() {
        let dir = tempfile::tempdir().expect("tempdir");
        let client = Client::new().expect("client");
        let version = |url: Option<&str>, file: &str| ModVersion {
            name: "x".into(),
            release_type: "release".into(),
            file_name: file.into(),
            url: url.map(str::to_string),
            sha1: None,
        };
        let blocked = install(&client, &version(None, "a.jar"), dir.path()).await.expect_err("no url");
        assert!(blocked.to_string().contains("does not allow downloads"));
        assert!(install(&client, &version(Some("http://example.invalid/a.jar"), "a.jar"), dir.path()).await.is_err());
        assert!(install(&client, &version(Some("https://example.invalid/a.jar"), ".."), dir.path()).await.is_err());

        let no_key = search(&client, Provider::CurseForge, None, &Query::default()).await.expect_err("needs key");
        assert!(no_key.to_string().contains("API key"));
        let odd = Project { provider: Provider::Modrinth, id: "../x".into(), title: String::new(), description: String::new(), author: String::new(), downloads: 0 };
        assert!(versions(&client, &odd, None, "26.3", &[]).await.is_err());
    }
}
