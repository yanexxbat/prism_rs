//! Launching an instance. Ported from `MinecraftInstance` (arguments, environment,
//! description), `minecraft/launch/*`, `minecraft/update/*`, `launch/steps/CheckJava`
//! and the argument handling of the Java-side `StandardLauncher`.

use crate::java::{self, JavaInstall};
use crate::meta::Meta;
use crate::pack_profile::PackProfile;
use crate::version_file::{LaunchProfile, RuntimeContext};
use crate::{assets, io_err, Error, Result};
use prism_auth::AuthSession;
use prism_core::fs::qt_path;
use prism_core::instance::Instance;
use prism_core::settings::{Effective, Settings};
use prism_net::{Client, Download};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use tokio::io::AsyncBufReadExt;

/// Lines for the launch log.
pub type Log = tokio::sync::mpsc::UnboundedSender<String>;

fn say(log: &Log, line: impl Into<String>) {
    // A closed log window is no reason to stop a launch.
    let _ = log.send(line.into());
}

/// A server or a world to join right after start. Ported from `MinecraftTarget`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Target {
    pub address: String,
    pub port: u16,
    pub world: String,
}

impl Target {
    pub fn world(name: &str) -> Self {
        Target { world: name.to_string(), ..Target::default() }
    }

    /// `host`, `host:port`, `[ipv6]` or `[ipv6]:port`; the port defaults to 25565.
    pub fn server(full_address: &str) -> Self {
        let full = full_address.trim();
        let (host, port) = match full.strip_prefix('[').and_then(|rest| rest.split_once(']')) {
            Some((ipv6, rest)) => (ipv6, rest.trim().strip_prefix(':')),
            None => match full.split_once(':') {
                Some((host, port)) if !port.contains(':') => (host, Some(port)),
                _ => (full, None),
            },
        };
        let port = port.and_then(|p| p.trim().parse().ok()).unwrap_or(25565);
        Target { address: host.to_string(), port, world: String::new() }
    }
}

pub struct LaunchRequest<'a> {
    pub data_dir: &'a Path,
    pub global: &'a Settings,
    /// Mutable because automatic Java selection is stored in `instance.cfg`, as in the original.
    pub instance: &'a mut Instance,
    pub session: &'a AuthSession,
    /// Used for the window size when the instance launches maximized.
    pub screen_size: Option<(u32, u32)>,
    pub target: Option<Target>,
}

/// A launch that is ready to start.
#[derive(Debug)]
pub struct Prepared {
    pub java: PathBuf,
    pub args: Vec<String>,
    pub working_dir: PathBuf,
    pub env: Vec<(OsString, OsString)>,
    pub censor: Vec<(String, String)>,
    pub natives_dir: PathBuf,
}

pub fn censor(text: &str, filter: &[(String, String)]) -> String {
    filter.iter().fold(text.to_string(), |text, (secret, replacement)| text.replace(secret, replacement))
}

impl Prepared {
    /// The command line with tokens and ids replaced, safe to log.
    pub fn censored_command_line(&self) -> String {
        censor(&format!("{} {}", qt_path(&self.java), self.args.join(" ")), &self.censor)
    }
}

/// Ported from `Commandline::splitArgs`.
pub fn split_args(args: &str) -> Vec<String> {
    let chars: Vec<char> = args.chars().collect();
    let (mut argv, mut current) = (Vec::new(), String::new());
    let (mut escape, mut in_quotes) = (false, None);
    for (i, &c) in chars.iter().enumerate() {
        if escape {
            current.push(c);
            escape = false;
        } else if let Some(quote) = in_quotes {
            // A backslash only escapes the quote or another backslash, so Windows paths survive.
            if c == '\\' && chars.get(i + 1).is_some_and(|next| *next == quote || *next == '\\') {
                escape = true;
            } else if c == quote {
                in_quotes = None;
            } else {
                current.push(c);
            }
        } else if c.is_whitespace() {
            if !current.is_empty() {
                argv.push(std::mem::take(&mut current));
            }
        } else if c == '"' || c == '\'' {
            in_quotes = Some(c);
        } else {
            current.push(c);
        }
    }
    if !current.is_empty() {
        argv.push(current);
    }
    argv
}

