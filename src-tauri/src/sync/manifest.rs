//! Skills install manifest (`skills.json`): syncs how skills are installed, not their files.

use std::collections::BTreeSet;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::model::Tool;
use crate::skills::model::{SkillScope, SkillSource};
use crate::skills::registry::Registry;
use crate::skills::source::normalize_url;

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ManifestScope {
    Global,
    Project { key: String },
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct ManifestSkill {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub git_ref: Option<String>,
    #[serde(default)]
    pub commit: Option<String>,
    pub skill_path: String,
    pub tool: Tool,
    pub scope: ManifestScope,
    #[serde(default)]
    pub installed_on: BTreeSet<String>,
    #[serde(default)]
    pub removed_on: BTreeSet<String>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct Manifest {
    pub format: u32,
    pub skills: Vec<ManifestSkill>,
}

impl Default for Manifest {
    fn default() -> Self {
        Manifest {
            format: 1,
            skills: Vec::new(),
        }
    }
}

impl Manifest {
    /// Pretty JSON (2 spaces) with a trailing newline.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        let mut s = serde_json::to_string_pretty(self)?;
        s.push('\n');
        Ok(s)
    }

    /// Err means abort the sync; never treat as an empty manifest. A missing file
    /// is the only case for Default. A newer `format`, unknown tool or scope
    /// values are errors. Urls and skill paths are normalized.
    pub fn from_json(text: &str) -> Result<Manifest, String> {
        let mut m: Manifest = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if m.format > 1 {
            return Err(format!(
                "skills.json format {} is newer than supported (1)",
                m.format
            ));
        }
        for s in &mut m.skills {
            s.url = norm_url(&s.url);
            s.skill_path = norm_path(&s.skill_path);
        }
        Ok(m)
    }
}

type Identity = (String, String, u8, ManifestScope);

fn tool_rank(t: Tool) -> u8 {
    match t {
        Tool::Claude => 0,
        Tool::Codex => 1,
    }
}

fn norm_url(u: &str) -> String {
    normalize_url(u)
}

fn norm_path(p: &str) -> String {
    p.trim_matches('/').to_string()
}

fn identity(s: &ManifestSkill) -> Identity {
    (
        norm_url(&s.url),
        norm_path(&s.skill_path),
        tool_rank(s.tool),
        s.scope.clone(),
    )
}

fn sort_skills(v: &mut [ManifestSkill]) {
    v.sort_by_key(identity);
}

fn make(name: &str, src: &SkillSource, tool: Tool, scope: ManifestScope) -> Option<ManifestSkill> {
    if src.url.trim().is_empty() {
        return None;
    }
    Some(ManifestSkill {
        name: name.to_string(),
        url: norm_url(&src.url),
        git_ref: src.git_ref.clone(),
        commit: src.commit.clone(),
        skill_path: norm_path(&src.skill_path),
        tool,
        scope,
        installed_on: BTreeSet::new(),
        removed_on: BTreeSet::new(),
    })
}

/// Installs on this machine that can be reproduced elsewhere. Plugin skills and
/// skills without a source never appear in the registry, so they are skipped;
/// project installs whose project has no key are skipped too.
///
/// Skill-lock entries (`npx skills`) cannot say which tool reads them; they are
/// treated as Codex global installs (`~/.agents/skills`). Registry entries win.
/// Callers pass `skill_lock` already filtered by `installed_lock_entries`, so
/// stale lock entries (directory gone) do not count as installed.
pub fn local_entries(
    registry: &Registry,
    skill_lock: &[(String, SkillSource)],
    projects: &[(String, Option<String>)],
) -> Vec<ManifestSkill> {
    let mut out: Vec<ManifestSkill> = Vec::new();
    for e in &registry.skills {
        for inst in &e.installs {
            let scope = match &inst.scope {
                SkillScope::Global => ManifestScope::Global,
                SkillScope::Project { project_id } => {
                    match projects
                        .iter()
                        .find(|(id, _)| id == project_id)
                        .and_then(|(_, k)| k.clone())
                    {
                        Some(key) => ManifestScope::Project { key },
                        None => continue,
                    }
                }
            };
            if let Some(m) = make(&e.name, &e.source, inst.tool, scope) {
                out.push(m);
            }
        }
    }
    for (name, src) in skill_lock {
        if let Some(m) = make(name, src, Tool::Codex, ManifestScope::Global) {
            out.push(m);
        }
    }
    // stable sort + dedup keeps the first (registry) entry for a repeated identity
    sort_skills(&mut out);
    out.dedup_by(|b, a| identity(a) == identity(b));
    out
}

/// Keeps only skill-lock entries whose directory still exists under
/// `<home>/.agents/skills/<key>`.
pub fn installed_lock_entries(
    home: &Path,
    lock: Vec<(String, SkillSource)>,
) -> Vec<(String, SkillSource)> {
    let base = home.join(".agents").join("skills");
    lock.into_iter()
        .filter(|(key, _)| !key.is_empty() && base.join(key).exists())
        .collect()
}

pub fn merge(base_remote: &Manifest, local: &[ManifestSkill], machine: &str) -> Manifest {
    let mut result: Vec<ManifestSkill> = Vec::new();
    for r in &base_remote.skills {
        let id = identity(r);
        let mut item = r.clone();
        if let Some(l) = local.iter().find(|l| identity(l) == id) {
            item.installed_on.insert(machine.to_string());
            item.removed_on.remove(machine);
            item.commit = l.commit.clone().or(item.commit.take());
            item.git_ref = l.git_ref.clone().or(item.git_ref.take());
        } else if item.installed_on.contains(machine) {
            item.installed_on.remove(machine);
            item.removed_on.insert(machine.to_string());
        }
        result.push(item);
    }
    let mut added: BTreeSet<Identity> = BTreeSet::new();
    for l in local {
        let id = identity(l);
        if base_remote.skills.iter().any(|r| identity(r) == id) || !added.insert(id) {
            continue;
        }
        let mut item = l.clone();
        item.installed_on = BTreeSet::from([machine.to_string()]);
        item.removed_on = BTreeSet::new();
        result.push(item);
    }
    sort_skills(&mut result);
    Manifest {
        format: 1,
        skills: result,
    }
}

fn has_local(local: &[ManifestSkill], s: &ManifestSkill) -> bool {
    let id = identity(s);
    local.iter().any(|l| identity(l) == id)
}

/// Skills other machines have that this one lacks (and did not remove on purpose).
pub fn missing_here(m: &Manifest, local: &[ManifestSkill], machine: &str) -> Vec<ManifestSkill> {
    m.skills
        .iter()
        .filter(|s| {
            !s.installed_on.is_empty()
                && !s.installed_on.contains(machine)
                && !has_local(local, s)
                && !s.removed_on.contains(machine)
        })
        .cloned()
        .collect()
}

/// Skills installed here that every other machine has removed.
pub fn removed_elsewhere(
    m: &Manifest,
    local: &[ManifestSkill],
    machine: &str,
) -> Vec<ManifestSkill> {
    m.skills
        .iter()
        .filter(|s| {
            has_local(local, s)
                && s.removed_on.iter().any(|o| o != machine)
                && s.installed_on.iter().all(|o| o == machine)
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::model::SourceOrigin;
    use crate::skills::registry::{InstallRecord, RegistryEntry};

    fn src(url: &str, path: &str) -> SkillSource {
        SkillSource {
            url: url.into(),
            git_ref: None,
            commit: Some("abc".into()),
            skill_path: path.into(),
            origin: SourceOrigin::Registry,
        }
    }

    fn entry(name: &str, source: SkillSource, installs: Vec<InstallRecord>) -> RegistryEntry {
        RegistryEntry {
            name: name.into(),
            source,
            clone_url: None,
            installs,
            content_hash: String::new(),
            installed_at: 0,
            updated_at: 0,
        }
    }

    fn inst(tool: Tool, scope: SkillScope) -> InstallRecord {
        InstallRecord {
            tool,
            scope,
            dir: "/x".into(),
        }
    }

    fn sk(name: &str, on: &[&str], off: &[&str]) -> ManifestSkill {
        ManifestSkill {
            name: name.into(),
            url: format!("https://github.com/o/{name}"),
            git_ref: None,
            commit: None,
            skill_path: String::new(),
            tool: Tool::Claude,
            scope: ManifestScope::Global,
            installed_on: on.iter().map(|s| s.to_string()).collect(),
            removed_on: off.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn man(skills: Vec<ManifestSkill>) -> Manifest {
        Manifest { format: 1, skills }
    }

    #[test]
    fn local_entries_skip_plugin_and_local() {
        let reg = Registry {
            version: 1,
            skills: vec![
                entry(
                    "a",
                    src("https://github.com/o/a", ""),
                    vec![
                        inst(Tool::Claude, SkillScope::Global),
                        inst(
                            Tool::Claude,
                            SkillScope::Project {
                                project_id: "p1".into(),
                            },
                        ),
                        inst(
                            Tool::Claude,
                            SkillScope::Project {
                                project_id: "p2".into(),
                            },
                        ),
                    ],
                ),
                entry(
                    "nosrc",
                    src("", ""),
                    vec![inst(Tool::Claude, SkillScope::Global)],
                ),
            ],
        };
        let lock = vec![("l".to_string(), src("https://github.com/o/l", "s/l"))];
        let projects = vec![
            ("p1".to_string(), Some("github.com/o/p".to_string())),
            ("p2".to_string(), None),
        ];
        let got = local_entries(&reg, &lock, &projects);
        assert_eq!(got.len(), 3);
        assert!(got.iter().all(|s| s.installed_on.is_empty()));
        assert!(got
            .iter()
            .any(|s| s.name == "a" && s.scope == ManifestScope::Global));
        assert!(got.iter().any(
            |s| matches!(&s.scope, ManifestScope::Project { key } if key == "github.com/o/p")
        ));
        let l = got.iter().find(|s| s.name == "l").unwrap();
        assert_eq!((l.tool, &l.scope), (Tool::Codex, &ManifestScope::Global));
        assert!(!got.iter().any(|s| s.name == "nosrc"));
    }

    #[test]
    fn merge_unions_two_machines() {
        let remote = man(vec![sk("x", &["A"], &[])]);
        let merged = merge(&remote, &[sk("y", &[], &[])], "B");
        assert_eq!(merged.skills.len(), 2);
        let x = merged.skills.iter().find(|s| s.name == "x").unwrap();
        let y = merged.skills.iter().find(|s| s.name == "y").unwrap();
        assert_eq!(x.installed_on, BTreeSet::from(["A".to_string()]));
        assert_eq!(y.installed_on, BTreeSet::from(["B".to_string()]));
    }

    #[test]
    fn uninstall_records_removal_not_delete() {
        let remote = man(vec![sk("x", &["A", "B"], &[])]);
        let merged = merge(&remote, &[], "A");
        assert_eq!(merged.skills.len(), 1);
        assert_eq!(
            merged.skills[0].installed_on,
            BTreeSet::from(["B".to_string()])
        );
        assert_eq!(
            merged.skills[0].removed_on,
            BTreeSet::from(["A".to_string()])
        );
        let again = merge(&merged, &[sk("x", &[], &[])], "A");
        assert!(again.skills[0].removed_on.is_empty());
        assert_eq!(again.skills[0].installed_on.len(), 2);
    }

    #[test]
    fn missing_here_lists_others() {
        let m = man(vec![
            sk("x", &["B"], &[]),
            sk("gone", &["B"], &["A"]),
            sk("have", &["B"], &[]),
            sk("nobody", &[], &["B"]),
        ]);
        let got = missing_here(&m, &[sk("have", &[], &[])], "A");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name, "x");
    }

    #[test]
    fn removed_elsewhere_prompts_only_when_last() {
        let m = man(vec![
            sk("last", &["A"], &["B"]),
            sk("others", &["A", "C"], &["B"]),
            sk("fine", &["A"], &[]),
        ]);
        let local = vec![
            sk("last", &[], &[]),
            sk("others", &[], &[]),
            sk("fine", &[], &[]),
        ];
        let got = removed_elsewhere(&m, &local, "A");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name, "last");
    }

    #[test]
    fn merge_is_deterministic() {
        let remote = man(vec![sk("z", &["B", "A"], &[]), sk("a", &["B"], &["C"])]);
        let local = vec![sk("m", &[], &[]), sk("z", &[], &[])];
        let s1 = merge(&remote, &local, "A").to_json().unwrap();
        let rev: Vec<_> = local.iter().rev().cloned().collect();
        let s2 = merge(&remote, &rev, "A").to_json().unwrap();
        assert_eq!(s1, s2);
        assert!(s1.ends_with("}\n"));
        assert!(s1.contains("\n  \"format\""));
    }

    #[test]
    fn from_json_rejects_bad_input() {
        assert!(Manifest::from_json("{not json").is_err());
        assert!(Manifest::from_json(r#"{"format":2,"skills":[]}"#).is_err());
        let bad_tool = r#"{"format":1,"skills":[{"name":"a","url":"u","skill_path":"","tool":"vim","scope":{"type":"global"}}]}"#;
        assert!(Manifest::from_json(bad_tool).is_err());
        let bad_scope = r#"{"format":1,"skills":[{"name":"a","url":"u","skill_path":"","tool":"claude","scope":{"type":"moon"}}]}"#;
        assert!(Manifest::from_json(bad_scope).is_err());
        assert!(Manifest::from_json(r#"{"format":1,"skills":[]}"#).is_ok());
    }

    #[test]
    fn from_json_normalizes_identity() {
        let t = r#"{"format":1,"skills":[{"name":"a","url":"https://github.com/O/a.git/","skill_path":"/s/a/","tool":"claude","scope":{"type":"global"}}]}"#;
        let m = Manifest::from_json(t).unwrap();
        assert_eq!(m.skills[0].skill_path, "s/a");
        assert!(!m.skills[0].url.ends_with(".git/"));
    }

    #[test]
    fn never_installed_here_is_not_marked_removed() {
        let remote = man(vec![sk("x", &["A"], &[])]);
        let merged = merge(&remote, &[], "B");
        assert!(merged.skills[0].removed_on.is_empty());
    }

    #[test]
    fn merge_keeps_remote_pin_when_local_has_none() {
        let mut r = sk("x", &["A"], &[]);
        r.commit = Some("pin".into());
        r.git_ref = Some("main".into());
        let merged = merge(&man(vec![r]), &[sk("x", &[], &[])], "B");
        assert_eq!(merged.skills[0].commit.as_deref(), Some("pin"));
        assert_eq!(merged.skills[0].git_ref.as_deref(), Some("main"));
    }

    #[test]
    fn empty_installed_on_edge_cases() {
        let m = man(vec![sk("x", &[], &["B"])]);
        assert!(missing_here(&m, &[], "A").is_empty());
        assert_eq!(removed_elsewhere(&m, &[sk("x", &[], &[])], "A").len(), 1);
    }

    #[test]
    fn missing_here_excludes_self_installed() {
        let m = man(vec![sk("x", &["A", "B"], &[])]);
        assert!(missing_here(&m, &[], "A").is_empty());
    }

    #[test]
    fn registry_wins_over_skill_lock() {
        let reg = Registry {
            version: 1,
            skills: vec![entry(
                "a",
                src("https://github.com/o/a", ""),
                vec![inst(Tool::Codex, SkillScope::Global)],
            )],
        };
        let mut lock_src = src("https://github.com/o/a", "");
        lock_src.origin = SourceOrigin::SkillLock;
        lock_src.commit = Some("lock".into());
        let got = local_entries(&reg, &[("a".into(), lock_src)], &[]);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].commit.as_deref(), Some("abc"));
    }

    #[test]
    fn installed_lock_entries_drops_missing_dirs() {
        let home = std::env::temp_dir().join(format!("md-lock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join(".agents").join("skills").join("here")).unwrap();
        let lock = vec![
            ("here".to_string(), src("https://github.com/o/h", "")),
            ("gone".to_string(), src("https://github.com/o/g", "")),
        ];
        let got = installed_lock_entries(&home, lock);
        let _ = std::fs::remove_dir_all(&home);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, "here");
    }
}
