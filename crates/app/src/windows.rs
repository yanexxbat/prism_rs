//! The dialogs: new instance, accounts, settings, instance editor, mod browser, log.
//! Simplified counterparts of `ui/dialogs` and `ui/pages` in the original.

use crate::app::{App, Msg};
use eframe::egui;
use prism_auth::{msa, AccountList, AccountState, AccountType};
use prism_core::settings::{parse_bool, Effective};
use prism_minecraft::java::{self, JavaInstall};
use prism_minecraft::loaders::{loader_versions, Loader};
use prism_minecraft::meta::VersionEntry;
use prism_minecraft::modplatform::{self, ModVersion, Project, Provider, Query};
use prism_minecraft::pack_profile::{create_instance, PackProfile};
use std::path::{Path, PathBuf};

struct NewInstance {
    name: String,
    versions: Vec<VersionEntry>,
    filter: String,
    snapshots: bool,
    minecraft: String,
    loader: Option<Loader>,
    loader_versions: Vec<VersionEntry>,
    loader_version: String,
}

struct Login {
    code: Option<msa::DeviceCode>,
    task: tokio::task::JoinHandle<()>,
}

struct SettingsDraft {
    java_path: String,
    min_mem: i64,
    max_mem: i64,
    width: i64,
    height: i64,
    maximized: bool,
    jvm_args: String,
    auto_java: bool,
    client_id: String,
    curseforge_key: String,
    javas: Vec<JavaInstall>,
}

struct InstanceEdit {
    id: String,
    name: String,
    override_java: bool,
    java_path: String,
    override_memory: bool,
    min_mem: i64,
    max_mem: i64,
    override_args: bool,
    jvm_args: String,
    override_window: bool,
    maximized: bool,
    width: i64,
    height: i64,
    minecraft: String,
    components: Vec<String>,
    loader: Option<Loader>,
    loader_versions: Vec<VersionEntry>,
    loader_version: String,
    /// The version list for `loader` has been asked for.
    loader_requested: bool,
}

struct ModBrowser {
    instance_id: String,
    mods_dir: PathBuf,
    minecraft: String,
    loaders: Vec<Loader>,
    provider: Provider,
    text: String,
    results: Vec<Project>,
    selected: Option<usize>,
    versions: Vec<ModVersion>,
    busy: bool,
}

#[derive(Default)]
pub struct Windows {
    pub accounts_open: bool,
    pub log: Option<String>,
    pub confirm_delete: Option<String>,
    new_instance: Option<NewInstance>,
    login: Option<Login>,
    offline_name: String,
    remove_armed: Option<usize>,
    settings: Option<SettingsDraft>,
    edit: Option<InstanceEdit>,
    mods: Option<ModBrowser>,
}

/// The recommended entry, else the first.
fn default_version(versions: &[VersionEntry]) -> String {
    versions.iter().find(|v| v.recommended).or(versions.first()).map(|v| v.version.clone()).unwrap_or_default()
}

impl Windows {
    /// Lets a test move on to the next dialog.
    #[cfg(test)]
    pub fn close_all(&mut self) {
        *self = Windows::default();
    }

    /// Results of background work that belong to a dialog.
    pub fn handle(&mut self, msg: Msg, accounts: &mut AccountList, status: &mut String) {
        match msg {
            Msg::MinecraftVersions(versions) => {
                if let Some(dialog) = &mut self.new_instance {
                    dialog.minecraft = versions.iter().find(|v| v.kind == "release").map(|v| v.version.clone()).unwrap_or_default();
                    dialog.versions = versions;
                }
            }
            Msg::LoaderVersions(loader, minecraft, versions) => {
                if let Some(dialog) = self.new_instance.as_mut().filter(|d| d.loader == Some(loader) && d.minecraft == minecraft) {
                    dialog.loader_version = default_version(&versions);
                    dialog.loader_versions = versions;
                } else if let Some(dialog) = self.edit.as_mut().filter(|d| d.loader == Some(loader) && d.minecraft == minecraft) {
                    if !versions.iter().any(|v| v.version == dialog.loader_version) {
                        dialog.loader_version = default_version(&versions);
                    }
                    dialog.loader_versions = versions;
                }
            }
            Msg::DeviceCode(code) => {
                if let Some(login) = &mut self.login {
                    login.code = Some(code);
                }
            }
            Msg::LoggedIn(result) => {
                self.login = None;
                *status = match result {
                    Ok(account) => {
                        let name = account.profile_name().to_string();
                        accounts.add(*account);
                        match accounts.save() {
                            Ok(()) => format!("Logged in as {name}."),
                            Err(e) => format!("Could not save accounts: {e}"),
                        }
                    }
                    Err(error) => format!("Login failed: {error}"),
                };
            }
            Msg::Javas(javas) => {
                *status = format!("Found {} Java installation(s).", javas.len());
                if let Some(draft) = &mut self.settings {
                    draft.javas = javas;
                }
            }
            Msg::ModResults(result) => {
                if let Some(browser) = &mut self.mods {
                    browser.busy = false;
                    browser.selected = None;
                    browser.versions.clear();
                    match result {
                        Ok(projects) => browser.results = projects,
                        Err(error) => *status = error,
                    }
                }
            }
            Msg::ModVersions(result) => {
                if let Some(browser) = &mut self.mods {
                    browser.busy = false;
                    match result {
                        Ok(versions) => browser.versions = versions,
                        Err(error) => *status = error,
                    }
                }
            }
            Msg::Status(_) | Msg::Reload(_) | Msg::Refreshed { .. } => {}
        }
    }
}

