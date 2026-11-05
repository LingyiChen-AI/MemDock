use std::path::{Path, PathBuf};

/// Locations of the local Claude Code and Codex configuration directories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentPaths {
    pub claude_home: PathBuf,
    pub codex_home: PathBuf,
}

impl AgentPaths {
    /// Builds the default layout under `home`: `home/.claude` and `home/.codex`.
    pub fn from_home(home: &Path) -> Self {
        Self {
            claude_home: home.join(".claude"),
            codex_home: home.join(".codex"),
        }
    }

    /// Detects the paths for the current user. `CLAUDE_CONFIG_DIR` and `CODEX_HOME`
    /// override the defaults when set to a non-empty value.
    pub fn detect() -> Self {
        let home = dirs::home_dir().unwrap_or_default();
        let mut paths = Self::from_home(&home);
        if let Some(dir) = non_empty_env("CLAUDE_CONFIG_DIR") {
            paths.claude_home = dir;
        }
        if let Some(dir) = non_empty_env("CODEX_HOME") {
            paths.codex_home = dir;
        }
        paths
    }
}

fn non_empty_env(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Serializes tests that mutate process environment variables.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn from_home_builds_expected_dirs() {
        let home = Path::new("/home/tester");
        let p = AgentPaths::from_home(home);
        assert_eq!(p.claude_home, home.join(".claude"));
        assert_eq!(p.codex_home, home.join(".codex"));
    }

    #[test]
    fn detect_respects_env_overrides() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let prev_codex = std::env::var_os("CODEX_HOME");
        let prev_claude = std::env::var_os("CLAUDE_CONFIG_DIR");
        unsafe {
            std::env::set_var("CODEX_HOME", tmp.path());
            std::env::remove_var("CLAUDE_CONFIG_DIR");
        }
        let p = AgentPaths::detect();
        unsafe {
            match prev_codex {
                Some(v) => std::env::set_var("CODEX_HOME", v),
                None => std::env::remove_var("CODEX_HOME"),
            }
            match prev_claude {
                Some(v) => std::env::set_var("CLAUDE_CONFIG_DIR", v),
                None => std::env::remove_var("CLAUDE_CONFIG_DIR"),
            }
        }
        assert_eq!(p.codex_home, tmp.path());
    }
}
