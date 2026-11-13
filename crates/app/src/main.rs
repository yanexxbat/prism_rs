//! prism_rs: a Rust port of Prism Launcher. See NOTICE.md.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
#[cfg(test)]
mod gui_tests;
mod theme;
mod windows;

use std::path::PathBuf;

/// Command line, with the option names of the original (`Application.cpp`).
#[derive(Debug, Default, PartialEq)]
pub struct Cli {
    pub dir: Option<PathBuf>,
    pub launch: Option<String>,
    pub profile: Option<String>,
    pub offline: Option<String>,
    pub server: Option<String>,
    pub world: Option<String>,
}

const USAGE: &str = "prism_rs [options]
  -d, --dir <directory>   Use a custom path as application root
  -l, --launch <instance> Launch the specified instance (by instance ID)
  -s, --server <address>  Join the specified server on launch (only with --launch)
  -w, --world <world>     Join the specified world on launch (only with --launch)
  -a, --profile <profile> Use the account specified by its profile name (only with --launch)
  -o, --offline <name>    Launch offline, with given player name (only with --launch)";

fn parse_cli(args: impl Iterator<Item = String>) -> Result<Cli, String> {
    let mut cli = Cli::default();
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        let (name, inline) = match arg.split_once('=') {
            Some((name, value)) if name.starts_with("--") => (name.to_string(), Some(value.to_string())),
            _ => (arg.clone(), None),
        };
        let mut value = || inline.clone().or_else(|| args.next()).ok_or(format!("{name} needs a value"));
        match name.as_str() {
            "-d" | "--dir" => cli.dir = Some(PathBuf::from(value()?)),
            "-l" | "--launch" => cli.launch = Some(value()?),
            "-a" | "--profile" => cli.profile = Some(value()?),
            "-o" | "--offline" => cli.offline = Some(value()?),
            "-s" | "--server" => cli.server = Some(value()?),
            "-w" | "--world" => cli.world = Some(value()?),
            "-h" | "--help" => return Err(USAGE.to_string()),
            other => return Err(format!("unknown option {other}\n{USAGE}")),
        }
    }
    let launch_only = cli.profile.is_some() || cli.offline.is_some() || cli.server.is_some() || cli.world.is_some();
    if launch_only && cli.launch.is_none() {
        return Err("--server, --world, --profile and --offline can only be used in combination with --launch!".into());
    }
    if cli.server.is_some() && cli.world.is_some() {
        return Err("--server and --world cannot be used together".into());
    }
    Ok(cli)
}

/// `%APPDATA%\prism_rs`, or the XDG data folder elsewhere. Never Prism Launcher's own folder.
fn default_data_dir() -> PathBuf {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local").join("share")))
    };
    base.unwrap_or_else(|| PathBuf::from(".")).join("prism_rs")
}

fn main() -> std::process::ExitCode {
    let cli = match parse_cli(std::env::args().skip(1)) {
        Ok(cli) => cli,
        Err(message) => {
            eprintln!("{message}");
            return std::process::ExitCode::FAILURE;
        }
    };
    tracing_subscriber::fmt().with_writer(std::io::stderr).init();

    let data_dir = cli.dir.clone().unwrap_or_else(default_data_dir);
    let run = || -> Result<(), String> {
        let data_dir = std::path::absolute(&data_dir).map_err(|e| format!("{}: {e}", data_dir.display()))?;
        std::fs::create_dir_all(&data_dir).map_err(|e| format!("{}: {e}", data_dir.display()))?;
        let runtime = tokio::runtime::Runtime::new().map_err(|e| format!("could not start the async runtime: {e}"))?;
        let state = app::App::new(runtime.handle().clone(), data_dir, cli).map_err(|e| e.to_string())?;
        let options = eframe::NativeOptions {
            viewport: eframe::egui::ViewportBuilder::default().with_inner_size([960.0, 600.0]).with_title("prism_rs").with_transparent(true),
            ..Default::default()
        };
        eframe::run_native("prism_rs", options, Box::new(move |_creation| {
            // Blurs the desktop behind the window. Without it the window is still see-through.
            #[cfg(windows)]
            if let Err(e) = window_vibrancy::apply_acrylic(_creation, Some((24, 24, 26, 110))) {
                tracing::warn!("no acrylic blur: {e}");
            }
            Ok(Box::new(state))
        })).map_err(|e| e.to_string())
    };
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Cli, String> {
        parse_cli(args.iter().map(|a| a.to_string()))
    }

    #[test]
    fn options_of_the_original() {
        let cli = parse(&["-d", "data", "--launch=My Instance", "-o", "Steve", "--server", "mc.example.org"]).expect("cli");
        assert_eq!(cli.dir, Some(PathBuf::from("data")));
        assert_eq!(cli.launch.as_deref(), Some("My Instance"));
        assert_eq!((cli.offline.as_deref(), cli.server.as_deref()), (Some("Steve"), Some("mc.example.org")));
        assert_eq!(parse(&[]).expect("cli"), Cli::default());
    }

    #[test]
    fn bad_combinations_are_refused() {
        assert!(parse(&["--offline", "Steve"]).is_err(), "needs --launch");
        assert!(parse(&["-l", "a", "-s", "x", "-w", "y"]).is_err());
        assert!(parse(&["--launch"]).is_err(), "missing value");
        assert!(parse(&["--nope"]).is_err());
    }
}