/// Replaces `${key}`; unknown keys become empty, as in the original.
fn replace_tokens(text: &str, vars: &BTreeMap<&str, String>) -> String {
    let (mut out, mut rest) = (String::new(), text);
    while let Some(start) = rest.find("${") {
        let Some(len) = rest[start..].find('}') else { break };
        out += &rest[..start];
        out += vars.get(&rest[start + 2..start + len]).map(String::as_str).unwrap_or_default();
        rest = &rest[start + len + 1..];
    }
    out + rest
}

/// Ported from `makeProfileVarMapping`, plus the session part of `processMinecraftArgs`.
fn variables(
    profile: &LaunchProfile,
    instance: &Instance,
    data_dir: &Path,
    session: Option<&AuthSession>,
) -> BTreeMap<&'static str, String> {
    let assets = profile.assets();
    let mut vars = BTreeMap::from([
        ("profile_name", instance.name()),
        ("version_name", profile.minecraft_version.clone()),
        ("version_type", profile.minecraft_version_type.clone()),
        ("game_directory", qt_path(&instance.game_root())),
        ("game_assets", qt_path(&assets::assets_dir(data_dir, &assets.id, &instance.resources_dir()))),
        ("assets_root", qt_path(&data_dir.join("assets"))),
        ("assets_index_name", assets.id),
        ("library_directory", qt_path(&data_dir.join("libraries"))),
    ]);
    if let Some(session) = session {
        vars.insert("auth_session", session.session.clone());
        vars.insert("auth_access_token", session.access_token.clone());
        vars.insert("auth_player_name", session.player_name.clone());
        vars.insert("auth_uuid", session.uuid.clone());
        vars.insert("user_properties", "{}".to_string());
        vars.insert("user_type", session.user_type.clone());
    }
    vars
}

/// Ported from `MinecraftInstance::javaArguments` and `extraArguments`.
fn jvm_arguments(
    profile: &LaunchProfile,
    settings: Effective,
    java: &JavaInstall,
    vars: &BTreeMap<&str, String>,
    agent_jars: &[PathBuf],
) -> Vec<String> {
    let mut args = vec!["-Duser.language=en".to_string()];
    // Custom arguments go first so the launcher's own ones override them.
    args.extend(split_args(&settings.get("JvmArgs")));
    args.extend(profile.jvm_args.iter().map(|arg| replace_tokens(arg, vars)));
    for (agent, jar) in profile.agents.iter().zip(agent_jars) {
        let argument = agent.argument.as_deref().filter(|a| !a.is_empty()).map(|a| format!("={a}")).unwrap_or_default();
        args.push(format!("-javaagent:{}{argument}", qt_path(jar)));
    }
    if cfg!(windows) {
        // Makes Intel drivers treat the process as the official launcher's. See MCL-767.
        args.push("-XX:HeapDumpPath=MojangTricksIntelDriversForPerformance_javaw.exe_minecraft.exe.heapdump".into());
    }
    let (min, max) = (settings.get_int("MinMemAlloc"), settings.get_int("MaxMemAlloc"));
    args.push(format!("-Xms{}m", min.min(max)));
    args.push(format!("-Xmx{}m", min.max(max)));
    if java.requires_perm_gen() && settings.get_int("PermGen") != 64 {
        args.push(format!("-XX:PermSize={}m", settings.get_int("PermGen")));
    }
    args
}

/// Ported from `processMinecraftArgs`, followed by what `StandardLauncher.launch` appends.
fn game_arguments(
    profile: &LaunchProfile,
    vars: &BTreeMap<&str, String>,
    window: (i64, i64),
    target: Option<&Target>,
) -> Vec<String> {
    let mut args: Vec<String> = profile.minecraft_arguments.split(' ').filter(|a| !a.is_empty()).map(String::from).collect();
    for tweaker in &profile.tweakers {
        args.extend(["--tweakClass".to_string(), tweaker.clone()]);
    }
    let mut args: Vec<String> = args.iter().map(|arg| replace_tokens(arg, vars)).collect();
    args.extend(["--width".to_string(), window.0.to_string(), "--height".to_string(), window.1.to_string()]);
    match target {
        Some(t) if !t.address.is_empty() && profile.traits.contains("feature:is_quick_play_multiplayer") => {
            args.extend(["--quickPlayMultiplayer".to_string(), format!("{}:{}", t.address, t.port)]);
        }
        Some(t) if !t.address.is_empty() => {
            args.extend(["--server".to_string(), t.address.clone(), "--port".to_string(), t.port.to_string()]);
        }
        Some(t) if !t.world.is_empty() && profile.traits.contains("feature:is_quick_play_singleplayer") => {
            args.extend(["--quickPlaySingleplayer".to_string(), t.world.clone()]);
        }
        _ => {}
    }
    args
}

