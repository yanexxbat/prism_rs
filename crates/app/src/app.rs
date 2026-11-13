//! Application state, background work and the main window.
//! A simplified counterpart of `Application`, `MainWindow` and `LaunchController`.

use crate::theme;
use crate::windows::Windows;
use crate::Cli;
use eframe::egui;
use prism_auth::{msa, Account, AccountList, AccountType, AuthSession};
use prism_core::instance::{self, Instance};
use prism_core::settings::Settings;
use prism_minecraft::java::JavaInstall;
use prism_minecraft::launch::{self, LaunchRequest, Target};
use prism_minecraft::loaders::Loader;
use prism_minecraft::meta::{Meta, VersionEntry};
use prism_minecraft::modplatform::{ModVersion, Project};
use prism_minecraft::pack_profile::PackProfile;
use prism_net::Client;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};

/// Results of background work, delivered to the UI thread.
pub enum Msg {
    Status(String),
    /// Something on disk changed: reload the instance list and show this text.
    Reload(String),
    MinecraftVersions(Vec<VersionEntry>),
    LoaderVersions(Loader, String, Vec<VersionEntry>),
    DeviceCode(msa::DeviceCode),
    LoggedIn(Result<Box<Account>, String>),
    /// A refreshed account for the list index, an error text, and the launch waiting for it.
    Refreshed { index: usize, account: Box<Account>, error: Option<String>, then: Option<PendingLaunch> },
    Javas(Vec<JavaInstall>),
    ModResults(Result<Vec<Project>, String>),
    ModVersions(Result<Vec<ModVersion>, String>),
}

#[derive(Clone)]
pub struct PendingLaunch {
    pub instance_id: String,
    pub target: Option<Target>,
}

/// One line of the instance list.
pub struct Row {
    pub instance: Instance,
    pub minecraft: String,
    pub loader: Option<Loader>,
    pub group: String,
}

/// A game that is starting or running, with its log.
pub struct Running {
    pub lines: Vec<String>,
    rx: tokio::sync::mpsc::UnboundedReceiver<String>,
    kill: Option<tokio::sync::oneshot::Sender<()>>,
    pub finished: bool,
}

impl Running {
    pub fn kill(&mut self) {
        if let Some(kill) = self.kill.take() {
            let _ = kill.send(());
        }
    }
}

pub struct App {
    pub rt: tokio::runtime::Handle,
    pub data_dir: PathBuf,
    pub client: Client,
    pub meta: Meta,
    pub global: Settings,
    pub accounts: AccountList,
    pub rows: Vec<Row>,
    pub selected: Option<String>,
    pub tx: Sender<Msg>,
    rx: Receiver<Msg>,
    pub status: String,
    pub running: BTreeMap<String, Running>,
    pub windows: Windows,
    cli: Option<Cli>,
    themed: bool,
}

impl App {
    pub fn new(rt: tokio::runtime::Handle, data_dir: PathBuf, cli: Cli) -> Result<Self, Box<dyn std::error::Error>> {
        let global = Settings::load(&data_dir.join("prismlauncher.cfg"))?;
        let accounts = AccountList::load(&data_dir.join("accounts.json"))?;
        let client = Client::new()?;
        let meta = Meta::new(client.clone(), &data_dir, global.get("MetaURLOverride").unwrap_or_default());
        let (tx, rx) = std::sync::mpsc::channel();
        let mut app = App {
            rt,
            data_dir,
            client,
            meta,
            global,
            accounts,
            rows: Vec::new(),
            selected: None,
            tx,
            rx,
            status: String::new(),
            running: BTreeMap::new(),
            windows: Windows::default(),
            cli: Some(cli).filter(|cli| cli.launch.is_some()),
            themed: false,
        };
        app.reload();
        Ok(app)
    }

    pub fn instances_dir(&self) -> PathBuf {
        self.data_dir.join(self.global.get("InstanceDir").unwrap_or("instances"))
    }

    pub fn msa_client_id(&self) -> Option<String> {
        prism_auth::msa_client_id(self.global.get("MSAClientIDOverride").unwrap_or_default())
    }

    pub fn curseforge_key(&self) -> Option<String> {
        prism_minecraft::modplatform::curseforge_api_key(self.global.get("FlameKeyOverride").unwrap_or_default())
    }