/// Files in a mods folder: (file name, enabled).
fn installed_mods(dir: &Path) -> Vec<(String, bool)> {
    let mut mods: Vec<(String, bool)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().is_file())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .map(|name| (name.clone(), !name.ends_with(".disabled")))
        .collect();
    mods.sort_by_key(|(name, _)| name.to_lowercase());
    mods
}

fn memory_field(ui: &mut egui::Ui, value: &mut i64) {
    ui.add(egui::DragValue::new(value).range(128..=1_048_576).suffix(" MiB"));
}

impl App {
    pub fn open_new_instance(&mut self, ctx: &egui::Context) {
        self.windows.new_instance = Some(NewInstance {
            name: String::new(),
            versions: Vec::new(),
            filter: String::new(),
            snapshots: false,
            minecraft: String::new(),
            loader: None,
            loader_versions: Vec::new(),
            loader_version: String::new(),
        });
        let meta = self.meta.clone();
        self.spawn(ctx, async move {
            match meta.version_list("net.minecraft", false).await {
                Ok(list) => Msg::MinecraftVersions(list.versions),
                Err(e) => Msg::Status(format!("Could not load the version list: {e}")),
            }
        });
    }

    pub fn open_settings(&mut self) {
        let get = Effective { global: &self.global, instance: None };
        self.windows.settings = Some(SettingsDraft {
            java_path: get.get("JavaPath"),
            min_mem: get.get_int("MinMemAlloc"),
            max_mem: get.get_int("MaxMemAlloc"),
            width: get.get_int("MinecraftWinWidth"),
            height: get.get_int("MinecraftWinHeight"),
            maximized: get.get_bool("LaunchMaximized"),
            jvm_args: get.get("JvmArgs"),
            auto_java: get.get_bool("AutomaticJavaSwitch"),
            client_id: get.get("MSAClientIDOverride"),
            curseforge_key: get.get("FlameKeyOverride"),
            javas: Vec::new(),
        });
    }

    pub fn open_edit(&mut self, id: &str) {
        let Some(row) = self.row(id) else { return };
        let own = &row.instance.settings;
        let flag = |key: &str| own.get(key).is_some_and(parse_bool);
        // What is in effect now, so a newly ticked override starts from the current value.
        let get = Effective { global: &self.global, instance: Some(own) };
        let pack = PackProfile::load(&row.instance.root).ok();
        let components = pack.iter().flat_map(|p| &p.components).map(|c| format!("{} {}", c.name(), c.version)).collect();
        let loader_version = row.loader.and_then(|l| pack.as_ref()?.component(l.uid()).map(|c| c.version.clone()));
        self.windows.edit = Some(InstanceEdit {
            id: id.to_string(),
            name: row.instance.name(),
            override_java: flag("OverrideJavaLocation"),
            java_path: get.get("JavaPath"),
            override_memory: flag("OverrideMemory"),
            min_mem: get.get_int("MinMemAlloc"),
            max_mem: get.get_int("MaxMemAlloc"),
            override_args: flag("OverrideJavaArgs"),
            jvm_args: get.get("JvmArgs"),
            override_window: flag("OverrideWindow"),
            maximized: get.get_bool("LaunchMaximized"),
            width: get.get_int("MinecraftWinWidth"),
            height: get.get_int("MinecraftWinHeight"),
            minecraft: row.minecraft.clone(),
            components,
            loader: row.loader,
            loader_versions: Vec::new(),
            loader_version: loader_version.unwrap_or_default(),
            loader_requested: false,
        });
    }

