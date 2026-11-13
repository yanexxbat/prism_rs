//! Finding and checking Java runtimes.
//! Ported from `JavaUtils`, `JavaChecker`, `JavaVersion` and `JavaInstallList`.

use crate::{Error, Result};
use std::path::{Path, PathBuf};

pub const JAVA_EXECUTABLE: &str = if cfg!(windows) { "javaw.exe" } else { "java" };

#[derive(Clone, Debug, PartialEq)]
pub struct JavaInstall {
    pub path: PathBuf,
    /// `java.version`, e.g. `21.0.13` or `1.8.0_392`.
    pub version: String,
    pub major: u32,
    /// `os.arch`, e.g. `amd64`.
    pub real_architecture: String,
    pub vendor: String,
    pub is_64bit: bool,
}

impl JavaInstall {
    /// `32` or `64`, the form library paths use for `${arch}`.
    pub fn architecture(&self) -> &'static str {
        if self.is_64bit {
            "64"
        } else {
            "32"
        }
    }

    pub fn requires_perm_gen(&self) -> bool {
        self.major < 8
    }
}

/// `1.8.0_392` is Java 8; `21.0.13` is Java 21. Unparseable versions give 0.
pub fn major_version(version: &str) -> u32 {
    let mut numbers = version.split(|c: char| !c.is_ascii_digit()).map(|n| n.parse::<u32>().unwrap_or(0));
    match (numbers.next(), numbers.next()) {
        (Some(1), Some(minor)) => minor,
        (Some(major), _) => major,
        _ => 0,
    }
}

/// Reads the three properties from `java -XshowSettings:properties -version` output.
fn parse_properties(path: &Path, output: &str) -> Result<JavaInstall> {
    let property = |name: &str| {
        output.lines().find_map(|line| {
            let (key, value) = line.split_once('=')?;
            (key.trim() == name).then(|| value.trim().to_string())
        })
    };
    let missing = || Error::Java(format!("{} did not report its version", path.display()));
    let version = property("java.version").ok_or_else(missing)?;
    let real_architecture = property("os.arch").ok_or_else(missing)?;
    let is_64bit =
        matches!(real_architecture.as_str(), "x86_64" | "amd64" | "aarch64" | "arm64" | "riscv64" | "ppc64le" | "ppc64");
    Ok(JavaInstall {
        path: path.to_path_buf(),
        major: major_version(&version),
        version,
        real_architecture,
        vendor: property("java.vendor").unwrap_or_default(),
        is_64bit,
    })
}

/// Runs the binary to learn its version and architecture.
pub async fn check(path: &Path) -> Result<JavaInstall> {
    let mut command = tokio::process::Command::new(path);
    command.args(["-XshowSettings:properties", "-version"]).stdin(std::process::Stdio::null());
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: no console flash for java.exe
    let output = tokio::time::timeout(std::time::Duration::from_secs(15), command.output())
        .await
        .map_err(|_| Error::Java(format!("{} did not answer in time", path.display())))?
        .map_err(|e| Error::Java(format!("The Java binary \"{}\" couldn't be run: {e}", path.display())))?;
    // The settings go to stderr, but an old JVM may use stdout.
    let text = format!("{}\n{}", String::from_utf8_lossy(&output.stderr), String::from_utf8_lossy(&output.stdout));
    parse_properties(path, &text)
}

/// Looks for `bin/<java>` below `dir`, a few levels deep. Covers `<data>/java` and
/// the Mojang launcher's `runtime/<name>/<platform>/<name>/bin` layout.
fn scan_dir(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let binary = dir.join("bin").join(JAVA_EXECUTABLE);
    if binary.is_file() {
        out.push(binary);
        return;
    }
    if depth == 0 {
        return;
    }
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        if entry.path().is_dir() {
            scan_dir(&entry.path(), depth - 1, out);
        }
    }
}