    pub fn reload(&mut self) {
        let dir = self.instances_dir();
        let groups = instance::load_groups(&dir);
        let instances = instance::load_instances(&dir).unwrap_or_else(|e| {
            self.status = e.to_string();
            Vec::new()
        });
        self.rows = instances
            .into_iter()
            .map(|instance| {
                let pack = PackProfile::load(&instance.root).ok();
                Row {
                    minecraft: pack.as_ref().and_then(|p| p.component("net.minecraft")).map(|c| c.version.clone()).unwrap_or_default(),
                    loader: pack.as_ref().and_then(Loader::of),
                    group: groups.get(&instance.id).cloned().unwrap_or_default(),
                    instance,
                }
            })
            .collect();
        if !self.rows.iter().any(|row| Some(&row.instance.id) == self.selected.as_ref()) {
            self.selected = None;
        }
    }

    pub fn row(&self, id: &str) -> Option<&Row> {
        self.rows.iter().find(|row| row.instance.id == id)
    }

    /// Runs `work` in the background and hands its message to the UI.
    pub fn spawn(&self, ctx: &egui::Context, work: impl std::future::Future<Output = Msg> + Send + 'static) -> tokio::task::JoinHandle<()> {
        let (tx, ctx) = (self.tx.clone(), ctx.clone());
        self.rt.spawn(async move {
            let _ = tx.send(work.await);
            ctx.request_repaint();
        })
    }

    pub fn save_accounts(&mut self) {
        if let Err(e) = self.accounts.save() {
            self.status = format!("Could not save accounts: {e}");
        }
    }

    /// Ported from `LaunchController`: pick the account, refresh it when due, then start.
    /// `offline_name` asks for an offline launch with that player name.
    pub fn launch(&mut self, ctx: &egui::Context, instance_id: &str, profile: Option<&str>, offline_name: Option<String>, target: Option<Target>) {
        if self.running.get(instance_id).is_some_and(|r| !r.finished) {
            self.status = "This instance is already running.".into();
            return;
        }
        let index = match profile {
            Some(name) => self.accounts.accounts.iter().position(|a| a.profile_name() == name),
            None => self.accounts.accounts.iter().position(|a| a.active),
        };
        let Some(index) = index else {
            self.status = "No account to launch with. Add an account and make it the default.".into();
            self.windows.accounts_open = true;
            return;
        };
        let account = self.accounts.accounts[index].clone();
        let pending = PendingLaunch { instance_id: instance_id.to_string(), target };

        if let Some(name) = offline_name {
            if account.kind == AccountType::Msa && !account.owns_minecraft() {
                self.status = "This account is not known to own Minecraft; log in online first.".into();
                return;
            }
            self.start_game(ctx, pending, account.session(Some(&name)));
        } else if account.should_refresh() {
            let Some(client_id) = self.msa_client_id() else {
                self.status = "Microsoft login needs a client ID (see README). You can still use Launch Offline.".into();
                return;
            };
            self.status = format!("Refreshing the login of {}...", account.profile_name());
            let client = self.client.clone();
            self.spawn(ctx, async move {
                let mut account = Box::new(account);
                let result = msa::refresh(&client, &msa::Endpoints::default(), &client_id, &mut account).await;
                Msg::Refreshed { index, account, error: result.err().map(|e| e.to_string()), then: Some(pending) }
            });
        } else {
            self.launch_with(ctx, pending, &account);
        }
    }

    /// The online launch once the account is known to be current.
    fn launch_with(&mut self, ctx: &egui::Context, pending: PendingLaunch, account: &Account) {
        if account.kind == AccountType::Msa && !account.owns_minecraft() {
            // The original offers the demo here; demo mode is not ported.
            self.status = format!("The account {} does not own Minecraft.", account.profile_name());
            return;
        }
        self.start_game(ctx, pending, account.session(None));
    }

