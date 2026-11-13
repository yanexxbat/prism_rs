//! Qt `QSettings` INI format, as used by `prismlauncher.cfg` and `instance.cfg`.
//! Ported from `settings/INIFile.cpp` and Qt's INI escaping rules.

use crate::{fs, Result};
use std::collections::BTreeMap;
use std::path::Path;

/// Keys of the `[General]` section have no prefix; other sections use `Section/key`.
#[derive(Default, Clone, Debug, PartialEq)]
pub struct IniFile(pub BTreeMap<String, String>);

impl IniFile {
    pub fn parse(text: &str) -> Self {
        let mut map = BTreeMap::new();
        let mut section = String::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                let name = unescape_key(name);
                section = if name.eq_ignore_ascii_case("general") { String::new() } else { format!("{name}/") };
                continue;
            }
            if let Some((key, value)) = line.split_once('=') {
                let key = unescape_key(key.trim()).replace('\\', "/");
                map.insert(format!("{section}{key}"), unescape_value(value.trim()));
            }
        }
        IniFile(map)
    }

    /// A missing file is an empty config, as with `QSettings`.
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        Ok(Self::parse(&fs::read_to_string(path)?))
    }

    pub fn serialize(&self) -> String {
        let eol = if cfg!(windows) { "\r\n" } else { "\n" };
        let mut sections: BTreeMap<&str, Vec<(&str, &str)>> = BTreeMap::new();
        for (key, value) in &self.0 {
            let (section, key) = key.split_once('/').unwrap_or(("", key));
            sections.entry(section).or_default().push((key, value));
        }
        // "ConfigVersion" marks the file as the current format for the original launcher.
        let general = sections.entry("").or_default();
        if !general.iter().any(|(k, _)| *k == "ConfigVersion") {
            general.insert(0, ("ConfigVersion", "1.3"));
        }
        let mut out = String::new();
        for (section, entries) in sections {
            let name = if section.is_empty() { "General".to_string() } else { escape_key(section) };
            out += &format!("[{name}]{eol}");
            for (key, value) in entries {
                out += &format!("{}={}{eol}", escape_key(&key.replace('/', "\\")), escape_value(value));
            }
            out += eol;
        }
        out
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        fs::write_atomic(path, self.serialize().as_bytes())
    }
}

fn unescape_value(raw: &str) -> String {
    let mut out = String::new();
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {}
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('a') => out.push('\x07'),
                Some('b') => out.push('\x08'),
                Some('f') => out.push('\x0c'),
                Some('v') => out.push('\x0b'),
                Some('0') => out.push('\0'),
                Some('x') => {
                    let mut hex = String::new();
                    while let Some(d) = chars.peek().filter(|d| d.is_ascii_hexdigit()) {
                        hex.push(*d);
                        chars.next();
                    }
                    if let Some(decoded) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                        out.push(decoded);
                    }
                }
                Some(other) => out.push(other),
                None => {}
            },
            other => out.push(other),
        }
    }
    match out.strip_prefix("@@") {
        Some(rest) => format!("@{rest}"),
        None => out,
    }
}

fn escape_value(value: &str) -> String {
    let mut out = String::new();
    let mut needs_quotes = false;
    // Qt writes a hex escape greedily, so a following hex digit must be escaped too.
    let mut escape_next_if_hex = false;
    if value.starts_with('@') {
        out.push('@');
    }
    for c in value.chars() {
        if matches!(c, ';' | ',' | '=') {
            needs_quotes = true;
        }
        if escape_next_if_hex && c.is_ascii_hexdigit() {
            out += &format!("\\x{:x}", c as u32);
            continue;
        }
        escape_next_if_hex = false;
        match c {
            '\0' => {
                out += "\\0";
                escape_next_if_hex = true;
            }
            '\x07' => out += "\\a",
            '\x08' => out += "\\b",
            '\x0c' => out += "\\f",
            '\n' => out += "\\n",
            '\r' => out += "\\r",
            '\t' => out += "\\t",
            '\x0b' => out += "\\v",
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) <= 0x1f => {
                out += &format!("\\x{:x}", c as u32);
                escape_next_if_hex = true;
            }
            c => out.push(c),
        }
    }
    if needs_quotes || out.starts_with(' ') || out.ends_with(' ') {
        format!("\"{out}\"")
    } else {
        out
    }
}

fn escape_key(key: &str) -> String {
    let mut out = String::new();
    for c in key.chars() {
        match c {
            c if c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '\\') => out.push(c),
            c if (c as u32) <= 0xff => out += &format!("%{:02X}", c as u32),
            c => out += &format!("%U{:04X}", c as u32),
        }
    }
    out
}

fn unescape_key(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let (digits, skip) = match (chars[i], chars.get(i + 1)) {
            ('%', Some('U')) => (4, 2),
            ('%', Some(_)) => (2, 1),
            _ => (0, 0),
        };
        let hex: String = chars.iter().skip(i + skip).take(digits).collect();
        match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
            Some(decoded) if digits > 0 && hex.len() == digits => {
                out.push(decoded);
                i += skip + digits;
            }
            _ => {
                out.push(chars[i]);
                i += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_prism_instance_cfg() {
        let text = "[General]\r\nConfigVersion=1.3\r\nname=26.3\r\nJavaPath=C:/java/bin/javaw.exe\r\n\
                    JvmArgs=\"-Dfoo=bar -Xss4m\"\r\nnotes=line1\\nline2\r\nOverrideJavaLocation=true\r\n\r\n\
                    [UI]\r\nmods_Page\\Columns=\"AAAA/w==\"\r\n";
        let ini = IniFile::parse(text);
        assert_eq!(ini.0["name"], "26.3");
        assert_eq!(ini.0["JvmArgs"], "-Dfoo=bar -Xss4m");
        assert_eq!(ini.0["notes"], "line1\nline2");
        assert_eq!(ini.0["UI/mods_Page/Columns"], "AAAA/w==");
    }

    #[test]
    fn reads_old_format_without_section() {
        let ini = IniFile::parse("# comment\nname=Old\nIntendedVersion=1.7.10\n");
        assert_eq!(ini.0["name"], "Old");
        assert_eq!(ini.0["IntendedVersion"], "1.7.10");
    }

    #[test]
    fn round_trips_special_values() {
        let mut ini = IniFile::default();
        for (key, value) in [
            ("plain", "value"),
            ("quoted", "a=b;c,d"),
            ("escapes", "tab\there \"q\" back\\slash\nnewline"),
            ("spaces", " padded "),
            ("at", "@home"),
            ("unicode", "Überlebenswelt ✔"),
            ("control", "\u{1}f"),
            ("empty", ""),
            ("UI/mods_Page/Columns", "AAAA"),
        ] {
            ini.0.insert(key.to_string(), value.to_string());
        }
        let text = ini.serialize();
        assert!(text.starts_with("[General]"));
        assert!(text.contains("quoted=\"a=b;c,d\""));
        assert!(text.contains("mods_Page\\Columns=AAAA"));
        let mut expected = ini.clone();
        expected.0.insert("ConfigVersion".into(), "1.3".into());
        assert_eq!(IniFile::parse(&text), expected);
    }
}