fn window_size(settings: Effective, screen_size: Option<(u32, u32)>) -> (i64, i64) {
    match screen_size {
        Some((w, h)) if settings.get_bool("LaunchMaximized") => (w.into(), h.into()),
        _ => (settings.get_int("MinecraftWinWidth"), settings.get_int("MinecraftWinHeight")),
    }
}

/// Ported from `CleanEnviroment`, `getVariables` and `createEnvironment`.
fn environment(instance: &Instance, settings: Effective, java: &Path, jvm_args: &[String]) -> Vec<(OsString, OsString)> {
    const IGNORED: [&str; 8] =
        ["JAVA_ARGS", "CLASSPATH", "CONFIGPATH", "JAVA_HOME", "JRE_HOME", "_JAVA_OPTIONS", "JAVA_OPTIONS", "JAVA_TOOL_OPTIONS"];
    let native = |path: &Path| std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf()).into_os_string();
    let mut env: Vec<(OsString, OsString)> = std::env::vars_os()
        .filter(|(key, _)| {
            let key = key.to_string_lossy();
            !IGNORED.iter().any(|i| key.eq_ignore_ascii_case(i)) && !key.starts_with("LAUNCHER_")
        })
        .collect();
    env.extend([
        ("INST_NAME".into(), instance.name().into()),
        ("INST_ID".into(), instance.id.clone().into()),
        ("INST_DIR".into(), native(&instance.root)),
        ("INST_MC_DIR".into(), native(&instance.game_root())),
        ("INST_JAVA".into(), native(java)),
        ("INST_JAVA_ARGS".into(), jvm_args.join(" ").into()),
        ("NO_COLOR".into(), "1".into()),
    ]);
    let custom: BTreeMap<String, serde_json::Value> = serde_json::from_str(&settings.get("Env")).unwrap_or_default();
    for (key, value) in custom {
        let value = value.as_str().map(str::to_string).unwrap_or_else(|| value.to_string());
        env.push((key.into(), value.into()));
    }
    env
}

/// Ported from `ExtractNatives`: unpack every native jar into the instance's `natives` folder.
fn extract_natives(jars: &[PathBuf], target: &Path, jnilib_hack: bool) -> Result<()> {
    std::fs::create_dir_all(target).map_err(io_err(target))?;
    for jar in jars {
        let failed = |reason: String| {
            Error::Launch(format!("Couldn't extract native jar '{}' to destination '{}': {reason}", jar.display(), target.display()))
        };
        let file = std::fs::File::open(jar).map_err(|e| failed(e.to_string()))?;
        let mut zip = zip::ZipArchive::new(file).map_err(|e| failed(e.to_string()))?;
        for index in 0..zip.len() {
            let mut entry = zip.by_index(index).map_err(|e| failed(e.to_string()))?;
            // enclosed_name refuses entries that would land outside the target folder.
            let relative = entry.enclosed_name().ok_or_else(|| failed(format!("unsafe path '{}'", entry.name())))?;
            let mut out = target.join(relative);
            if jnilib_hack && out.extension().is_some_and(|e| e == "jnilib") {
                out.set_extension("dylib");
            }
            if entry.is_dir() {
                std::fs::create_dir_all(&out).map_err(io_err(&out))?;
            } else {
                if let Some(parent) = out.parent() {
                    std::fs::create_dir_all(parent).map_err(io_err(parent))?;
                }
                let mut file = std::fs::File::create(&out).map_err(io_err(&out))?;
                std::io::copy(&mut entry, &mut file).map_err(io_err(&out))?;
            }
        }
    }
    Ok(())
}