    fn start_game(&mut self, ctx: &egui::Context, pending: PendingLaunch, session: AuthSession) {
        let Some(row) = self.row(&pending.instance_id) else {
            self.status = format!("There is no instance with the id '{}'.", pending.instance_id);
            return;
        };
        let mut instance = row.instance.clone();
        let (log, rx) = tokio::sync::mpsc::unbounded_channel();
        let (kill, killed) = tokio::sync::oneshot::channel();
        let (client, meta, global, data_dir) = (self.client.clone(), self.meta.clone(), self.global.clone(), self.data_dir.clone());
        let screen_size = ctx.input(|i| i.viewport().monitor_size.map(|size| {
            let pixels = size * i.pixels_per_point();
            (pixels.x as u32, pixels.y as u32)
        }));
        let repaint = ctx.clone();
        self.rt.spawn(async move {
            let request = LaunchRequest {
                data_dir: &data_dir,
                global: &global,
                instance: &mut instance,
                session: &session,
                screen_size,
                target: pending.target,
            };
            let result = match launch::prepare(&client, &meta, request, &log).await {
                Ok(prepared) => launch::run(&prepared, &log, killed).await.map(|_| ()),
                Err(e) => Err(e),
            };
            if let Err(e) = result {
                let _ = log.send(format!("\n{e}"));
            }
            // Dropping `log` tells the UI that this launch is over.
            repaint.request_repaint();
        });
        self.status = format!("Launching {}...", pending.instance_id);
        self.windows.log = Some(pending.instance_id.clone());
        self.running.insert(pending.instance_id, Running { lines: Vec::new(), rx, kill: Some(kill), finished: false });
    }

    fn pump_logs(&mut self, ctx: &egui::Context) {
        const MAX_LINES: usize = 100_000; // the original's ConsoleMaxLines default
        let mut just_finished = false;
        for running in self.running.values_mut().filter(|r| !r.finished) {
            loop {
                match running.rx.try_recv() {
                    // An empty message is a blank line; `lines()` alone would drop it.
                    Ok(text) if text.is_empty() => running.lines.push(text),
                    Ok(text) => running.lines.extend(text.lines().map(str::to_string)),
                    Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                    Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                        running.finished = true;
                        running.kill = None;
                        just_finished = true;
                        break;
                    }
                }
            }
            if running.lines.len() > MAX_LINES {
                let excess = running.lines.len() - MAX_LINES;
                running.lines.drain(..excess);
            }
        }
        if just_finished {
            // A launch may have stored a Java path in instance.cfg.
            self.reload();
            self.status = "The game has exited.".into();
        }
        if self.running.values().any(|r| !r.finished) {
            ctx.request_repaint_after(std::time::Duration::from_millis(150));
        }
    }

    fn handle(&mut self, msg: Msg, ctx: &egui::Context) {
        match msg {
            Msg::Status(text) => self.status = text,
            Msg::Reload(text) => {
                self.reload();
                self.status = text;
            }
            Msg::Refreshed { index, account, error, then } => {
                let same = self.accounts.accounts.get(index).is_some_and(|a| a.kind == account.kind && a.msa_client_id == account.msa_client_id);
                if same {
                    let active = self.accounts.accounts[index].active;
                    self.accounts.accounts[index] = Account { active, ..*account.clone() };
                    self.save_accounts();
                }
                match (error, then) {
                    (Some(error), _) => self.status = format!("Login failed: {error} You can use Launch Offline."),
                    (None, Some(pending)) => self.launch_with(ctx, pending, &account),
                    (None, None) => self.status = format!("{} is logged in.", account.profile_name()),
                }
            }
            other => self.windows.handle(other, &mut self.accounts, &mut self.status),
        }
    }

    fn top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("toolbar").frame(theme::glass()).show_separator_line(false).show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Add Instance").clicked() {
                    self.open_new_instance(ctx);
                }
                if ui.button("Accounts").clicked() {
                    self.windows.accounts_open = true;
                }
                if ui.button("Settings").clicked() {
                    self.open_settings();
                }
                if ui.button("Reload").clicked() {
                    self.reload();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let account = self.accounts.default_account().map(Account::profile_name).unwrap_or("No default account");
                    ui.label(account);
                });
            });
        });
        egui::TopBottomPanel::bottom("status").frame(theme::glass().inner_margin(egui::Margin::symmetric(12, 6))).show_separator_line(false).show(ctx, |ui| {
            ui.label(if self.status.is_empty() { self.data_dir.display().to_string() } else { self.status.clone() });
        });
    }

    fn actions(&mut self, ctx: &egui::Context) {
        // The glass frame is drawn inside the panel: a side panel clips a frame's outer margin.
        let panel = egui::SidePanel::right("actions").exact_width(214.0).frame(egui::Frame::NONE).show_separator_line(false);
        panel.show(ctx, |ui| theme::glass().show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            let Some(id) = self.selected.clone() else {
                ui.label("Select an instance.");
                return;
            };
            let Some(row) = self.row(&id) else { return };
            let (name, root) = (row.instance.name(), row.instance.root.clone());
            let loader = row.loader.map(|l| format!(" + {}", l.name())).unwrap_or_default();
            ui.heading(&name);
            ui.label(format!("Minecraft {}{loader}", row.minecraft));
            ui.separator();
            let is_running = self.running.get(&id).is_some_and(|r| !r.finished);
            let wide = |ui: &mut egui::Ui, enabled: bool, text: &str| {
                ui.add_enabled(enabled, egui::Button::new(text).min_size(egui::vec2(170.0, 0.0))).clicked()
            };
            let launch = egui::Button::new(egui::RichText::new("Launch").strong().color(theme::ON_ACCENT)).fill(theme::ACCENT).min_size(egui::vec2(170.0, 34.0));
            if ui.add_enabled(!is_running, launch).clicked() {
                self.launch(ctx, &id, None, None, None);
            }
            if wide(ui, !is_running, "Launch Offline") {
                let name = self.accounts.default_account().map(|a| a.profile_name().to_string());
                self.launch(ctx, &id, None, name.or(Some("Player".into())), None);
            }
            if wide(ui, is_running, "Kill") {
                if let Some(running) = self.running.get_mut(&id) {
                    running.kill();
                }
            }
            if wide(ui, self.running.contains_key(&id), "Log") {
                self.windows.log = Some(id.clone());
            }
            ui.separator();
            if wide(ui, true, "Edit") {
                self.open_edit(&id);
            }
            if wide(ui, true, "Mods") {
                self.open_mods(&id);
            }
            if wide(ui, true, "Folder") {
                open_folder(&root, &mut self.status);
            }
            if wide(ui, !is_running, "Delete") {
                self.windows.confirm_delete = Some(id.clone());
            }
        }));
    }

    fn instance_list(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().frame(theme::glass()).show(ctx, |ui| {
            if self.rows.is_empty() {
                ui.label("No instances yet. Use \"Add Instance\" to create one.");
                return;
            }
            let mut groups: BTreeMap<&str, Vec<&Row>> = BTreeMap::new();
            for row in &self.rows {
                groups.entry(row.group.as_str()).or_default().push(row);
            }
            let mut clicked = None;
            let mut double_clicked = None;
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                for (group, rows) in groups {
                    let title = if group.is_empty() { "Ungrouped" } else { group };
                    egui::CollapsingHeader::new(title).default_open(true).show(ui, |ui| {
                        for row in rows {
                            let id = &row.instance.id;
                            let loader = row.loader.map(|l| format!(", {}", l.name())).unwrap_or_default();
                            let state = if self.running.get(id).is_some_and(|r| !r.finished) { "  (running)" } else { "" };
                            let text = format!("{}   [{}{loader}]{state}", row.instance.name(), row.minecraft);
                            let response = ui.selectable_label(self.selected.as_ref() == Some(id), text);
                            if response.clicked() {
                                clicked = Some(id.clone());
                            }
                            if response.double_clicked() {
                                double_clicked = Some(id.clone());
                            }
                        }
                    });
                }
            });
            if let Some(id) = clicked {
                self.selected = Some(id);
            }
            if let Some(id) = double_clicked {
                self.launch(ctx, &id, None, None, None);
            }
        });
    }
}

