//! Drives the real UI code without a window: clicks go through egui's accessibility tree.

use crate::app::App;
use crate::Cli;
use eframe::egui::accesskit::Role;
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

type Ui<'a> = Harness<'a, App>;

fn harness(runtime: &tokio::runtime::Runtime, data_dir: &Path) -> Ui<'static> {
    let app = App::new(runtime.handle().clone(), data_dir.to_path_buf(), Cli::default()).expect("app");
    let mut ui = Harness::builder().with_size([960.0, 600.0]).build_state(|ctx, app: &mut App| app.ui(ctx), app);
    ui.step();
    ui
}

fn click(ui: &mut Ui, role: Role, label: &str) {
    ui.get_by_role_and_label(role, label).click();
    ui.step();
    ui.step();
}

fn click_containing(ui: &mut Ui, text: &str) {
    ui.get_by_label_contains(text).click();
    ui.step();
    ui.step();
}

/// Types into the n-th text field on screen.
fn type_into(ui: &mut Ui, index: usize, text: &str) {
    ui.get_all_by_role(Role::TextInput).nth(index).expect("text field").focus();
    ui.step();
    ui.get_all_by_role(Role::TextInput).nth(index).expect("text field").type_text(text);
    ui.step();
    ui.step();
}

/// Runs frames until `done` holds; background work reports through the app's message queue.
fn wait_for(ui: &mut Ui, what: &str, seconds: u64, done: impl Fn(&Ui) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        ui.step();
        if done(ui) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}; status: {}", ui.state().status);
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn dialogs_work_without_network() {
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    let dir = tempfile::tempdir().expect("tempdir");
    let instance = prism_core::instance::create_instance(&dir.path().join("instances"), "Local").expect("instance");
    std::fs::write(
        instance.root.join("mmc-pack.json"),
        r#"{"components":[{"uid":"net.minecraft","version":"1.20.1","important":true}],"formatVersion":1}"#,
    )
    .expect("pack");
    let mut ui = harness(&runtime, dir.path());

    // Instance list and selection.
    click_containing(&mut ui, "Local   [1.20.1]");
    assert_eq!(ui.state().selected.as_deref(), Some("Local"));

    // Launching without an account opens the account window instead.
    click(&mut ui, Role::Button, "Launch");
    assert!(ui.state().status.contains("No account"), "{}", ui.state().status);
    assert!(ui.state().windows.accounts_open);
    assert!(ui.state().running.is_empty());

    // An offline account is refused while no Microsoft account owns the game.
    type_into(&mut ui, 0, "Steve");
    click(&mut ui, Role::Button, "Add offline account");
    assert!(ui.state().status.contains("You must add a Microsoft account"), "{}", ui.state().status);
    assert!(ui.state().accounts.accounts.is_empty());
    ui.state_mut().windows.close_all();
    ui.step();

    // Settings are written to prismlauncher.cfg.
    click(&mut ui, Role::Button, "Settings");
    click(&mut ui, Role::CheckBox, "Start maximized");
    click(&mut ui, Role::Button, "Save");
    assert_eq!(ui.state().status, "Settings saved.");
    let cfg = std::fs::read_to_string(dir.path().join("prismlauncher.cfg")).expect("cfg");
    assert!(cfg.contains("LaunchMaximized=true") && cfg.contains("MaxMemAlloc=4096"), "{cfg}");

    // The instance editor writes override flags to instance.cfg.
    click(&mut ui, Role::Button, "Edit");
    click(&mut ui, Role::CheckBox, "Use memory limits of its own");
    click(&mut ui, Role::Button, "Save");
    let cfg = std::fs::read_to_string(instance.root.join("instance.cfg")).expect("cfg");
    assert!(cfg.contains("OverrideMemory=true") && cfg.contains("OverrideJavaLocation=false"), "{cfg}");

    // Deleting asks first; cancelling keeps the folder.
    click(&mut ui, Role::Button, "Delete");
    assert!(ui.query_by_label_contains("to the recycle bin?").is_some());
    click(&mut ui, Role::Button, "Cancel");
    assert!(instance.root.is_dir() && ui.state().windows.confirm_delete.is_none());

    // The mod browser says why it cannot search for a vanilla instance.
    click(&mut ui, Role::Button, "Mods");
    assert!(ui.query_by_label_contains("no mod loader").is_some());
}

