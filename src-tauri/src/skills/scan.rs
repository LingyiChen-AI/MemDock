use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use walkdir::WalkDir;

use crate::model::{Project, Tool};
use crate::paths::AgentPaths;
use crate::projects::normalize_path;
use crate::skills::frontmatter::parse_frontmatter;
use crate::skills::model::{SkillEntry, SkillKind, SkillScope};

const SKILL_FILE: &str = "SKILL.md";

/// Where a group of skills lives and how to label them.
struct Ctx<'a> {
    tool: Tool,
    scope: &'a SkillScope,
    kind: SkillKind,
    plugin: Option<&'a str>,
}

/// Finds every skill directory (a directory holding a `SKILL.md`) in the global
/// and per-project locations of Claude Code and Codex. The same physical
/// directory reached through two locations is listed once per location.
pub fn scan_skills(paths: &AgentPaths, home: &Path, projects: &[Project]) -> Vec<SkillEntry> {
    let mut out = Vec::new();
    let global = SkillScope::Global;
    let scope_of = |p: &Project| SkillScope::Project {
        project_id: p.id.clone(),
    };

    // Global Claude (skipping `synced`).
    let ctx = Ctx {
        tool: Tool::Claude,
        scope: &global,
        kind: SkillKind::User,
        plugin: None,
    };
    scan_children(
        &paths.claude_home.join("skills"),
        &["synced"],
        &ctx,
        &mut out,
    );

    // Global Codex (skipping `.system`) and the shared ~/.agents.
    let ctx = Ctx {
        tool: Tool::Codex,
        scope: &global,
        kind: SkillKind::User,
        plugin: None,
    };
    scan_children(
        &paths.codex_home.join("skills"),
        &[".system"],
        &ctx,
        &mut out,
    );
    scan_children(&home.join(".agents").join("skills"), &[], &ctx, &mut out);

    // Projects.
    for p in projects {
        let scope = scope_of(p);
        let root = Path::new(&p.path);
        let claude = Ctx {
            tool: Tool::Claude,
            scope: &scope,
            kind: SkillKind::User,
            plugin: None,
        };
        scan_children(&root.join(".claude").join("skills"), &[], &claude, &mut out);
        let codex = Ctx {
            tool: Tool::Codex,
            scope: &scope,
            kind: SkillKind::User,
            plugin: None,
        };
        scan_children(&root.join(".agents").join("skills"), &[], &codex, &mut out);
        scan_children(&root.join(".codex").join("skills"), &[], &codex, &mut out);
    }

    // Plugins.
    for (key, install) in plugin_installs(&paths.claude_home) {
        let ctx = Ctx {
            tool: Tool::Claude,
            scope: &global,
            kind: SkillKind::Plugin,
            plugin: Some(&key),
        };
        scan_children(&install.join("skills"), &[], &ctx, &mut out);
    }

    // Codex built-in.
    let ctx = Ctx {
        tool: Tool::Codex,
        scope: &global,
        kind: SkillKind::CodexSystem,
        plugin: None,
    };
    scan_children(
        &paths.codex_home.join("skills").join(".system"),
        &[],
        &ctx,
        &mut out,
    );

    // claude.ai synced (any depth).
    let ctx = Ctx {
        tool: Tool::Claude,
        scope: &global,
        kind: SkillKind::ClaudeSynced,
        plugin: None,
    };
    let synced = paths.claude_home.join("skills").join("synced");
    for entry in WalkDir::new(&synced)
        .follow_links(true)
        .sort_by_file_name()
        .into_iter()
        .flatten()
    {
        if entry.file_type().is_file() && entry.file_name() == SKILL_FILE {
            if let Some(dir) = entry.path().parent() {
                push_skill(dir, &ctx, &mut out);
            }
        }
    }
    out
}