    pub fn open_mods(&mut self, id: &str) {
        let Some(row) = self.row(id) else { return };
        self.windows.mods = Some(ModBrowser {
            instance_id: id.to_string(),
            mods_dir: row.instance.mods_dir(),
            minecraft: row.minecraft.clone(),
            loaders: row.loader.map(|l| l.accepted(&row.minecraft)).unwrap_or_default(),
            provider: Provider::Modrinth,
            text: String::new(),
            results: Vec::new(),
            selected: None,
            versions: Vec::new(),
            busy: false,
        });
    }

    fn request_loader_versions(&self, ctx: &egui::Context, loader: Loader, minecraft: String) {
        let meta = self.meta.clone();
        self.spawn(ctx, async move {
            match loader_versions(&meta, loader, &minecraft).await {
                Ok(versions) => Msg::LoaderVersions(loader, minecraft, versions),
                Err(e) => Msg::Status(format!("Could not load {} versions: {e}", loader.name())),
            }
        });
    }

    pub fn show_windows(&mut self, ctx: &egui::Context) {
        self.new_instance_window(ctx);
        self.accounts_window(ctx);
        self.settings_window(ctx);
        self.edit_window(ctx);
        self.mods_window(ctx);
        self.log_window(ctx);
        self.delete_window(ctx);
    }

