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
    /// "green" | "blue" | "violet" | "orange" | "teal"; empty means "green".
    pub accent: Option<String>,
    pub manual_projects: Vec<String>,
    /// Opt-in: query Claude Code's quota endpoint with its OAuth token.
    pub claude_quota_enabled: bool,
    /// Opt-in: query Codex's live quota with its local ChatGPT login token.
    pub codex_quota_enabled: bool,
}

/// Loads settings from `path`. A missing file yields `Settings::default()`.
/// An existing but unparseable file is copied to `<path>.bak` (kept if one
/// already exists, otherwise a timestamped copy) before defaults are returned,
/// so the next save cannot silently destroy the user's data.
pub fn load_settings(path: &Path) -> Settings {
    let Ok(text) = fs::read_to_string(path) else {
        return Settings::default();
    };
    match serde_json::from_str(&text) {
        Ok(s) => s,
        Err(_) => {
            backup_corrupt(path);
            Settings::default()
        }
    }
}

fn backup_corrupt(path: &Path) {
    let base = path
        .file_name()
        .map(OsString::from)
        .unwrap_or_else(|| OsString::from("settings.json"));
    let mut bak = base.clone();
    bak.push(".bak");
    let mut target = path.with_file_name(bak);
    if target.exists() {
        let ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let mut named = base;
        named.push(format!(".{ms}.bak"));
        target = path.with_file_name(named);
    }
    let _ = fs::copy(path, target);
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
    fn corrupt_settings_is_backed_up_before_default() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("settings.json");
        std::fs::write(&path, "{ \"manual_projects\": [\"/a\"] oops").unwrap();
        assert_eq!(load_settings(&path), Settings::default());
        let bak = tmp.path().join("settings.json.bak");
        assert_eq!(
            std::fs::read_to_string(&bak).unwrap(),
            "{ \"manual_projects\": [\"/a\"] oops"
        );
        // A second corrupt load keeps the first backup and adds another copy.
        std::fs::write(&path, "garbage2").unwrap();
        load_settings(&path);
        assert!(std::fs::read_to_string(&bak).unwrap().contains("oops"));
        let n = std::fs::read_dir(tmp.path()).unwrap().count();
        assert_eq!(n, 3);
    }

    #[test]
    fn save_then_load_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("settings.json");
        let s = Settings {
            language: Some("zh-CN".to_string()),
            theme: Some("dark".to_string()),
            accent: Some("blue".to_string()),
            manual_projects: vec!["/a".to_string()],
            claude_quota_enabled: true,
            codex_quota_enabled: true,
        };
        save_settings(&path, &s).unwrap();
        let loaded = load_settings(&path);
        assert_eq!(loaded.manual_projects, vec!["/a".to_string()]);
        assert_eq!(loaded, s);
        // The temporary file must not be left behind after the atomic rename.
        assert!(!tmp.path().join("settings.json.tmp").exists());
    }
}
