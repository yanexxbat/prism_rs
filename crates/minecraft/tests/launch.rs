//! Real launches of the current Minecraft release, vanilla and with Fabric.
//!
//! These need network access, a Java the game accepts and a desktop session, so they
//! are ignored by default. Run them one at a time:
//!
//! ```text
//! cargo test -p prism-minecraft --test launch -- --ignored --nocapture --test-threads=1
//! ```
//!
//! `PRISM_RS_TEST_DIR` sets the data folder (default: `_testdata` in the workspace).
//! `PRISM_RS_TEST_JAVA_DIR` sets the folder scanned for Java runtimes (the `JavaDir` setting).
//!
//! The session is an offline one built here; the account list is not involved.

use prism_auth::AuthSession;
use prism_core::instance::Instance;
use prism_core::settings::Settings;
use prism_minecraft::launch::{self, LaunchRequest};
use prism_minecraft::meta::Meta;
use prism_minecraft::pack_profile::create_instance;
use std::path::PathBuf;
use std::time::Duration;

/// Log lines that only appear once the game has opened its window and is loading resources.
const STARTED: [&str; 3] = ["Backend library", "Sound engine started", "OpenAL initialized"];

async fn launch_latest_release(loader_uid: Option<&str>, must_log: &[&str]) {
    let data_dir = std::env::var_os("PRISM_RS_TEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../_testdata"));
    let data_dir = std::path::absolute(data_dir).expect("absolute data dir");
    let mut global = Settings::load(&data_dir.join("prismlauncher.cfg")).expect("global settings");
    if let Ok(java_dir) = std::env::var("PRISM_RS_TEST_JAVA_DIR") {
        global.set("JavaDir", java_dir);
    }

    let client = prism_net::Client::new().expect("client");
    let meta = Meta::new(client.clone(), &data_dir, "");
    let releases = meta.version_list("net.minecraft", false).await.expect("version list");
    let release = releases.versions.iter().find(|v| v.kind == "release").expect("a release").version.clone();

    let loader = match loader_uid {
        Some(uid) => {
            let versions = meta.version_list(uid, false).await.expect("loader versions");
            let version = versions.versions.iter().find(|v| v.recommended).or(versions.versions.first()).expect("a loader");
            Some((uid.to_string(), version.version.clone()))
        }
        None => None,
    };
    let name = format!("test-{}-{release}", if loader.is_some() { "fabric" } else { "vanilla" });
    let instances_dir = data_dir.join("instances");
    let mut instance = match Instance::load(&instances_dir.join(&name)) {
        Ok(existing) if existing.root.join("mmc-pack.json").is_file() => existing,
        _ => {
            let loader = loader.as_ref().map(|(uid, version)| (uid.as_str(), version.as_str()));
            create_instance(&meta, &instances_dir, &name, &release, loader).await.expect("create instance")
        }
    };

    // An offline account in an online launch: files are downloaded, the token is "0".
    let session = AuthSession { offline: false, ..AuthSession::offline("prism_rs_test") };
    let (log, mut lines) = tokio::sync::mpsc::unbounded_channel();
    let request = LaunchRequest {
        data_dir: &data_dir,
        global: &global,
        instance: &mut instance,
        session: &session,
        screen_size: None,
        target: None,
    };
    let prepared = launch::prepare(&client, &meta, request, &log).await.expect("prepare launch");
    let command_line = prepared.censored_command_line();
    println!("COMMAND LINE: {command_line}");
    std::fs::write(data_dir.join(format!("{name}.cmdline.txt")), &command_line).expect("write command line");

    let (kill, killed) = tokio::sync::oneshot::channel();
    let game = tokio::spawn(async move { launch::run(&prepared, &log, killed).await });

    let mut seen: Vec<String> = Vec::new();
    let watch = async {
        while let Some(line) = lines.recv().await {
            println!("{line}");
            let started = STARTED.iter().any(|marker| line.contains(marker));
            seen.push(line);
            if started {
                return true;
            }
        }
        false
    };
    let started = tokio::time::timeout(Duration::from_secs(300), watch).await.unwrap_or(false);
    let _ = kill.send(());
    let _ = tokio::time::timeout(Duration::from_secs(30), game).await;

    assert!(started, "the game did not reach its window within the time limit");
    for needle in must_log {
        assert!(seen.iter().any(|line| line.contains(needle)), "log has no line with '{needle}'");
    }
    assert!(!instance.natives_dir().exists(), "natives must be removed after the game exits");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "starts the real game"]
async fn vanilla_release_starts() {
    launch_latest_release(None, &["Main class: net.minecraft.client.main.Main"]).await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "starts the real game"]
async fn fabric_release_starts() {
    launch_latest_release(Some("net.fabricmc.fabric-loader"), &["with Fabric Loader"]).await;
}