/// Ported from `AutoInstallJava` (without the download), `CheckJava` and `VerifyJavaInstall`.
async fn select_java(request: &mut LaunchRequest<'_>, compatible: &[u32], log: &Log) -> Result<JavaInstall> {
    let global = request.global;
    let java_dir = request.data_dir.join(global.get("JavaDir").unwrap_or("java"));
    let settings = Effective { global, instance: Some(&request.instance.settings) };
    let pinned = settings.get_bool("OverrideJavaLocation") && Path::new(&settings.get("JavaPath")).is_file();
    let mut detected = None;

    if settings.get_bool("AutomaticJavaSwitch") && !pinned {
        let installs = java::detect(&java_dir).await;
        match installs.iter().find(|j| compatible.contains(&j.major)) {
            Some(found) => {
                if !found.is_64bit {
                    say(log, "The automatic Java mechanism detected a 32-bit installation of Java.");
                }
                let instance = &mut request.instance.settings;
                instance.set("OverrideJavaLocation", true);
                instance.set("JavaPath", qt_path(&found.path));
                instance.set("AutomaticJava", true);
                instance.save()?;
                say(log, format!("Compatible Java found at: {}.", qt_path(&found.path)));
            }
            None => say(log, "No compatible Java version was found. Using the default one."),
        }
        detected = Some(installs);
    }

    let settings = Effective { global, instance: Some(&request.instance.settings) };
    let mut path = settings.get("JavaPath");
    if path.is_empty() {
        // Port decision: with no Java configured, take the newest one found.
        let installs = match detected {
            Some(installs) => installs,
            None => java::detect(&java_dir).await,
        };
        path = installs.first().map(|j| qt_path(&j.path)).unwrap_or_default();
    }
    if !Path::new(&path).is_file() {
        return Err(Error::Java(format!(
            "The Java binary \"{path}\" couldn't be found. Please fix the Java path override in the instance's settings or disable it."
        )));
    }
    let install = java::check(Path::new(&path)).await?;
    say(
        log,
        format!(
            "Java is version {}, using {} ({}) architecture, from {}.\n",
            install.version,
            install.architecture(),
            install.real_architecture,
            install.vendor
        ),
    );

    if compatible.is_empty() || compatible.contains(&install.major) {
        return Ok(install);
    }
    if settings.get_bool("IgnoreJavaCompatibility") {
        say(log, "Java major version is incompatible. Things might break.\n");
        return Ok(install);
    }
    let wanted: Vec<String> = compatible.iter().map(|major| format!("Java version {major}")).collect();
    Err(Error::Java(format!(
        "This instance is not compatible with Java version {}.\nPlease switch to one of the following Java versions for this instance:\n{}\n\
         Go to instance Java settings to change your Java version or disable the Java compatibility check if you know what you're doing.",
        install.major,
        wanted.join("\n")
    )))
}

/// `[✔] name` lines for the files in a mods folder.
fn mod_lines(dir: &Path) -> Vec<String> {
    let mut names: Vec<(String, bool)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().is_file())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let enabled = !name.ends_with(".disabled");
            let stem = Path::new(name.trim_end_matches(".disabled")).file_stem()?.to_string_lossy().into_owned();
            Some((stem, enabled))
        })
        .collect();
    names.sort_by_key(|(name, _)| name.to_lowercase());
    names.iter().map(|(name, on)| if *on { format!("  [✔] {name}") } else { format!("  [✘] {name} (disabled)") }).collect()
}

/// Ported from `MinecraftInstance::verboseDescription`.
#[allow(clippy::too_many_arguments)]
fn description(
    pack: &PackProfile,
    profile: &LaunchProfile,
    instance: &Instance,
    data_dir: &Path,
    jars: &[PathBuf],
    native_jars: &[PathBuf],
    window: (i64, i64),
    maximized: bool,
    target: Option<&Target>,
) -> Vec<String> {
    let mut out = vec!["Components:".to_string()];
    for (i, c) in pack.components.iter().enumerate() {
        out.push(format!("  {}) {} ({}) {}", i + 1, c.name(), c.uid, c.version));
    }
    out.extend([String::new(), "Launcher: standard".into(), format!("Main class: {}", profile.main_class), String::new()]);

    let mods = mod_lines(&instance.mods_dir());
    if !mods.is_empty() {
        out.push("Mods:".into());
        out.extend(mods);
        out.push(String::new());
    }
    if !profile.traits.is_empty() {
        out.push("Traits:".into());
        out.extend(profile.traits.iter().map(|t| format!("  {t}")));
        out.push(String::new());
    }
    out.push("Libraries:".into());
    out.extend(jars.iter().map(|j| format!("  {}", qt_path(j))));
    out.extend([String::new(), "Native libraries:".into()]);
    out.extend(native_jars.iter().map(|j| format!("  {}", qt_path(j))));
    out.extend([String::new(), "Natives path:".into(), format!("  {}", qt_path(&instance.natives_dir())), String::new()]);

    // Without a session, so no name, id or token appears here.
    let args = game_arguments(profile, &variables(profile, instance, data_dir, None), window, target);
    out.extend(["Minecraft arguments:".into(), format!("  {}", args.join(" ")), String::new()]);
    out.push(if maximized {
        "Window size: max (if available)".into()
    } else {
        format!("Window size: {} x {}", window.0, window.1)
    });
    out.push(String::new());
    out
}