    /// A loader choice and its version list. Returns true when the loader was changed.
    fn loader_picker(ui: &mut egui::Ui, loader: &mut Option<Loader>, versions: &[VersionEntry], version: &mut String) -> bool {
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.label("Mod loader:");
            changed |= ui.selectable_value(loader, None, "None").changed();
            for candidate in Loader::ALL {
                changed |= ui.selectable_value(loader, Some(candidate), candidate.name()).changed();
            }
        });
        if loader.is_some() {
            if versions.is_empty() {
                ui.weak("No loader version for this Minecraft version (or still loading).");
            } else {
                egui::ComboBox::from_label("Loader version").selected_text(version.as_str()).show_ui(ui, |ui| {
                    for entry in versions {
                        ui.selectable_value(version, entry.version.clone(), &entry.version);
                    }
                });
            }
        }
        changed
    }

    fn new_instance_window(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.windows.new_instance.take() else { return };
        let (mut open, mut create, mut reload_loaders) = (true, false, false);
        egui::Window::new("New Instance").open(&mut open).default_size([440.0, 480.0]).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Name:");
                ui.add(egui::TextEdit::singleline(&mut dialog.name).hint_text(dialog.minecraft.as_str()));
            });
            ui.horizontal(|ui| {
                ui.label("Filter:");
                ui.text_edit_singleline(&mut dialog.filter);
                ui.checkbox(&mut dialog.snapshots, "Snapshots and old versions");
            });
            if dialog.versions.is_empty() {
                ui.spinner();
            }
            egui::ScrollArea::vertical().max_height(240.0).auto_shrink([false, false]).show(ui, |ui| {
                let shown = dialog.versions.iter().filter(|v| (dialog.snapshots || v.kind == "release") && v.version.contains(dialog.filter.trim()));
                for entry in shown {
                    let date = entry.release_time.get(..10).unwrap_or_default();
                    let text = format!("{}   {}   {date}", entry.version, entry.kind);
                    if ui.selectable_label(dialog.minecraft == entry.version, text).clicked() {
                        dialog.minecraft = entry.version.clone();
                        reload_loaders = true;
                    }
                }
            });
            ui.separator();
            reload_loaders |= Self::loader_picker(ui, &mut dialog.loader, &dialog.loader_versions, &mut dialog.loader_version);
            ui.separator();
            let ready = !dialog.minecraft.is_empty() && (dialog.loader.is_none() || !dialog.loader_version.is_empty());
            create = ui.add_enabled(ready, egui::Button::new("Create")).clicked();
        });
        if reload_loaders {
            dialog.loader_versions.clear();
            dialog.loader_version.clear();
            if let Some(loader) = dialog.loader {
                self.request_loader_versions(ctx, loader, dialog.minecraft.clone());
            }
        }
        if create {
            // An empty name means the version, as in the original.
            let name = if dialog.name.trim().is_empty() { dialog.minecraft.clone() } else { dialog.name.trim().to_string() };
            let (meta, dir, minecraft) = (self.meta.clone(), self.instances_dir(), dialog.minecraft.clone());
            let loader = dialog.loader.map(|l| (l.uid(), dialog.loader_version.clone()));
            self.status = format!("Creating {name}...");
            self.spawn(ctx, async move {
                let loader = loader.as_ref().map(|(uid, version)| (*uid, version.as_str()));
                match create_instance(&meta, &dir, &name, &minecraft, loader).await {
                    Ok(instance) => Msg::Reload(format!("Created instance {}.", instance.name())),
                    Err(e) => Msg::Status(format!("Could not create the instance: {e}")),
                }
            });
        } else if open {
            self.windows.new_instance = Some(dialog);
        }
    }

    fn accounts_window(&mut self, ctx: &egui::Context) {
        if !self.windows.accounts_open {
            return;
        }
        enum Action {
            Default(usize),
            Remove(usize),
            Refresh(usize),
            AddMicrosoft,
            AddOffline,
            CancelLogin,
        }
        let client_id = self.msa_client_id();
        let (mut open, mut action) = (true, None);
        let windows = &mut self.windows;
        let accounts = &self.accounts.accounts;
        egui::Window::new("Accounts").open(&mut open).default_width(520.0).show(ctx, |ui| {
            egui::Grid::new("accounts").num_columns(5).striped(true).show(ui, |ui| {
                for (index, account) in accounts.iter().enumerate() {
                    if ui.radio(account.active, "").on_hover_text("Default account").clicked() {
                        action = Some(Action::Default(index));
                    }
                    ui.label(account.profile_name());
                    ui.label(if account.kind == AccountType::Msa { "Microsoft" } else { "Offline" });
                    ui.label(match &account.state {
                        AccountState::Unchecked => "not checked yet".to_string(),
                        AccountState::Online => "ready".to_string(),
                        AccountState::Offline => "servers unreachable".to_string(),
                        AccountState::Expired => "expired, log in again".to_string(),
                        AccountState::Errored(reason) => reason.clone(),
                    });
                    ui.horizontal(|ui| {
                        if account.kind == AccountType::Msa && ui.add_enabled(client_id.is_some(), egui::Button::new("Refresh")).clicked() {
                            action = Some(Action::Refresh(index));
                        }
                        // Removing throws the stored login away, so it takes two clicks.
                        let armed = windows.remove_armed == Some(index);
                        if ui.button(if armed { "Really remove?" } else { "Remove" }).clicked() {
                            if armed {
                                action = Some(Action::Remove(index));
                            } else {
                                windows.remove_armed = Some(index);
                            }
                        }
                    });
                    ui.end_row();
                }
            });
            ui.separator();
            match &windows.login {
                Some(Login { code: Some(code), .. }) => {
                    ui.label(format!("Open {} in a browser and enter this code:", code.verification_uri));
                    ui.horizontal(|ui| {
                        ui.heading(&code.user_code);
                        if ui.button("Copy code").clicked() {
                            ui.ctx().copy_text(code.user_code.clone());
                        }
                        if ui.button("Open browser").clicked() {
                            ui.ctx().open_url(egui::OpenUrl::new_tab(&code.verification_uri));
                        }
                        if ui.button("Cancel").clicked() {
                            action = Some(Action::CancelLogin);
                        }
                    });
                }
                Some(_) => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Asking Microsoft for a login code...");
                    });
                }
                None => {
                    let button = ui
                        .add_enabled(client_id.is_some(), egui::Button::new("Add Microsoft account"))
                        .on_disabled_hover_text("Set a Microsoft client ID in Settings or in PRISM_RS_MSA_CLIENT_ID first.");
                    if button.clicked() {
                        action = Some(Action::AddMicrosoft);
                    }
                }
            }
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut windows.offline_name).hint_text("Player name").desired_width(160.0));
                if ui.add_enabled(!windows.offline_name.trim().is_empty(), egui::Button::new("Add offline account")).clicked() {
                    action = Some(Action::AddOffline);
                }
            });
        });
        self.windows.accounts_open = open;

        match action {
            Some(Action::Default(index)) => {
                self.accounts.set_default(index);
                self.save_accounts();
            }
            Some(Action::Remove(index)) => {
                self.windows.remove_armed = None;
                self.accounts.accounts.remove(index);
                self.save_accounts();
            }
            Some(Action::Refresh(index)) => {
                let (client, account) = (self.client.clone(), self.accounts.accounts[index].clone());
                if let Some(client_id) = client_id {
                    self.status = format!("Refreshing the login of {}...", account.profile_name());
                    self.spawn(ctx, async move {
                        let mut account = Box::new(account);
                        let result = msa::refresh(&client, &msa::Endpoints::default(), &client_id, &mut account).await;
                        Msg::Refreshed { index, account, error: result.err().map(|e| e.to_string()), then: None }
                    });
                }
            }
            Some(Action::AddMicrosoft) => {
                if let Some(client_id) = client_id {
                    let (client, tx, repaint) = (self.client.clone(), self.tx.clone(), ctx.clone());
                    let task = self.spawn(ctx, async move {
                        let show = |code: &msa::DeviceCode| {
                            let _ = tx.send(Msg::DeviceCode(code.clone()));
                            repaint.request_repaint();
                        };
                        let result = msa::login_with_device_code(&client, &msa::Endpoints::default(), &client_id, show).await;
                        Msg::LoggedIn(result.map(Box::new).map_err(|e| e.to_string()))
                    });
                    self.windows.login = Some(Login { code: None, task });
                }
            }
            Some(Action::AddOffline) => {
                let name = std::mem::take(&mut self.windows.offline_name);
                match self.accounts.add_offline(name.trim()) {
                    Ok(()) => self.save_accounts(),
                    Err(e) => self.status = e.to_string(),
                }
            }
            Some(Action::CancelLogin) => {
                if let Some(login) = self.windows.login.take() {
                    login.task.abort();
                }
            }
            None => {}
        }
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        let Some(mut draft) = self.windows.settings.take() else { return };
        let (mut open, mut save, mut detect) = (true, false, false);
        egui::Window::new("Settings").open(&mut open).default_width(520.0).show(ctx, |ui| {
            ui.strong("Java");
            ui.horizontal(|ui| {
                ui.label("Java path:");
                ui.add(egui::TextEdit::singleline(&mut draft.java_path).desired_width(330.0));
                detect = ui.button("Detect").clicked();
            });
            for found in &draft.javas {
                let text = format!("Java {} ({}, {})  {}", found.version, found.real_architecture, found.vendor, found.path.display());
                if ui.selectable_label(Path::new(&draft.java_path) == found.path, text).clicked() {
                    draft.java_path = prism_core::fs::qt_path(&found.path);
                }
            }
            ui.checkbox(&mut draft.auto_java, "Pick a compatible Java for each instance automatically");
            ui.horizontal(|ui| {
                ui.label("Memory, minimum:");
                memory_field(ui, &mut draft.min_mem);
                ui.label("maximum:");
                memory_field(ui, &mut draft.max_mem);
            });
            ui.horizontal(|ui| {
                ui.label("JVM arguments:");
                ui.add(egui::TextEdit::singleline(&mut draft.jvm_args).desired_width(360.0));
            });
            ui.separator();
            ui.strong("Game window");
            ui.horizontal(|ui| {
                ui.checkbox(&mut draft.maximized, "Start maximized");
                ui.label("Width:");
                ui.add_enabled(!draft.maximized, egui::DragValue::new(&mut draft.width).range(1..=65535));
                ui.label("Height:");
                ui.add_enabled(!draft.maximized, egui::DragValue::new(&mut draft.height).range(1..=65535));
            });
            ui.separator();
            ui.strong("Services");
            ui.horizontal(|ui| {
                ui.label("Microsoft client ID:");
                ui.add(egui::TextEdit::singleline(&mut draft.client_id).desired_width(300.0));
            });
            ui.horizontal(|ui| {
                ui.label("CurseForge API key:");
                ui.add(egui::TextEdit::singleline(&mut draft.curseforge_key).password(true).desired_width(300.0));
            });
            ui.weak("Both are stored in prismlauncher.cfg in the data folder. Environment variables take precedence.");
            ui.separator();
            save = ui.button("Save").clicked();
        });
        if detect {
            let java_dir = self.data_dir.join(self.global.get("JavaDir").unwrap_or("java"));
            self.status = "Looking for Java...".into();
            self.spawn(ctx, async move { Msg::Javas(java::detect(&java_dir).await) });
        }
        if save {
            let global = &mut self.global;
            global.set("JavaPath", draft.java_path.trim());
            global.set("MinMemAlloc", draft.min_mem);
            global.set("MaxMemAlloc", draft.max_mem);
            global.set("MinecraftWinWidth", draft.width);
            global.set("MinecraftWinHeight", draft.height);
            global.set("LaunchMaximized", draft.maximized);
            global.set("JvmArgs", draft.jvm_args.trim());
            global.set("AutomaticJavaSwitch", draft.auto_java);
            global.set("MSAClientIDOverride", draft.client_id.trim());
            global.set("FlameKeyOverride", draft.curseforge_key.trim());
            self.status = match global.save() {
                Ok(()) => "Settings saved.".into(),
                Err(e) => format!("Could not save settings: {e}"),
            };
        } else if open {
            self.windows.settings = Some(draft);
        }
    }

    fn edit_window(&mut self, ctx: &egui::Context) {
        let Some(mut edit) = self.windows.edit.take() else { return };
        let Some(row) = self.row(&edit.id) else { return };
        let (current_loader, root) = (row.loader, row.instance.root.clone());
        let (mut open, mut save, mut apply_loader, mut reload_loaders) = (true, false, false, false);
        egui::Window::new(format!("Edit {}", edit.id)).open(&mut open).default_width(520.0).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Name:");
                ui.text_edit_singleline(&mut edit.name);
            });
            ui.separator();
            ui.checkbox(&mut edit.override_java, "Use a Java installation of its own");
            ui.add_enabled(edit.override_java, egui::TextEdit::singleline(&mut edit.java_path).desired_width(480.0));
            ui.checkbox(&mut edit.override_memory, "Use memory limits of its own");
            ui.add_enabled_ui(edit.override_memory, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Minimum:");
                    memory_field(ui, &mut edit.min_mem);
                    ui.label("Maximum:");
                    memory_field(ui, &mut edit.max_mem);
                });
            });
            ui.checkbox(&mut edit.override_args, "Use JVM arguments of its own");
            ui.add_enabled(edit.override_args, egui::TextEdit::singleline(&mut edit.jvm_args).desired_width(480.0));
            ui.checkbox(&mut edit.override_window, "Use a window size of its own");
            ui.add_enabled_ui(edit.override_window, |ui| {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut edit.maximized, "Start maximized");
                    ui.add(egui::DragValue::new(&mut edit.width).range(1..=65535));
                    ui.label("x");
                    ui.add(egui::DragValue::new(&mut edit.height).range(1..=65535));
                });
            });
            save = ui.button("Save").clicked();
            ui.separator();
            ui.strong("Components");
            for component in &edit.components {
                ui.label(component);
            }
            reload_loaders = Self::loader_picker(ui, &mut edit.loader, &edit.loader_versions, &mut edit.loader_version);
            let ready = edit.loader.is_none() || !edit.loader_version.is_empty();
            let label = if edit.loader.is_none() { "Remove mod loader" } else { "Install this loader" };
            let changes = edit.loader != current_loader || edit.loader.is_some();
            apply_loader = ui.add_enabled(ready && changes, egui::Button::new(label)).clicked();
        });
        if save {
            if let Some(row) = self.rows.iter_mut().find(|row| row.instance.id == edit.id) {
                let own = &mut row.instance.settings;
                own.set("name", edit.name.trim());
                own.set("OverrideJavaLocation", edit.override_java);
                own.set("JavaPath", edit.java_path.trim());
                own.set("OverrideMemory", edit.override_memory);
                own.set("MinMemAlloc", edit.min_mem);
                own.set("MaxMemAlloc", edit.max_mem);
                own.set("OverrideJavaArgs", edit.override_args);
                own.set("JvmArgs", edit.jvm_args.trim());
                own.set("OverrideWindow", edit.override_window);
                own.set("LaunchMaximized", edit.maximized);
                own.set("MinecraftWinWidth", edit.width);
                own.set("MinecraftWinHeight", edit.height);
                self.status = match own.save() {
                    Ok(()) => "Instance saved.".into(),
                    Err(e) => format!("Could not save the instance: {e}"),
                };
            }
        }
        if reload_loaders {
            edit.loader_versions.clear();
            edit.loader_version.clear();
            edit.loader_requested = false;
        }
        if let Some(loader) = edit.loader.filter(|_| !edit.loader_requested) {
            edit.loader_requested = true;
            self.request_loader_versions(ctx, loader, edit.minecraft.clone());
        }
        if apply_loader {
            let (meta, loader, version) = (self.meta.clone(), edit.loader, edit.loader_version.trim().to_string());
            self.status = "Changing the mod loader...".into();
            self.spawn(ctx, async move {
                let work = async {
                    let mut pack = PackProfile::load(&root)?;
                    for other in Loader::ALL {
                        pack.remove(other.uid());
                    }
                    if let Some(loader) = loader {
                        pack.set_component_version(loader.uid(), &version, false);
                    }
                    pack.resolve(&meta, false, false).await?;
                    pack.save()
                };
                match work.await {
                    Ok(()) => Msg::Reload("Mod loader changed.".into()),
                    Err(e) => Msg::Status(format!("Could not change the mod loader: {e}")),
                }
            });
        } else if open && !save {
            self.windows.edit = Some(edit);
        }
    }

    fn mods_window(&mut self, ctx: &egui::Context) {
        let Some(mut browser) = self.windows.mods.take() else { return };
        let key = self.curseforge_key();
        let installed = installed_mods(&browser.mods_dir);
        let (mut open, mut search, mut pick, mut install) = (true, false, None, None);
        let (mut toggle, mut remove) = (None, None);
        egui::Window::new(format!("Mods of {}", browser.instance_id)).open(&mut open).default_size([760.0, 520.0]).show(ctx, |ui| {
            if browser.loaders.is_empty() {
                ui.colored_label(ui.visuals().warn_fg_color, "This instance has no mod loader. Install one under Edit first.");
            }
            ui.horizontal(|ui| {
                search |= ui.selectable_value(&mut browser.provider, Provider::Modrinth, "Modrinth").changed();
                ui.add_enabled_ui(key.is_some(), |ui| {
                    let changed = ui
                        .selectable_value(&mut browser.provider, Provider::CurseForge, "CurseForge")
                        .on_disabled_hover_text("Set a CurseForge API key in Settings or in PRISM_RS_CURSEFORGE_API_KEY first.")
                        .changed();
                    search |= changed;
                });
                let field = ui.add(egui::TextEdit::singleline(&mut browser.text).hint_text("Search mods").desired_width(260.0));
                search |= field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                search |= ui.button("Search").clicked();
                if browser.busy {
                    ui.spinner();
                }
            });
            ui.separator();
            ui.columns(2, |columns| {
                egui::ScrollArea::vertical().id_salt("results").max_height(260.0).auto_shrink([false, false]).show(&mut columns[0], |ui| {
                    for (index, project) in browser.results.iter().enumerate() {
                        let text = format!("{}  by {}  ({} downloads)", project.title, project.author, project.downloads);
                        if ui.selectable_label(browser.selected == Some(index), text).on_hover_text(&project.description).clicked() {
                            pick = Some(index);
                        }
                    }
                });
                egui::ScrollArea::vertical().id_salt("versions").max_height(260.0).auto_shrink([false, false]).show(&mut columns[1], |ui| {
                    if browser.selected.is_some() && browser.versions.is_empty() && !browser.busy {
                        ui.weak("No file for this Minecraft version and loader.");
                    }
                    for (index, version) in browser.versions.iter().enumerate() {
                        ui.horizontal(|ui| {
                            if ui.button("Install").clicked() {
                                install = Some(index);
                            }
                            ui.label(format!("{}  ({})", version.name, version.release_type));
                        });
                    }
                });
            });
            ui.separator();
            ui.strong(format!("Installed ({})", installed.len()));
            egui::ScrollArea::vertical().id_salt("installed").max_height(140.0).auto_shrink([false, false]).show(ui, |ui| {
                for (name, enabled) in &installed {
                    ui.horizontal(|ui| {
                        let mut on = *enabled;
                        if ui.checkbox(&mut on, name.trim_end_matches(".disabled")).changed() {
                            toggle = Some(name.clone());
                        }
                        if ui.small_button("Remove").clicked() {
                            remove = Some(name.clone());
                        }
                    });
                }
            });
        });

        if search {
            browser.busy = true;
            let (client, provider) = (self.client.clone(), browser.provider);
            let query = Query { text: browser.text.trim().to_string(), minecraft_version: browser.minecraft.clone(), loaders: browser.loaders.clone(), offset: 0 };
            let key = key.clone();
            self.spawn(ctx, async move {
                Msg::ModResults(modplatform::search(&client, provider, key.as_deref(), &query).await.map_err(|e| e.to_string()))
            });
        }
        if let Some(index) = pick {
            browser.selected = Some(index);
            browser.versions.clear();
            browser.busy = true;
            let (client, project, key) = (self.client.clone(), browser.results[index].clone(), key.clone());
            let (minecraft, loaders) = (browser.minecraft.clone(), browser.loaders.clone());
            self.spawn(ctx, async move {
                let result = modplatform::versions(&client, &project, key.as_deref(), &minecraft, &loaders).await;
                Msg::ModVersions(result.map_err(|e| e.to_string()))
            });
        }
        if let Some(index) = install {
            let (client, version, dir) = (self.client.clone(), browser.versions[index].clone(), browser.mods_dir.clone());
            self.status = format!("Downloading {}...", version.file_name);
            self.spawn(ctx, async move {
                match modplatform::install(&client, &version, &dir).await {
                    Ok(path) => Msg::Status(format!("Installed {}.", path.file_name().unwrap_or_default().to_string_lossy())),
                    Err(e) => Msg::Status(format!("Could not install the mod: {e}")),
                }
            });
        }
        if let Some(name) = toggle {
            // Enabled and disabled mods differ by the ".disabled" suffix, as in the original.
            let from = browser.mods_dir.join(&name);
            let to = match name.strip_suffix(".disabled") {
                Some(enabled) => browser.mods_dir.join(enabled),
                None => browser.mods_dir.join(format!("{name}.disabled")),
            };
            if let Err(e) = std::fs::rename(&from, &to) {
                self.status = format!("Could not rename {name}: {e}");
            }
        }
        if let Some(name) = remove {
            self.status = match trash::delete(browser.mods_dir.join(&name)) {
                Ok(()) => format!("Moved {name} to the recycle bin."),
                Err(e) => format!("Could not remove {name}: {e}"),
            };
        }
        if open {
            self.windows.mods = Some(browser);
        }
    }

    fn log_window(&mut self, ctx: &egui::Context) {
        let Some(id) = self.windows.log.clone() else { return };
        let Some(running) = self.running.get_mut(&id) else {
            self.windows.log = None;
            return;
        };
        let mut open = true;
        egui::Window::new(format!("Log of {id}")).open(&mut open).default_size([820.0, 460.0]).show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Copy").clicked() {
                    ui.ctx().copy_text(running.lines.join("\n"));
                }
                if ui.button("Clear").clicked() {
                    running.lines.clear();
                }
                if ui.add_enabled(!running.finished, egui::Button::new("Kill")).clicked() {
                    running.kill();
                }
                ui.label(if running.finished { "finished" } else { "running" });
            });
            ui.separator();
            let height = ui.text_style_height(&egui::TextStyle::Monospace);
            egui::ScrollArea::both().auto_shrink([false, false]).stick_to_bottom(true).show_rows(ui, height, running.lines.len(), |ui, range| {
                for line in &running.lines[range] {
                    ui.add(egui::Label::new(egui::RichText::new(line).monospace()).extend());
                }
            });
        });
        if !open {
            self.windows.log = None;
        }
    }

    fn delete_window(&mut self, ctx: &egui::Context) {
        let Some(id) = self.windows.confirm_delete.clone() else { return };
        let Some(row) = self.row(&id) else {
            self.windows.confirm_delete = None;
            return;
        };
        let (name, root) = (row.instance.name(), row.instance.root.clone());
        let (mut delete, mut cancel) = (false, false);
        egui::Window::new("Delete instance").collapsible(false).resizable(false).show(ctx, |ui| {
            ui.label(format!("Move the instance \"{name}\" with all its worlds and mods to the recycle bin?"));
            ui.label(root.display().to_string());
            ui.horizontal(|ui| {
                delete = ui.button("Delete").clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
        if delete {
            // Only the recycle bin: a failed move never falls back to deleting for good.
            let result = trash::delete(&root);
            self.reload();
            self.status = match result {
                Ok(()) => format!("Moved {name} to the recycle bin."),
                Err(e) => format!("Could not delete {name}: {e}"),
            };
        }
        if delete || cancel {
            self.windows.confirm_delete = None;
        }
    }
}