#[cfg(windows)]
fn registry_candidates(out: &mut Vec<PathBuf>) {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_ENUMERATE_SUB_KEYS, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY};
    use winreg::RegKey;
    // (key, value holding the install folder, sub-key below each version)
    const KEYS: &[(&str, &str, &str)] = &[
        ("SOFTWARE\\JavaSoft\\Java Runtime Environment", "JavaHome", ""),
        ("SOFTWARE\\JavaSoft\\Java Development Kit", "JavaHome", ""),
        ("SOFTWARE\\JavaSoft\\JRE", "JavaHome", ""),
        ("SOFTWARE\\JavaSoft\\JDK", "JavaHome", ""),
        ("SOFTWARE\\AdoptOpenJDK\\JRE", "Path", "\\hotspot\\MSI"),
        ("SOFTWARE\\AdoptOpenJDK\\JDK", "Path", "\\hotspot\\MSI"),
        ("SOFTWARE\\Eclipse Foundation\\JDK", "Path", "\\hotspot\\MSI"),
        ("SOFTWARE\\Eclipse Adoptium\\JRE", "Path", "\\hotspot\\MSI"),
        ("SOFTWARE\\Eclipse Adoptium\\JDK", "Path", "\\hotspot\\MSI"),
        ("SOFTWARE\\Semeru\\JRE", "Path", "\\openj9\\MSI"),
        ("SOFTWARE\\Semeru\\JDK", "Path", "\\openj9\\MSI"),
        ("SOFTWARE\\Microsoft\\JDK", "Path", "\\hotspot\\MSI"),
        ("SOFTWARE\\Azul Systems\\Zulu", "InstallationPath", ""),
        ("SOFTWARE\\BellSoft\\Liberica", "InstallationPath", ""),
    ];
    for hive in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        for view in [KEY_WOW64_64KEY, KEY_WOW64_32KEY] {
            for (key, value, suffix) in KEYS {
                let Ok(versions) = RegKey::predef(hive).open_subkey_with_flags(key, KEY_READ | KEY_ENUMERATE_SUB_KEYS | view)
                else {
                    continue;
                };
                for version in versions.enum_keys().flatten() {
                    let home = versions
                        .open_subkey_with_flags(format!("{version}{suffix}"), KEY_READ | view)
                        .and_then(|k| k.get_value::<String, _>(value));
                    if let Ok(home) = home {
                        out.push(Path::new(&home).join("bin").join(JAVA_EXECUTABLE));
                    }
                }
            }
        }
    }
}

#[cfg(not(windows))]
fn registry_candidates(out: &mut Vec<PathBuf>) {
    for dir in ["/usr/lib/jvm", "/usr/lib64/jvm", "/usr/lib32/jvm", "/opt/jdk", "/opt/jdks", "/Library/Java/JavaVirtualMachines"] {
        scan_dir(Path::new(dir), 4, out);
    }
}

/// Paths that may be Java binaries. `java_dir` is the launcher's own Java folder.
pub fn find_java_paths(java_dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    registry_candidates(&mut out);
    scan_dir(java_dir, 4, &mut out);
    if let Some(appdata) = std::env::var_os("APPDATA") {
        scan_dir(&Path::new(&appdata).join(".minecraft").join("runtime"), 4, &mut out);
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let store = Path::new(&local).join("Packages").join("Microsoft.4297127D64EC6_8wekyb3d8bbwe");
        scan_dir(&store.join("LocalCache").join("Local").join("runtime"), 4, &mut out);
    }
    if let Some(home) = std::env::var_os("JAVA_HOME") {
        out.push(Path::new(&home).join("bin").join(JAVA_EXECUTABLE));
    }
    if let Some(path) = std::env::var_os("PATH") {
        out.extend(std::env::split_paths(&path).map(|dir| dir.join(JAVA_EXECUTABLE)));
    }
    let mut seen = std::collections::BTreeSet::new();
    out.retain(|p| p.is_file() && seen.insert(p.to_string_lossy().to_lowercase()));
    out
}

/// Every working Java found, 64-bit first, then newest first.
pub async fn detect(java_dir: &Path) -> Vec<JavaInstall> {
    let checks = find_java_paths(java_dir).into_iter().map(|path| async move { check(&path).await });
    let mut installs: Vec<JavaInstall> = futures_util::future::join_all(checks)
        .await
        .into_iter()
        .filter_map(|result| result.map_err(|e| tracing::debug!("{e}")).ok())
        .collect();
    installs.sort_by(|a, b| {
        let key = |j: &JavaInstall| (j.is_64bit, j.major, prism_core::version::Version::new(&j.version));
        key(b).cmp(&key(a))
    });
    installs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn major_versions() {
        assert_eq!(major_version("1.8.0_392"), 8);
        assert_eq!(major_version("21.0.13"), 21);
        assert_eq!(major_version("25"), 25);
        assert_eq!(major_version("17.0.9+9-LTS"), 17);
        assert_eq!(major_version("garbage"), 0);
    }

    #[test]
    fn parses_show_settings_output() {
        let output = "Property settings:\n    java.vendor = Eclipse Adoptium\n    java.version = 21.0.13\n    \
                      java.version.date = 2026-10-20\n    os.arch = amd64\n    os.name = Windows 11\n\n\
                      openjdk version \"21.0.13\" 2026-10-20 LTS\n";
        let java = parse_properties(Path::new("javaw.exe"), output).expect("parse");
        assert_eq!((java.major, java.version.as_str(), java.vendor.as_str()), (21, "21.0.13", "Eclipse Adoptium"));
        assert_eq!((java.real_architecture.as_str(), java.architecture()), ("amd64", "64"));
        assert!(parse_properties(Path::new("x"), "Error: could not open jvm.cfg").is_err());
    }

    #[test]
    fn scan_finds_nested_runtimes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bin = dir.path().join("java-runtime-epsilon/windows-x64/java-runtime-epsilon/bin");
        std::fs::create_dir_all(&bin).expect("mkdir");
        std::fs::write(bin.join(JAVA_EXECUTABLE), b"").expect("write");
        let mut found = Vec::new();
        scan_dir(dir.path(), 4, &mut found);
        assert_eq!(found, [bin.join(JAVA_EXECUTABLE)]);
    }
}
