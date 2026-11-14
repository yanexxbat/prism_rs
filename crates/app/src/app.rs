//! Application state, background work and the main window.
//! A simplified counterpart of `Application`, `MainWindow` and `LaunchController`.

use crate::glass::{self, Glass};
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
    pub glass: Glass,
    /// The glass panel this frame; dialogs stay inside it.
    pub panel: egui::Rect,
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
            glass: Glass::default(),
            panel: egui::Rect::ZERO,
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

    /// Title, subtitle and the main actions on the left; the navigation on the right.
    fn header(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.label(theme::title("Instances"));
                let account = self.accounts.default_account().map(Account::profile_name).unwrap_or("No default account");
                let count = match self.rows.len() {
                    1 => "1 instance".to_string(),
                    n => format!("{n} instances"),
                };
                ui.label(theme::subtitle(format!("{count}  ·  {account}")));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                ui.spacing_mut().item_spacing.x = 22.0;
                let page = self.windows.page();
                if theme::nav(ui, "Settings", page == Some("Settings"), true).clicked() {
                    self.open_settings();
                }
                let selected = self.selected.clone();
                if theme::nav(ui, "Mods", page == Some("Mods"), selected.is_some()).clicked() {
                    if let Some(id) = selected {
                        self.open_mods(&id);
                    }
                }
                if theme::nav(ui, "Accounts", page == Some("Accounts"), true).clicked() {
                    self.windows.accounts_open = true;
                }
                if theme::nav(ui, "Instances", page.is_none(), true).clicked() {
                    self.windows.close_pages();
                }
            });
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui.button("Add Instance").clicked() {
                self.open_new_instance(ctx);
            }
            if ui.button("Reload").clicked() {
                self.reload();
            }
        });
        ui.add_space(8.0);
    }

    /// The selected instance: what it is, its actions, and the icon row at the bottom right.
    fn details(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.set_min_size(ui.available_size());
        let Some(id) = self.selected.clone() else {
            ui.label(theme::caption("Select an instance."));
            return;
        };
        let Some(row) = self.row(&id) else { return };
        let (name, root, minecraft, loader) = (row.instance.name(), row.instance.root.clone(), row.minecraft.clone(), row.loader);
        let is_running = self.running.get(&id).is_some_and(|r| !r.finished);
        // The icon row first, as a bottom panel, so a short tile can never stack it on the buttons.
        let icons = egui::TopBottomPanel::bottom("detail icons").frame(egui::Frame::NONE).show_separator_line(false);
        icons.show_inside(ui, |ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if theme::icon_button(ui, "▶", "Launch", true, !is_running).clicked() {
                    self.launch(ctx, &id, None, None, None);
                }
                if theme::icon_button(ui, "🗁", "Folder", false, true).clicked() {
                    open_folder(&root, &mut self.status);
                }
                if theme::icon_button(ui, "🖹", "Log", false, self.running.contains_key(&id)).clicked() {
                    self.windows.log = Some(id.clone());
                }
                if theme::icon_button(ui, "⚙", "Edit", false, true).clicked() {
                    self.open_edit(&id);
                }
            });
        });
        egui::CentralPanel::default().frame(egui::Frame::NONE).show_inside(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.detail_actions(ui, ctx, &id, &name, &minecraft, loader, is_running));
        });
    }

    /// Name, version, badges and the wide action buttons of the selected instance.
    #[allow(clippy::too_many_arguments)]
    fn detail_actions(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, id: &str, name: &str, minecraft: &str, loader: Option<Loader>, is_running: bool) {
        ui.add(egui::Label::new(egui::RichText::new(name).font(theme::semibold(19.0)).color(theme::TEXT_TITLE)).wrap());
        ui.label(theme::subtitle(format!("Minecraft {minecraft}")));
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            theme::badge(ui, minecraft);
            if let Some(loader) = loader {
                theme::badge(ui, loader.name());
            }
            if is_running {
                theme::badge(ui, "Running");
            }
        });
        ui.add_space(10.0);
        let wide = |ui: &mut egui::Ui, enabled: bool, text: &str| {
            ui.add_enabled(enabled, egui::Button::new(text).min_size(egui::vec2(ui.available_width(), 30.0))).clicked()
        };
        if wide(ui, !is_running, "Launch Offline") {
            let name = self.accounts.default_account().map(|a| a.profile_name().to_string());
            self.launch(ctx, id, None, name.or(Some("Player".into())), None);
        }
        if wide(ui, is_running, "Kill") {
            if let Some(running) = self.running.get_mut(id) {
                running.kill();
            }
        }
        if wide(ui, !is_running, "Delete") {
            self.windows.confirm_delete = Some(id.to_string());
        }
    }

    /// The instances as glass tiles, by group.
    fn instance_list(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if self.rows.is_empty() {
            ui.label(theme::caption("No instances yet. Use \"Add Instance\" to create one."));
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
                let header = egui::RichText::new(title.to_uppercase()).size(11.0).color(theme::TEXT_DIM);
                egui::CollapsingHeader::new(header).id_salt(title).default_open(true).show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(12.0, 12.0);
                        for row in rows {
                            let id = &row.instance.id;
                            let running = self.running.get(id).is_some_and(|r| !r.finished);
                            let response = instance_card(ui, row, self.selected.as_ref() == Some(id), running);
                            if response.clicked() {
                                clicked = Some(id.clone());
                            }
                            if response.double_clicked() {
                                double_clicked = Some(id.clone());
                            }
                        }
                    });
                });
            }
        });
        if let Some(id) = clicked {
            self.selected = Some(id);
        }
        if let Some(id) = double_clicked {
            self.launch(ctx, &id, None, None, None);
        }
    }
}