/// The whole path a user takes: create a Fabric instance, install a mod, launch, read the log, kill.
///
/// ```text
/// cargo test -p prism-app full_flow -- --ignored --nocapture
/// ```
/// Uses the same folders as `crates/minecraft/tests/launch.rs` and an offline account fixture.
#[test]
#[ignore = "needs network and starts the real game"]
fn full_flow_with_the_real_game() {
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    let data_dir = std::env::var_os("PRISM_RS_TEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../_testdata"));
    let data_dir = std::path::absolute(data_dir).expect("absolute");
    std::fs::create_dir_all(&data_dir).expect("data dir");
    let instance_root = data_dir.join("instances").join("gui-fabric");
    let _ = std::fs::remove_dir_all(&instance_root);
    if let Ok(java_dir) = std::env::var("PRISM_RS_TEST_JAVA_DIR") {
        let mut global = prism_core::settings::Settings::load(&data_dir.join("prismlauncher.cfg")).expect("settings");
        global.set("JavaDir", java_dir);
        global.save().expect("save settings");
    }
    // The UI refuses to create this account without an owner; the fixture stands in for one.
    let accounts = data_dir.join("accounts.json");
    if !accounts.exists() {
        let fixture = r#"{"accounts":[{"active":true,"profile":{"capes":[],"id":"0f2a6f6f8a4b3c0e9d7e5a1b2c3d4e5f","name":"prism_rs_test","skin":{"id":"","url":"","variant":""}},"type":"Offline","ygg":{"iat":1790000000,"token":"0"}}],"formatVersion":3}"#;
        std::fs::write(&accounts, fixture).expect("accounts fixture");
    }
    let mut ui = harness(&runtime, &data_dir);

    // New instance with Fabric.
    click(&mut ui, Role::Button, "Add Instance");
    wait_for(&mut ui, "the Minecraft version list", 60, |ui| ui.query_all_by_label_contains("   release   ").next().is_some());
    type_into(&mut ui, 0, "gui-fabric");
    click(&mut ui, Role::Button, "Fabric");
    wait_for(&mut ui, "the Fabric version list", 60, |ui| ui.query_all_by_label("Loader version").next().is_some());
    click(&mut ui, Role::Button, "Create");
    wait_for(&mut ui, "the new instance", 120, |ui| ui.state().row("gui-fabric").is_some());
    let row = ui.state().row("gui-fabric").expect("row");
    assert_eq!(row.loader, Some(prism_minecraft::loaders::Loader::Fabric));
    println!("created gui-fabric for Minecraft {}", row.minecraft);

    // Mod browser: search Modrinth, pick Sodium, install the newest file.
    click_containing(&mut ui, "gui-fabric   [");
    click(&mut ui, Role::Button, "Mods");
    type_into(&mut ui, 0, "sodium");
    click(&mut ui, Role::Button, "Search");
    wait_for(&mut ui, "search results", 60, |ui| ui.query_all_by_label_contains("Sodium  by ").next().is_some());
    ui.get_all_by_label_contains("Sodium  by ").next().expect("Sodium").click();
    wait_for(&mut ui, "the file list", 60, |ui| ui.query_all_by_role_and_label(Role::Button, "Install").next().is_some());
    ui.get_all_by_role_and_label(Role::Button, "Install").next().expect("Install").click();
    let mods_dir = instance_root.join("minecraft").join("mods");
    let jar_in = |dir: &Path| {
        std::fs::read_dir(dir).into_iter().flatten().flatten().any(|e| e.path().extension().is_some_and(|x| x == "jar"))
    };
    wait_for(&mut ui, "the mod download", 120, |_| jar_in(&mods_dir));
    wait_for(&mut ui, "the installed list", 10, |ui| ui.query_by_label_contains("Installed (1)").is_some());
    println!("status: {}", ui.state().status);
    ui.state_mut().windows.close_all();
    ui.step();

    // Launch, watch the log window, kill.
    click(&mut ui, Role::Button, "Launch");
    assert!(ui.state().running.contains_key("gui-fabric"), "{}", ui.state().status);
    assert!(ui.query_by_label("Log of gui-fabric").is_some(), "the log window opens on launch");
    let log_has = |ui: &Ui, text: &str| ui.state().running["gui-fabric"].lines.iter().any(|line| line.contains(text));
    wait_for(&mut ui, "the game window", 400, |ui| log_has(ui, "Backend library") || ui.state().running["gui-fabric"].finished);
    let lines = ui.state().running["gui-fabric"].lines.clone();
    for line in &lines {
        println!("{}", line.chars().take(300).collect::<String>());
    }
    assert!(log_has(&ui, "Backend library"), "the game did not reach its window");
    assert!(log_has(&ui, "with Fabric Loader"));
    assert!(lines.iter().any(|l| l.starts_with("  [✔] sodium")), "the installed mod is listed in the launch log");
    assert!(!lines.iter().any(|l| l.contains("0f2a6f6f8a4b3c0e9d7e5a1b2c3d4e5f")), "the profile id is censored");

    ui.state_mut().windows.log = None;
    ui.step();
    click(&mut ui, Role::Button, "Kill");
    wait_for(&mut ui, "the game to exit", 60, |ui| ui.state().running["gui-fabric"].finished);
    assert!(!instance_root.join("natives").exists());
}
