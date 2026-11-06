//! `README.md` of the sync repository (MemDockHub): what the repository is,
//! a link to the MemDock project, and the project's own README.

/// The MemDock project on GitHub.
pub const PROJECT_URL: &str = "https://github.com/LingyiChen-AI/MemDock";
/// First line of a README MemDock wrote; only such READMEs (or GitHub's
/// auto-generated one) are replaced, so a README the user wrote is kept.
pub const MARKER: &str = "<!-- memdock:hub-readme -->";

const PROJECT_README: &str = include_str!("../../../README.md");

pub fn hub_readme() -> String {
    format!(
        "{MARKER}\n# MemDockHub\n\n\
这是 [MemDock]({PROJECT_URL}) 的记忆同步仓库：你在各台电脑上的 Claude Code / Codex 记忆和 Skill 安装清单都同步在这里。请保持私有。\n\n\
This is the sync repository of [MemDock]({PROJECT_URL}): your Claude Code / Codex memories and the skills install list, shared across your computers. Keep it private.\n\n\
- `global/`：全局记忆 / global memories\n\
- `projects/`：按项目（git 远程地址或目录名）存放的项目记忆 / project memories\n\
- `skills.json`：Skill 安装清单 / skills install list\n\
- `memdock_settings.json`：MemDock 偏好设置（不含本机路径）/ MemDock preferences (no local paths)\n\
- `memdock.json`：仓库标记，请勿删除 / repository marker, do not delete\n\n\
项目地址 / Project: {PROJECT_URL}\n\n\
---\n\n\
{}",
        // Same bytes whatever line endings the checkout used (keeps the blob id stable).
        PROJECT_README.trim_start_matches('\u{feff}').replace("\r\n", "\n")
    )
}

/// Whether an existing root README may be replaced: absent, written by
/// MemDock before, or GitHub's auto-generated two-liner.
pub fn replaceable(existing: Option<&[u8]>) -> bool {
    let Some(bytes) = existing else { return true };
    let text = String::from_utf8_lossy(bytes);
    let t = text.trim();
    t.starts_with(MARKER) || t.lines().filter(|l| !l.trim().is_empty()).count() <= 2 && t.starts_with("# ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readme_links_the_project_and_embeds_its_readme() {
        let r = hub_readme();
        assert!(r.starts_with(MARKER));
        assert!(r.contains(PROJECT_URL));
        assert!(r.contains("# MemDock\n"));
    }

    #[test]
    fn only_generated_or_auto_init_readmes_are_replaced() {
        assert!(replaceable(None));
        assert!(replaceable(Some(b"# MemDockHub\nMemDock memory sync")));
        assert!(replaceable(Some(hub_readme().as_bytes())));
        assert!(!replaceable(Some("# My notes\n\nline one\nline two\nline three\n".as_bytes())));
    }
}