/// Runs every step before the process start: components, Java, downloads, natives.
pub async fn prepare(client: &Client, meta: &Meta, mut request: LaunchRequest<'_>, log: &Log) -> Result<Prepared> {
    let offline = request.session.offline;
    let data_dir = request.data_dir;
    let game_root = request.instance.game_root();
    say(log, format!("Minecraft folder is:\n  {}\n", qt_path(&game_root)));
    // The second folder works around Minecraft bug MCL-3732.
    let server_packs = game_root.join("server-resource-packs");
    std::fs::create_dir_all(&server_packs).map_err(io_err(&server_packs))?;

    let mut pack = PackProfile::load(&request.instance.root)?;
    pack.resolve(meta, offline, true).await?;

    // The Java requirement does not depend on the system, so any context will do here.
    let system = RuntimeContext::current_system().to_string();
    let probe = RuntimeContext { java_architecture: "64".into(), java_real_architecture: "amd64".into(), system: system.clone() };
    let compatible = pack.launch_profile(&probe).compatible_java_majors;
    let java = select_java(&mut request, &compatible, log).await?;

    let ctx = RuntimeContext {
        java_architecture: java.architecture().to_string(),
        java_real_architecture: java.real_architecture.clone(),
        system,
    };
    let profile = pack.launch_profile(&ctx);
    let instance = &*request.instance;
    let settings = Effective { global: request.global, instance: Some(&instance.settings) };

    if profile.traits.contains("legacyLaunch") || profile.traits.contains("alphaLaunch") {
        return Err(Error::Launch("This Minecraft version needs the legacy applet launcher, which is not ported.".into()));
    }
    if profile.has_jar_mods {
        return Err(Error::Launch("This instance uses jar mods, which are not ported.".into()));
    }
    if profile.main_class.is_empty() {
        return Err(Error::Launch("The instance has no main class. Its components are incomplete.".into()));
    }

    let libraries_dir = data_dir.join("libraries");
    let local_dir = instance.local_library_dir();
    let concurrency = settings.get_int("NumberOfConcurrentDownloads").max(1) as usize;
    let agent_libraries = profile.agents.iter().map(|a| &a.library);
    if offline {
        // Ported from `EnsureOfflineLibraries`.
        let missing: Vec<String> = profile
            .all_libraries()
            .chain(agent_libraries)
            .filter_map(|l| l.files(&ctx, &libraries_dir, &local_dir).jar.or(l.files(&ctx, &libraries_dir, &local_dir).native))
            .filter(|path| !path.is_file())
            .map(|path| qt_path(&path))
            .collect();
        if !missing.is_empty() {
            return Err(Error::Launch(format!(
                "Required files are missing and cannot be downloaded in offline mode:\n{}",
                missing.join("\n")
            )));
        }
    } else {
        let mut downloads: BTreeMap<PathBuf, Download> = BTreeMap::new();
        for library in profile.all_libraries().chain(agent_libraries) {
            if let Some(download) = library.download(&ctx, &libraries_dir, &local_dir)? {
                downloads.insert(download.path.clone(), download);
            }
        }
        let downloads: Vec<Download> = downloads.into_values().collect();
        if !downloads.is_empty() {
            say(log, format!("Downloading {} libraries...", downloads.len()));
            client.download_all(&downloads, concurrency, |_, _| {}).await.map_err(|e| {
                Error::Launch(format!("Game update failed: it was impossible to fetch the required libraries.\nReason:\n{e}"))
            })?;
        }
        say(log, "Updating assets...");
        // Atomic rather than Cell so the launch future stays Send.
        let tenth = std::sync::atomic::AtomicUsize::new(0);
        let progress = |done: usize, total: usize| {
            let now = done * 10 / total.max(1);
            if now > tenth.swap(now, std::sync::atomic::Ordering::Relaxed) {
                say(log, format!("Assets: {done} of {total}"));
            }
        };
        assets::update(client, data_dir, &profile.assets(), &settings.get("ResourceURLOverride"), concurrency, progress)
            .await
            .map_err(|e| Error::Launch(format!("Failed to download assets:\n{e}")))?;
    }

    let (jars, native_jars) = profile.library_files(&ctx, &libraries_dir, &local_dir);
    let window = window_size(settings, request.screen_size);
    let target = request.target.clone().or_else(|| {
        // Per-instance "join on launch"; these keys have no global counterpart.
        let get = |key: &str| instance.settings.get(key).unwrap_or_default().to_string();
        if !prism_core::settings::parse_bool(&get("JoinServerOnLaunch")) {
            None
        } else if !get("JoinServerOnLaunchAddress").is_empty() {
            Some(Target::server(&get("JoinServerOnLaunchAddress")))
        } else {
            Some(get("JoinWorldOnLaunch")).filter(|w| !w.is_empty()).map(|w| Target::world(&w))
        }
    });

    let maximized = settings.get_bool("LaunchMaximized");
    for line in description(&pack, &profile, instance, data_dir, &jars, &native_jars, window, maximized, target.as_ref()) {
        say(log, line);
    }

    let natives_dir = instance.natives_dir();
    extract_natives(&native_jars, &natives_dir, java.major >= 8)?;
    assets::reconstruct(data_dir, &profile.assets().id, &instance.resources_dir())?;

    let vars = variables(&profile, instance, data_dir, Some(request.session));
    let agent_jars: Vec<PathBuf> =
        profile.agents.iter().filter_map(|a| a.library.files(&ctx, &libraries_dir, &local_dir).jar).collect();
    let jvm_args = jvm_arguments(&profile, settings, &java, &vars, &agent_jars);
    let censor = request.session.censor_filter();
    say(log, format!("Java arguments:\n  {}\n", crate::launch::censor(&jvm_args.join(" "), &censor)));

    let env = environment(instance, settings, &java.path, &jvm_args);
    let classpath: Vec<String> = jars.iter().map(|jar| qt_path(jar)).collect();
    let mut args = jvm_args;
    args.push(format!("-Djava.library.path={}", qt_path(&natives_dir)));
    args.extend(["-cp".to_string(), classpath.join(if cfg!(windows) { ";" } else { ":" }), profile.main_class.clone()]);
    args.extend(game_arguments(&profile, &vars, window, target.as_ref()));

    Ok(Prepared { java: java.path, args, working_dir: game_root, env, censor, natives_dir })
}

