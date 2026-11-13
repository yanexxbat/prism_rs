//! Global and per-instance settings.
//! Ported from `SettingsObject`, `OverrideSetting` and the registrations in
//! `Application.cpp`, `BaseInstance.cpp` and `MinecraftInstance.cpp`.

use crate::ini::IniFile;
use crate::Result;
use std::path::{Path, PathBuf};

/// One INI file on disk.
#[derive(Clone, Debug)]
pub struct Settings {
    path: PathBuf,
    ini: IniFile,
}

impl Settings {
    pub fn load(path: &Path) -> Result<Self> {
        Ok(Settings { path: path.to_path_buf(), ini: IniFile::load(path)? })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.ini.0.get(key).map(String::as_str)
    }

    pub fn set(&mut self, key: &str, value: impl ToString) {
        self.ini.0.insert(key.to_string(), value.to_string());
    }

    pub fn remove(&mut self, key: &str) {
        self.ini.0.remove(key);
    }

    pub fn save(&self) -> Result<()> {
        self.ini.save(&self.path)
    }
}

/// Default of a global setting. Keys the port does not use have no default.
pub fn global_default(key: &str) -> &'static str {
    match key {
        "InstanceDir" => "instances",
        "JavaDir" => "java",
        "MinMemAlloc" => "512",
        // shortcut: the original uses RAM / 1.5 on machines under 6 GB, upgrade if small machines matter
        "MaxMemAlloc" => "4096",
        "PermGen" => "128",
        "MinecraftWinWidth" => "854",
        "MinecraftWinHeight" => "480",
        "AutomaticJavaSwitch" | "ShowConsoleOnError" => "true",
        "NumberOfConcurrentDownloads" => "6",
        "Env" => "{}",
        _ => "",
    }
}

/// The instance key that must be `true` before the instance's own value of `key` is used.
pub fn override_gate(key: &str) -> Option<&'static str> {
    Some(match key {
        "JavaPath" | "IgnoreJavaCompatibility" | "JavaArchitecture" | "JavaRealArchitecture" | "JavaVersion"
        | "JavaVendor" => "OverrideJavaLocation",
        "JvmArgs" => "OverrideJavaArgs",
        "LaunchMaximized" | "MinecraftWinWidth" | "MinecraftWinHeight" => "OverrideWindow",
        "MinMemAlloc" | "MaxMemAlloc" | "PermGen" | "LowMemWarning" => "OverrideMemory",
        "Env" => "OverrideEnv",
        "ShowConsole" | "AutoCloseConsole" | "ShowConsoleOnError" => "OverrideConsole",
        "CloseAfterLaunch" | "QuitAfterGameStop" => "OverrideMiscellaneous",
        _ => return None,
    })
}

/// `QVariant::toBool` for strings: empty, "0" and "false" are false.
pub fn parse_bool(value: &str) -> bool {
    !(value.is_empty() || value == "0" || value.eq_ignore_ascii_case("false"))
}

/// Read access to a setting with the override rules applied.
#[derive(Clone, Copy)]
pub struct Effective<'a> {
    pub global: &'a Settings,
    pub instance: Option<&'a Settings>,
}

impl Effective<'_> {
    pub fn get(&self, key: &str) -> String {
        let global = || self.global.get(key).unwrap_or(global_default(key)).to_string();
        let Some(instance) = self.instance else { return global() };
        match override_gate(key) {
            // An active override with no stored value yields the default, not the global value.
            Some(gate) if instance.get(gate).is_some_and(parse_bool) => {
                instance.get(key).unwrap_or(global_default(key)).to_string()
            }
            Some(_) => global(),
            None => instance.get(key).map(str::to_string).unwrap_or_else(global),
        }
    }

    pub fn get_bool(&self, key: &str) -> bool {
        parse_bool(&self.get(key))
    }

    pub fn get_int(&self, key: &str) -> i64 {
        self.get(key).trim().parse().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(pairs: &[(&str, &str)]) -> Settings {
        let mut s = Settings { path: PathBuf::new(), ini: IniFile::default() };
        for (k, v) in pairs {
            s.set(k, v);
        }
        s
    }

    #[test]
    fn instance_value_needs_its_gate() {
        let global = settings(&[("MaxMemAlloc", "8192")]);
        let closed = settings(&[("MaxMemAlloc", "2048")]);
        let open = settings(&[("MaxMemAlloc", "2048"), ("OverrideMemory", "true")]);
        let open_without_value = settings(&[("OverrideMemory", "true")]);

        let get = |instance| Effective { global: &global, instance }.get_int("MaxMemAlloc");
        assert_eq!(get(None), 8192);
        assert_eq!(get(Some(&closed)), 8192);
        assert_eq!(get(Some(&open)), 2048);
        assert_eq!(get(Some(&open_without_value)), 4096);
    }

    #[test]
    fn ungated_keys_fall_back_to_global_then_default() {
        let global = settings(&[]);
        let instance = settings(&[("name", "Test")]);
        let effective = Effective { global: &global, instance: Some(&instance) };
        assert_eq!(effective.get("name"), "Test");
        assert_eq!(effective.get("InstanceDir"), "instances");
        assert!(effective.get_bool("AutomaticJavaSwitch"));
    }
}