/// One instance as a small glass tile: name, version badges, a glow when selected.
fn instance_card(ui: &mut egui::Ui, row: &Row, selected: bool, running: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(196.0, 84.0), egui::Sense::click());
    let loader = row.loader.map(|l| format!(", {}", l.name())).unwrap_or_default();
    let state = if running { "  (running)" } else { "" };
    // What screen readers (and the tests) read: the same text the plain list used to show.
    let label = format!("{}   [{}{loader}]{state}", row.instance.name(), row.minecraft);
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, &label));
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let hot = response.hovered();
    let painter = ui.painter();
    let radius = egui::CornerRadius::same(16);
    if selected {
        painter.add(egui::Shadow { offset: [0, 0], blur: 20, spread: 0, color: theme::white(34) }.as_shape(rect, radius));
    }
    let (fill, edge) = match (selected, hot) {
        (true, _) => (16, 150),
        (false, true) => (11, 80),
        _ => (5, 34),
    };
    painter.rect(rect, radius, theme::white(fill), egui::Stroke::new(1.0_f32, theme::white(edge)), egui::StrokeKind::Inside);
    let mut job = egui::text::LayoutJob::simple_singleline(row.instance.name(), theme::semibold(14.0), theme::TEXT);
    job.wrap = egui::text::TextWrapping::truncate_at_width(rect.width() - 28.0);
    let name = ui.fonts(|fonts| fonts.layout_job(job));
    painter.galley(rect.min + egui::vec2(14.0, 14.0), name, theme::TEXT);
    let mut x = rect.left() + 14.0;
    let y = rect.bottom() - 26.0;
    let badges = [Some(row.minecraft.as_str()), row.loader.map(|l| l.name()), running.then_some("Running")];
    for text in badges.into_iter().flatten().filter(|t| !t.is_empty()) {
        x += theme::paint_badge(ui, egui::pos2(x, y), text) + 6.0;
    }
    response
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
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.glass.window(ctx, frame, self.acrylic());
        self.ui(ctx);
    }

    /// Nothing behind the panel, so the corners and the margin around it are see-through.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }
}

impl App {
    /// One frame. Separate from `eframe::App::update` so tests can drive it without a window.
    pub fn ui(&mut self, ctx: &egui::Context) {
        if !std::mem::replace(&mut self.themed, true) {
            // New fonts are bound from the next pass on; egui runs that pass right away.
            theme::apply(ctx);
            ctx.request_discard("fonts");
            return;
        }
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
        let panel = glass::panel_rect(ctx.screen_rect(), glass::maximized(ctx));
        self.panel = panel;
        self.glass.paint(ctx, panel, self.acrylic());
        // From here on egui treats the panel as the screen, so dialogs, menus and tooltips stay
        // on the glass: outside it the window is clipped away on Windows.
        // The real size comes back at the end of the pass for painting.
        let screen = ctx.input_mut(|i| std::mem::replace(&mut i.screen_rect, panel));
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
            glass::title_bar(ui, panel, self.windows.page().unwrap_or("Instances"));
            ui.scope_builder(egui::UiBuilder::new().max_rect(glass::content_rect(panel)), |ui| {
                self.header(ui, ctx);
                egui::TopBottomPanel::bottom("status").frame(egui::Frame::NONE.inner_margin(egui::Margin { top: 8, ..Default::default() })).show_separator_line(false).show_inside(ui, |ui| {
                    ui.label(theme::caption(if self.status.is_empty() { self.data_dir.display().to_string() } else { self.status.clone() }));
                });
                let side = egui::SidePanel::right("actions").exact_width(238.0).resizable(false).show_separator_line(false);
                side.frame(egui::Frame::NONE.inner_margin(egui::Margin { left: 14, ..Default::default() })).show_inside(ui, |ui| {
                    theme::tile().show(ui, |ui| self.details(ui, ctx));
                });
                egui::CentralPanel::default().frame(egui::Frame::NONE).show_inside(ui, |ui| self.instance_list(ui, ctx));
            });
        });
        self.show_windows(ctx);
        glass::resize_edges(ctx, panel);
        ctx.input_mut(|i| i.screen_rect = screen);
    }

    /// The Windows acrylic blur instead of the built-in glass (Settings, Appearance).
    pub fn acrylic(&self) -> bool {
        cfg!(windows) && self.global.get("GlassAcrylic").is_some_and(prism_core::settings::parse_bool)
    }
}
