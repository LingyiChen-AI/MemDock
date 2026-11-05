use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::Path;

/// User-level Memdock settings persisted as JSON.
#[derive(Serialize, Deserialize, Default, Debug, Clone, PartialEq, Eq)]
#[serde(default)]
pub struct Settings {
    pub language: Option<String>,
    /// "dark" | "light" | "system"; empty means "dark".
    pub theme: Option<String>,
    pub manual_projects: Vec<String>,
}

/// Loads settings from `path`. A missing or unparseable file yields `Settings::default()`.
pub fn load_settings(path: &Path) -> Settings {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Saves settings atomically: writes `<path>.tmp` first, then renames it over `path`.
pub fn save_settings(path: &Path, s: &Settings) -> io::Result<()> {
    let json = serde_json::to_string_pretty(s)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let mut tmp_name = path
        .file_name()
        .map(OsString::from)
        .unwrap_or_else(|| OsString::from("settings.json"));
    tmp_name.push(".tmp");
    let tmp_path = path.with_file_name(tmp_name);
    fs::write(&tmp_path, json)?;
    fs::rename(&tmp_path, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_settings_missing_or_corrupt_returns_default() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("nope.json");
        assert_eq!(load_settings(&missing), Settings::default());

        let corrupt = tmp.path().join("bad.json");
        std::fs::write(&corrupt, "{ not json").unwrap();
        assert_eq!(load_settings(&corrupt), Settings::default());
    }

    #[test]
    fn save_then_load_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("settings.json");
        let s = Settings {
            language: Some("zh-CN".to_string()),
            theme: Some("dark".to_string()),
            manual_projects: vec!["/a".to_string()],
        };
        save_settings(&path, &s).unwrap();
        let loaded = load_settings(&path);
        assert_eq!(loaded.manual_projects, vec!["/a".to_string()]);
        assert_eq!(loaded, s);
        // The temporary file must not be left behind after the atomic rename.
        assert!(!tmp.path().join("settings.json.tmp").exists());
    }
}