/// Opens a folder in the system's file manager.
pub fn open_folder(path: &std::path::Path, status: &mut String) {
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    if let Err(e) = std::process::Command::new(program).arg(path).spawn() {
        *status = format!("Could not open {}: {e}", path.display());
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.ui(ctx);
    }

    /// Nothing behind the UI, so the desktop shows through the translucent background.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }
}

impl App {
    /// One frame. Separate from `eframe::App::update` so tests can drive it without a window.
    pub fn ui(&mut self, ctx: &egui::Context) {
        if !std::mem::replace(&mut self.themed, true) {
            theme::apply(ctx);
        }
        theme::paint_background(ctx);
        while let Ok(msg) = self.rx.try_recv() {
            self.handle(msg, ctx);
        }
        self.pump_logs(ctx);
        if let Some(cli) = self.cli.take() {
            // `--launch`: start right away, as the original does.
            let target = cli.server.as_deref().map(Target::server).or(cli.world.as_deref().map(Target::world));
            if let Some(id) = cli.launch {
                self.selected = Some(id.clone());
                self.launch(ctx, &id, cli.profile.as_deref(), cli.offline, target);
            }
        }
        self.top_bar(ctx);
        self.actions(ctx);
        self.instance_list(ctx);
        self.show_windows(ctx);
    }
}