/// Reads `installed_plugins.json` and returns `(plugin key, installPath)` pairs.
/// A missing or malformed file yields nothing.
fn plugin_installs(claude_home: &Path) -> Vec<(String, PathBuf)> {
    let Ok(text) = fs::read_to_string(claude_home.join("plugins").join("installed_plugins.json"))
    else {
        return Vec::new();
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    let Some(plugins) = json.get("plugins").and_then(|v| v.as_object()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (key, installs) in plugins {
        let Some(arr) = installs.as_array() else {
            continue;
        };
        for inst in arr {
            if let Some(p) = inst.get("installPath").and_then(|v| v.as_str()) {
                out.push((key.clone(), PathBuf::from(p)));
            }
        }
    }
    out
}

/// Adds every immediate subdirectory of `root` that contains a `SKILL.md`.
fn scan_children(root: &Path, skip: &[&str], ctx: &Ctx, out: &mut Vec<SkillEntry>) {
    let Ok(rd) = fs::read_dir(root) else { return };
    let mut dirs: Vec<PathBuf> = rd
        .flatten()
        .filter(|e| !skip.iter().any(|s| e.file_name() == *s))
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    for dir in dirs {
        push_skill(&dir, ctx, out);
    }
}

fn push_skill(dir: &Path, ctx: &Ctx, out: &mut Vec<SkillEntry>) {
    let md = dir.join(SKILL_FILE);
    if !md.is_file() {
        return;
    }
    let text = fs::read_to_string(&md).unwrap_or_default();
    let fm = parse_frontmatter(&text);
    let dir_name = dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let link_meta = fs::symlink_metadata(dir).ok();
    let is_symlink = link_meta.is_some_and(|m| m.file_type().is_symlink());
    let symlink_target = if is_symlink {
        fs::read_link(dir)
            .ok()
            .map(|t| t.to_string_lossy().to_string())
    } else {
        None
    };
    let (file_count, size_bytes) = dir_stats(dir);
    let modified_at = fs::metadata(&md)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64);
    let dir_str = dir.to_string_lossy().to_string();
    out.push(SkillEntry {
        id: normalize_path(&dir_str),
        name: fm.name.unwrap_or(dir_name),
        description: fm.description.unwrap_or_default(),
        tool: ctx.tool,
        scope: ctx.scope.clone(),
        kind: ctx.kind,
        dir: dir_str,
        is_symlink,
        symlink_target,
        readonly: ctx.kind != SkillKind::User,
        plugin: ctx.plugin.map(str::to_string),
        source: None,
        file_count,
        size_bytes,
        modified_at,
    });
}

/// Files and bytes under `dir` (following symlinks), excluding `.git`.
fn dir_stats(dir: &Path) -> (u32, u64) {
    let mut count = 0u32;
    let mut size = 0u64;
    let walker = WalkDir::new(dir)
        .follow_links(true)
        .into_iter()
        .filter_entry(|e| e.depth() == 0 || e.file_name() != ".git");
    for entry in walker.flatten() {
        if entry.file_type().is_file() {
            count += 1;
            size += entry.metadata().map(|m| m.len()).unwrap_or(0);
        }
    }
    (count, size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ProjectSource;
    use tempfile::TempDir;

    fn write(path: &Path, content: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn skill(dir: &Path, name: &str, desc: &str) {
        write(
            &dir.join("SKILL.md"),
            &format!("---\nname: {name}\ndescription: {desc}\n---\nbody\n"),
        );
    }

    fn project(dir: &Path) -> Project {
        let path = dir.to_string_lossy().to_string();
        Project {
            id: normalize_path(&path),
            path,
            name: "proj".into(),
            exists: true,
            sources: vec![ProjectSource::Manual],
        }
    }

    fn setup() -> (TempDir, AgentPaths) {
        let tmp = TempDir::new().unwrap();
        let paths = AgentPaths::from_home(tmp.path());
        (tmp, paths)
    }

    #[test]
    fn scans_claude_global_and_project() {
        let (tmp, paths) = setup();
        skill(&paths.claude_home.join("skills/alpha"), "alpha", "A skill");
        write(
            &paths.claude_home.join("skills/beta/SKILL.md"),
            "no frontmatter",
        );
        let proj_dir = tmp.path().join("work/p1");
        skill(
            &proj_dir.join(".claude/skills/gamma"),
            "gamma",
            "Project skill",
        );
        let p = project(&proj_dir);

        let out = scan_skills(&paths, tmp.path(), std::slice::from_ref(&p));
        let alpha = out.iter().find(|s| s.name == "alpha").unwrap();
        assert_eq!(alpha.description, "A skill");
        assert_eq!(alpha.tool, Tool::Claude);
        assert_eq!(alpha.scope, SkillScope::Global);
        assert_eq!(alpha.kind, SkillKind::User);
        assert!(!alpha.readonly);
        assert!(!alpha.is_symlink);
        assert_eq!(alpha.file_count, 1);
        assert!(alpha.size_bytes > 0);
        assert!(alpha.modified_at.is_some());
        assert_eq!(alpha.id, normalize_path(&alpha.dir));
        assert!(alpha.source.is_none());
        // Missing frontmatter falls back to the directory name.
        assert!(
            out.iter()
                .any(|s| s.name == "beta" && s.description.is_empty())
        );
        let gamma = out.iter().find(|s| s.name == "gamma").unwrap();
        assert_eq!(
            gamma.scope,
            SkillScope::Project {
                project_id: p.id.clone()
            }
        );
        assert_eq!(gamma.tool, Tool::Claude);
    }

    #[test]
    fn skips_dirs_without_skill_md() {
        let (tmp, paths) = setup();
        write(&paths.claude_home.join("skills/empty/readme.md"), "x");
        write(&paths.claude_home.join("skills/loose-file.md"), "x");
        skill(&paths.claude_home.join("skills/real"), "real", "d");
        let out = scan_skills(&paths, tmp.path(), &[]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "real");
    }

    #[test]
    fn counts_files_recursively_excluding_git() {
        let (tmp, paths) = setup();
        let dir = paths.claude_home.join("skills/multi");
        skill(&dir, "multi", "d");
        write(&dir.join("scripts/run.sh"), "abc");
        write(&dir.join(".git/config"), "ignored");
        let out = scan_skills(&paths, tmp.path(), &[]);
        assert_eq!(out[0].file_count, 2);
    }

    #[test]
    fn plugin_skills_readonly_from_installed_plugins() {
        let (tmp, paths) = setup();
        let install = tmp.path().join("cache/superpowers/7.0.0");
        skill(
            &install.join("skills/brainstorming"),
            "brainstorming",
            "Plan first",
        );
        let json = serde_json::json!({
            "version": 2,
            "plugins": {
                "superpowers@claude-plugins-official": [
                    { "scope": "user", "installPath": install.to_string_lossy(), "version": "7.0.0" }
                ]
            }
        });
        write(
            &paths.claude_home.join("plugins/installed_plugins.json"),
            &json.to_string(),
        );
        let out = scan_skills(&paths, tmp.path(), &[]);
        assert_eq!(out.len(), 1);
        let s = &out[0];
        assert_eq!(s.kind, SkillKind::Plugin);
        assert!(s.readonly);
        assert_eq!(
            s.plugin.as_deref(),
            Some("superpowers@claude-plugins-official")
        );
        assert_eq!(s.tool, Tool::Claude);
        assert_eq!(s.scope, SkillScope::Global);
    }

    #[test]
    fn plugin_manifest_malformed_is_ignored() {
        let (tmp, paths) = setup();
        skill(&paths.claude_home.join("skills/a"), "a", "d");
        write(
            &paths.claude_home.join("plugins/installed_plugins.json"),
            "{not json",
        );
        let out = scan_skills(&paths, tmp.path(), &[]);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn codex_system_and_agents_dirs() {
        let (tmp, paths) = setup();
        skill(
            &paths.codex_home.join("skills/.system/imagegen"),
            "imagegen",
            "Gen",
        );
        skill(&paths.codex_home.join("skills/mine"), "mine", "Mine");
        skill(
            &tmp.path().join(".agents/skills/shared"),
            "shared",
            "Shared",
        );
        let proj_dir = tmp.path().join("work/p2");
        skill(&proj_dir.join(".agents/skills/pa"), "pa", "d");
        skill(&proj_dir.join(".codex/skills/pc"), "pc", "d");
        let p = project(&proj_dir);

        let out = scan_skills(&paths, tmp.path(), std::slice::from_ref(&p));
        let sys = out.iter().find(|s| s.name == "imagegen").unwrap();
        assert_eq!(sys.kind, SkillKind::CodexSystem);
        assert!(sys.readonly);
        assert_eq!(sys.tool, Tool::Codex);
        let mine = out.iter().find(|s| s.name == "mine").unwrap();
        assert_eq!(mine.kind, SkillKind::User);
        assert!(!mine.readonly);
        assert_eq!(mine.tool, Tool::Codex);
        let shared = out.iter().find(|s| s.name == "shared").unwrap();
        assert_eq!(shared.tool, Tool::Codex);
        assert_eq!(shared.scope, SkillScope::Global);
        for n in ["pa", "pc"] {
            let s = out.iter().find(|s| s.name == n).unwrap();
            assert_eq!(
                s.scope,
                SkillScope::Project {
                    project_id: p.id.clone()
                }
            );
            assert_eq!(s.tool, Tool::Codex);
        }
        // `.system` itself is not a skill.
        assert_eq!(out.len(), 5);
    }

    #[test]
    fn claude_synced_is_readonly() {
        let (tmp, paths) = setup();
        skill(
            &paths.claude_home.join("skills/synced/uuid-1/inner"),
            "synced-one",
            "d",
        );
        skill(&paths.claude_home.join("skills/normal"), "normal", "d");
        let out = scan_skills(&paths, tmp.path(), &[]);
        assert_eq!(out.len(), 2);
        let s = out.iter().find(|s| s.name == "synced-one").unwrap();
        assert_eq!(s.kind, SkillKind::ClaudeSynced);
        assert!(s.readonly);
        assert_eq!(s.tool, Tool::Claude);
    }

    #[test]
    fn same_dir_in_two_locations_listed_twice() {
        let (tmp, paths) = setup();
        skill(&paths.claude_home.join("skills/dup"), "dup", "d");
        skill(&paths.codex_home.join("skills/dup"), "dup", "d");
        let out = scan_skills(&paths, tmp.path(), &[]);
        assert_eq!(out.iter().filter(|s| s.name == "dup").count(), 2);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_skill_flagged_with_target() {
        let (tmp, paths) = setup();
        let real = tmp.path().join("share/ego-skills");
        skill(&real, "ego", "d");
        write(&real.join("extra.txt"), "12345");
        fs::create_dir_all(paths.claude_home.join("skills")).unwrap();
        std::os::unix::fs::symlink(&real, paths.claude_home.join("skills/ego-browser")).unwrap();
        let out = scan_skills(&paths, tmp.path(), &[]);
        let s = out.iter().find(|s| s.name == "ego").unwrap();
        assert!(s.is_symlink);
        assert_eq!(
            s.symlink_target.as_deref(),
            Some(real.to_string_lossy().as_ref())
        );
        assert_eq!(s.file_count, 2);
    }
}
