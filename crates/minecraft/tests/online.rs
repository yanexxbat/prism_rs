//! Tests against the live metadata server and Modrinth. They need network access:
//!
//! ```text
//! cargo test -p prism-minecraft --test online -- --ignored
//! ```

use prism_minecraft::loaders::{loader_versions, Loader};
use prism_minecraft::meta::Meta;
use prism_minecraft::modplatform::{self, Provider, Query};
use prism_minecraft::pack_profile::create_instance;

#[tokio::test]
#[ignore = "needs network"]
async fn every_loader_resolves_to_a_launchable_component_list() {
    let dir = tempfile::tempdir().expect("tempdir");
    let meta = Meta::new(prism_net::Client::new().expect("client"), dir.path(), "");
    // 1.20.1 has all four loaders.
    for loader in Loader::ALL {
        let versions = loader_versions(&meta, loader, "1.20.1").await.expect("loader versions");
        let newest = versions.first().unwrap_or_else(|| panic!("{} has no version for 1.20.1", loader.name()));
        let name = format!("{}-test", loader.name());
        let instance = create_instance(&meta, &dir.path().join("instances"), &name, "1.20.1", Some((loader.uid(), &newest.version)))
            .await
            .unwrap_or_else(|e| panic!("{}: {e}", loader.name()));

        let mut pack = prism_minecraft::pack_profile::PackProfile::load(&instance.root).expect("load");
        pack.resolve(&meta, false, true).await.expect("launch check");
        assert_eq!(Loader::of(&pack), Some(loader));
        let uids: Vec<&str> = pack.components.iter().map(|c| c.uid.as_str()).collect();
        println!("{}: {uids:?}", loader.name());
        assert!(uids.contains(&"net.minecraft") && uids.contains(&"org.lwjgl3"));
    }
    assert!(loader_versions(&meta, Loader::Forge, "0.0.0").await.expect("list").is_empty());
}

#[tokio::test]
#[ignore = "needs network"]
async fn modrinth_search_and_install() {
    let dir = tempfile::tempdir().expect("tempdir");
    let client = prism_net::Client::new().expect("client");
    let query = Query { text: "sodium".into(), minecraft_version: "1.20.1".into(), loaders: vec![Loader::Fabric], offset: 0 };
    let projects = modplatform::search(&client, Provider::Modrinth, None, &query).await.expect("search");
    let sodium = projects.iter().find(|p| p.title == "Sodium").expect("Sodium in the results");

    let versions = modplatform::versions(&client, sodium, None, "1.20.1", &[Loader::Fabric]).await.expect("versions");
    let newest = versions.first().expect("a version for 1.20.1");
    assert!(newest.sha1.is_some(), "Modrinth always sends a SHA-1");
    let path = modplatform::install(&client, newest, &dir.path().join("mods")).await.expect("install");
    let data = std::fs::read(&path).expect("read mod");
    assert_eq!(Some(prism_net::sha1_hex(&data)), newest.sha1);
    println!("installed {} ({} bytes)", path.display(), data.len());
}