/// Starts the game and forwards its output until it exits or `kill` fires.
/// The `natives` folder is removed afterwards, as `ExtractNatives::finalize` does.
pub async fn run(
    prepared: &Prepared,
    log: &Log,
    kill: tokio::sync::oneshot::Receiver<()>,
) -> Result<std::process::ExitStatus> {
    let mut command = tokio::process::Command::new(&prepared.java);
    command
        .args(&prepared.args)
        .current_dir(&prepared.working_dir)
        .env_clear()
        .envs(prepared.env.iter().map(|(k, v)| (k, v)))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW

    let mut child = command.spawn().map_err(|e| Error::Launch(format!("Could not launch Minecraft: {e}")))?;
    say(log, format!("Minecraft process ID: {}\n", child.id().unwrap_or_default()));

    let pump = |stream: Option<Box<dyn tokio::io::AsyncRead + Send + Unpin>>| {
        let (log, filter) = (log.clone(), prepared.censor.clone());
        tokio::spawn(async move {
            let Some(stream) = stream else { return };
            let mut reader = tokio::io::BufReader::new(stream);
            let mut line = Vec::new();
            // Bytes, not strings: older Java prints in the system code page.
            while reader.read_until(b'\n', &mut line).await.is_ok_and(|read| read > 0) {
                let text = String::from_utf8_lossy(&line);
                say(&log, censor(text.trim_end_matches(['\r', '\n']), &filter));
                line.clear();
            }
        })
    };
    let out = pump(child.stdout.take().map(|s| Box::new(s) as _));
    let err = pump(child.stderr.take().map(|s| Box::new(s) as _));

    let status = tokio::select! {
        status = child.wait() => status,
        _ = kill => {
            say(log, "Killing the game on request...");
            let _ = child.kill().await;
            child.wait().await
        }
    };
    let _ = tokio::join!(out, err);
    let _ = std::fs::remove_dir_all(&prepared.natives_dir);

    let status = status.map_err(|e| Error::Launch(format!("Could not wait for Minecraft: {e}")))?;
    say(log, match status.code() {
        Some(code) => format!("Process exited with code {code}."),
        None => "Process crashed.".to_string(),
    });
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::version_file::VersionFile;
    use std::io::Write;

    fn ctx() -> RuntimeContext {
        RuntimeContext { java_architecture: "64".into(), java_real_architecture: "amd64".into(), system: "windows".into() }
    }

    fn java(major: u32) -> JavaInstall {
        JavaInstall {
            path: "javaw".into(),
            version: major.to_string(),
            major,
            real_architecture: "amd64".into(),
            vendor: String::new(),
            is_64bit: true,
        }
    }

    fn profile() -> LaunchProfile {
        let minecraft = VersionFile::from_json(
            br#"{"uid":"net.minecraft","version":"26.3","type":"release","mainClass":"net.minecraft.client.main.Main",
                "minecraftArguments":"--username ${auth_player_name} --version ${version_name}  --uuid ${auth_uuid} --accessToken ${auth_access_token} --versionType ${version_type} --unknown ${nope}",
                "mainJar":{"name":"com.mojang:minecraft:26.3:client"},"+jvmArgs":["-XX:StackShadowPages=32","-Dlib=${library_directory}"],
                "+traits":["feature:is_quick_play_multiplayer"],"+tweakers":["a.Tweak"]}"#,
            "test",
        )
        .expect("version file");
        let mut profile = LaunchProfile::default();
        profile.apply(&minecraft, &ctx());
        profile
    }

    #[test]
    fn split_args_like_the_original() {
        assert_eq!(split_args(r#"-Xss4m  "-Dname=two words" '-Dq=a "b"' -Dpath="C:\dir\file""#), [
            "-Xss4m",
            "-Dname=two words",
            r#"-Dq=a "b""#,
            r"-Dpath=C:\dir\file",
        ]);
        assert!(split_args("   ").is_empty());
    }

    #[test]
    fn server_addresses() {
        assert_eq!(Target::server("example.org"), Target { address: "example.org".into(), port: 25565, world: String::new() });
        assert_eq!(Target::server("example.org:1234").port, 1234);
        assert_eq!(Target::server("example.org:x").port, 25565);
        assert_eq!(Target::server("[::1]:777"), Target { address: "::1".into(), port: 777, world: String::new() });
        assert_eq!(Target::server("[::1]").port, 25565);
        assert_eq!(Target::server("a:b:c").address, "a:b:c");
    }

    #[test]
    fn arguments_match_the_original_order() {
        let dir = tempfile::tempdir().expect("tempdir");
        let instance = prism_core::instance::create_instance(&dir.path().join("instances"), "Test").expect("instance");
        let global = Settings::load(&dir.path().join("prismlauncher.cfg")).expect("settings");
        let mut own = instance.settings.clone();
        own.set("OverrideJavaArgs", true);
        own.set("JvmArgs", "-Dcustom=1");
        own.set("OverrideMemory", true);
        own.set("MinMemAlloc", 2048);
        own.set("MaxMemAlloc", 1024);
        let settings = Effective { global: &global, instance: Some(&own) };

        let profile = profile();
        let session = AuthSession {
            player_name: "Steve".into(),
            uuid: "0123".into(),
            access_token: "SECRET".into(),
            session: "token:SECRET:0123".into(),
            user_type: "msa".into(),
            offline: false,
        };
        let vars = variables(&profile, &instance, dir.path(), Some(&session));

        let jvm = jvm_arguments(&profile, settings, &java(25), &vars, &[]);
        let mut expected = vec![
            "-Duser.language=en".to_string(),
            "-Dcustom=1".into(),
            "-XX:StackShadowPages=32".into(),
            format!("-Dlib={}", qt_path(&dir.path().join("libraries"))),
        ];
        if cfg!(windows) {
            expected.push("-XX:HeapDumpPath=MojangTricksIntelDriversForPerformance_javaw.exe_minecraft.exe.heapdump".into());
        }
        // Swapped limits are put in order.
        expected.extend(["-Xms1024m".to_string(), "-Xmx2048m".into()]);
        assert_eq!(jvm, expected);
        assert!(jvm_arguments(&profile, settings, &java(7), &vars, &[]).contains(&"-XX:PermSize=128m".to_string()));

        let game = game_arguments(&profile, &vars, window_size(settings, None), Some(&Target::server("mc.example.org")));
        assert_eq!(game.join(" "), "--username Steve --version 26.3 --uuid 0123 --accessToken SECRET --versionType release \
             --unknown  --tweakClass a.Tweak --width 854 --height 480 --quickPlayMultiplayer mc.example.org:25565");

        // The description never sees the session, and the censor covers the rest.
        let anonymous = game_arguments(&profile, &variables(&profile, &instance, dir.path(), None), (1, 2), None);
        assert!(anonymous.join(" ").starts_with("--username  --version 26.3 --uuid  --accessToken  "));
        let prepared = Prepared {
            java: "javaw".into(),
            args: game,
            working_dir: PathBuf::new(),
            env: Vec::new(),
            censor: session.censor_filter(),
            natives_dir: PathBuf::new(),
        };
        let line = prepared.censored_command_line();
        assert!(!line.contains("SECRET") && line.contains("<ACCESS TOKEN>") && line.contains("<PROFILE ID>"), "{line}");
    }

    #[test]
    fn maximized_uses_the_screen_size_when_known() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut global = Settings::load(&dir.path().join("prismlauncher.cfg")).expect("settings");
        global.set("LaunchMaximized", true);
        let settings = Effective { global: &global, instance: None };
        assert_eq!(window_size(settings, Some((1920, 1040))), (1920, 1040));
        assert_eq!(window_size(settings, None), (854, 480));
    }

    #[test]
    fn environment_drops_java_overrides_and_adds_instance_info() {
        let dir = tempfile::tempdir().expect("tempdir");
        let instance = prism_core::instance::create_instance(dir.path(), "Env Test").expect("instance");
        let mut global = Settings::load(&dir.path().join("prismlauncher.cfg")).expect("settings");
        global.set("Env", r#"{"MY_VAR":"yes"}"#);
        let settings = Effective { global: &global, instance: Some(&instance.settings) };
        let env = environment(&instance, settings, Path::new("javaw"), &["-Xmx1m".to_string()]);
        let get = |key: &str| env.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v.to_string_lossy().into_owned());
        assert_eq!(get("INST_NAME").as_deref(), Some("Env Test"));
        assert_eq!(get("INST_JAVA_ARGS").as_deref(), Some("-Xmx1m"));
        assert_eq!(get("MY_VAR").as_deref(), Some("yes"));
        assert_eq!(get("NO_COLOR").as_deref(), Some("1"));
        assert!(!env.iter().any(|(k, _)| k.to_string_lossy().eq_ignore_ascii_case("_JAVA_OPTIONS")));
    }

    #[test]
    fn natives_are_extracted_and_escapes_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let make_jar = |name: &str, entry: &str| {
            let path = dir.path().join(name);
            let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).expect("create"));
            zip.start_file(entry, zip::write::SimpleFileOptions::default()).expect("start");
            zip.write_all(b"binary").expect("write");
            zip.finish().expect("finish");
            path
        };
        let target = dir.path().join("natives");
        let good = make_jar("good.jar", "sub/lwjgl.dll");
        let mac = make_jar("mac.jar", "liblwjgl.jnilib");
        extract_natives(&[good, mac], &target, true).expect("extract");
        assert_eq!(std::fs::read(target.join("sub/lwjgl.dll")).expect("read"), b"binary");
        assert!(target.join("liblwjgl.dylib").is_file());

        let evil = make_jar("evil.jar", "../outside.dll");
        assert!(extract_natives(&[evil], &target, true).is_err());
        assert!(!dir.path().join("outside.dll").exists());
    }

    #[test]
    fn mods_are_listed_with_their_state() {
        let dir = tempfile::tempdir().expect("tempdir");
        for name in ["sodium-1.0.jar", "Iris.jar.disabled"] {
            std::fs::write(dir.path().join(name), b"").expect("write");
        }
        assert_eq!(mod_lines(dir.path()), ["  [✘] Iris (disabled)", "  [✔] sodium-1.0"]);
    }
}
