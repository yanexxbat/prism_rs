//! File helpers, ported from `FileSystem.cpp`.

use crate::{Error, Result};
use std::path::{Path, PathBuf};

pub fn io_err(path: &Path) -> impl FnOnce(std::io::Error) -> Error + '_ {
    move |source| Error::Io { path: path.to_path_buf(), source }
}

pub fn read_to_string(path: &Path) -> Result<String> {
    std::fs::read_to_string(path).map_err(io_err(path))
}

pub fn create_dir_all(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path).map_err(io_err(path))
}

/// Writes through a temporary file so a crash never leaves a half-written config.
pub fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        create_dir_all(parent)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, data).map_err(io_err(&tmp))?;
    std::fs::rename(&tmp, path).map_err(io_err(path))
}

pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let text = read_to_string(path)?;
    serde_json::from_str(&text).map_err(|source| Error::Json { path: path.to_path_buf(), source })
}

pub fn write_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<()> {
    let text = serde_json::to_string_pretty(value)
        .map_err(|source| Error::Json { path: path.to_path_buf(), source })?;
    write_atomic(path, text.as_bytes())
}

const BAD_CHARS: &str = "<>:\"|?*\r\n!";

fn remove_chars(source: &str, replace: char, extra: &str) -> String {
    source
        .chars()
        .map(|c| if c.is_control() || BAD_CHARS.contains(c) || extra.contains(c) { replace } else { c })
        .collect()
}

pub fn remove_invalid_path_chars(path: &str) -> String {
    remove_chars(path, '-', "")
}

pub fn remove_invalid_filename_chars(name: &str) -> String {
    remove_chars(name, '-', "\\/")
}

/// A folder name for `name` that does not exist in `dir` yet: `name`, `name(1)`, `name(2)`…
pub fn dir_name_from_string(name: &str, dir: &Path) -> Result<String> {
    let base = remove_invalid_filename_chars(name);
    (0..=9000)
        .map(|n| if n == 0 { base.clone() } else { format!("{base}({n})") })
        .find(|candidate| !dir.join(candidate).exists())
        .ok_or_else(|| Error::Other(format!("no free folder name for '{name}'")))
}

/// Absolute path with forward slashes, the form Qt hands to Java.
pub fn qt_path(path: &Path) -> String {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    absolute.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_names_avoid_collisions_and_bad_chars() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(dir_name_from_string("a/b:c", dir.path()).expect("name"), "a-b-c");
        std::fs::create_dir(dir.path().join("a-b-c")).expect("mkdir");
        assert_eq!(dir_name_from_string("a/b:c", dir.path()).expect("name"), "a-b-c(1)");
    }
}
