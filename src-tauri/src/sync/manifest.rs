//! Skills install manifest (`skills.json`): syncs how skills are installed, not their files.
//!
//! # File format (`format: 1`)
//!
//! ```text
//! {
//!   "description": "MemDock skill install manifest ...",  // optional, export files only
//!   "format": 1,
//!   "skills": [
//!     {
//!       "name": "huashu-motion",               // SKILL.md name (directory name fallback)
//!       "url": "https://github.com/o/r",       // normalized repo URL; "" when local_only
//!       "git_ref": null,                       // branch / tag tracked; null = remote HEAD
//!       "commit": "c0ffee...",                 // commit installed, when known
//!       "skill_path": "skills/huashu-motion",  // folder inside the repo; "" = repo root
//!       "tool": "claude",                      // "claude" | "codex"
//!       "scope": { "type": "global" },         // or { "type": "project", "key": "github.com/o/p" }
//!       "installed_on": ["<machine id>"],      // machines that have it now
//!       "removed_on": ["<machine id>"],        // machines that uninstalled it
//!       "local_only": true                     // optional: no known source, copy by hand
//!     }
//!   ]
//! }
//! ```
//!
//! Every non-read-only skill found by the scan is recorded: registry installs,
//! git checkouts (origin remote), symlinks into a git repo, `npx skills`
//! (skill-lock) installs, and skills without any source (`local_only`: only
//! counted, never installable). Identity: `url + skill_path + tool + scope`,
//! or `name + tool + scope` for local-only entries.
//!
//! Compatibility: `local_only` / `description` are optional fields and older
//! readers ignore unknown fields (no `deny_unknown_fields`), so the format
//! number stays 1.

use std::collections::BTreeSet;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::model::Tool;
use crate::skills::model::{SkillEntry, SkillScope, SkillSource};
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
    /// No known source: recorded for the inventory, must be copied by hand.
    #[serde(default, skip_serializing_if = "is_false")]
    pub local_only: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl ManifestSkill {
    /// Local-only entries (flag set, or no url at all) cannot be installed.
    pub fn is_local_only(&self) -> bool {
        self.local_only || self.url.trim().is_empty()
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct Manifest {
    /// Human-readable note (export files); ignored when reading.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub format: u32,
    pub skills: Vec<ManifestSkill>,
}

impl Default for Manifest {
    fn default() -> Self {
        Manifest {
            description: None,
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
            if s.url.is_empty() {
                s.local_only = true;
            }
        }
        Ok(m)
    }
}

/// `(url, skill_path, tool, scope, local name)`; the name is only set for
/// local-only entries, which have no url to tell them apart.
type Identity = (String, String, u8, ManifestScope, String);

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
    if s.is_local_only() {
        return (
            String::new(),
            String::new(),
            tool_rank(s.tool),
            s.scope.clone(),
            s.name.clone(),
        );
    }
    (
        norm_url(&s.url),
        norm_path(&s.skill_path),
        tool_rank(s.tool),
        s.scope.clone(),
        String::new(),
    )
}

fn sort_skills(v: &mut [ManifestSkill]) {
    v.sort_by_key(identity);
}

fn make(name: &str, src: Option<&SkillSource>, tool: Tool, scope: ManifestScope) -> ManifestSkill {
    let src = src.filter(|s| !s.url.trim().is_empty());
    ManifestSkill {
        name: name.to_string(),
        url: src.map(|s| norm_url(&s.url)).unwrap_or_default(),
        git_ref: src.and_then(|s| s.git_ref.clone()),
        commit: src.and_then(|s| s.commit.clone()),
        skill_path: src.map(|s| norm_path(&s.skill_path)).unwrap_or_default(),
        tool,
        scope,
        installed_on: BTreeSet::new(),
        removed_on: BTreeSet::new(),
        local_only: src.is_none(),
    }
}

/// This machine's skills for the manifest, from a scan whose entries already
/// carry their inferred source (`scan_with_sources` / `infer_source`).
/// Read-only skills (plugins, Codex built-ins, synced) are skipped. Skills
/// with a source url become installable entries; the rest are recorded as
/// `local_only`. Project skills need their project's key in `projects`
/// (`(project_id, key)`), otherwise they are skipped. The first entry wins
/// for a repeated identity.
pub fn local_entries(
    entries: &[SkillEntry],
    projects: &[(String, Option<String>)],
) -> Vec<ManifestSkill> {
    let mut out: Vec<ManifestSkill> = Vec::new();
    for e in entries.iter().filter(|e| !e.readonly) {
        let scope = match &e.scope {
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
        out.push(make(&e.name, e.source.as_ref(), e.tool, scope));
    }
    // stable sort + dedup keeps the first entry for a repeated identity
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
        .filter(|(key, _)| valid_key(key) && base.join(key).exists())
        .collect()
}

/// A lock key must be a single plain directory name.
fn valid_key(key: &str) -> bool {
    if key.is_empty() || key == "." || key == ".." || key.contains('/') || key.contains('\\') {
        return false;
    }
    let mut comps = Path::new(key).components();
    matches!(
        (comps.next(), comps.next()),
        (Some(std::path::Component::Normal(_)), None)
    )
}

/// Skill-lock entries for a sync. Missing lock file -> Ok(empty); unreadable or
/// unparseable -> Err (caller must skip the manifest merge for this sync, since
/// "empty" would be read as "nothing installed here").
pub fn lock_entries_for_sync(home: &Path) -> Result<Vec<(String, SkillSource)>, String> {
    let file = home.join(".agents").join(".skill-lock.json");
    if !file.exists() {
        return Ok(Vec::new());
    }
    let text = std::fs::read_to_string(&file).map_err(|e| format!("skill-lock: {e}"))?;
    let v: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("skill-lock: {e}"))?;
    if !v.get("skills").is_some_and(|s| s.is_object()) {
        return Err("skill-lock: missing skills object".to_string());
    }
    Ok(installed_lock_entries(
        home,
        crate::skills::source::read_skill_lock(home),
    ))
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
            item.local_only = l.is_local_only();
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
        description: None,
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

/// How this machine's skills relate to another manifest (the synced
/// `skills.json` or an imported file).
#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct Comparison {
    /// In `remote`, not here, still installed somewhere else (or the file does
    /// not say), and not removed on this machine on purpose.
    pub missing_here: Vec<ManifestSkill>,
    /// Here, not in `remote` (local entries).
    pub only_here: Vec<ManifestSkill>,
    /// Here and in `remote` (local entries).
    pub in_both: Vec<ManifestSkill>,
}

/// Compares by identity (url + skill_path + tool + scope; name + tool + scope
/// for local-only entries).
pub fn compare(local: &[ManifestSkill], remote: &Manifest, machine: &str) -> Comparison {
    let remote_ids: BTreeSet<Identity> = remote.skills.iter().map(identity).collect();
    let mut seen: BTreeSet<Identity> = BTreeSet::new();
    let missing_here = remote
        .skills
        .iter()
        .filter(|s| {
            let nobody = s.installed_on.is_empty() && !s.removed_on.is_empty();
            let only_me = !s.installed_on.is_empty() && s.installed_on.iter().all(|o| o == machine);
            !has_local(local, s)
                && !s.removed_on.contains(machine)
                && !nobody
                && !only_me
                && seen.insert(identity(s))
        })
        .cloned()
        .collect();
    let (in_both, only_here) = local
        .iter()
        .cloned()
        .partition(|l| remote_ids.contains(&identity(l)));
    Comparison {
        missing_here,
        only_here,
        in_both,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::model::{SkillKind, SourceOrigin};

    fn src(url: &str, path: &str) -> SkillSource {
        SkillSource {
            url: url.into(),
            git_ref: None,
            commit: Some("abc".into()),
            skill_path: path.into(),
            origin: SourceOrigin::Registry,
        }
    }

    fn scanned(name: &str, tool: Tool, scope: SkillScope, source: Option<SkillSource>) -> SkillEntry {
        SkillEntry {
            id: format!("{name}-{tool:?}-{scope:?}"),
            name: name.into(),
            description: String::new(),
            tool,
            scope,
            kind: SkillKind::User,
            dir: format!("/x/{name}"),
            is_symlink: false,
            symlink_target: None,
            readonly: false,
            plugin: None,
            source,
            file_count: 1,
            size_bytes: 1,
            modified_at: None,
        }
    }

    fn proj(id: &str) -> SkillScope {
        SkillScope::Project {
            project_id: id.into(),
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
            local_only: false,
        }
    }

    fn local_sk(name: &str, on: &[&str]) -> ManifestSkill {
        let mut s = sk(name, on, &[]);
        s.url = String::new();
        s.local_only = true;
        s
    }

    fn man(skills: Vec<ManifestSkill>) -> Manifest {
        Manifest {
            description: None,
            format: 1,
            skills,
        }
    }

    #[test]
    fn local_entries_cover_every_user_skill() {
        let mut plugin = scanned("plug", Tool::Claude, SkillScope::Global, None);
        plugin.readonly = true;
        plugin.kind = SkillKind::Plugin;
        let mut git_src = src("git@github.com:o/r.git", "/skills/g/");
        git_src.origin = SourceOrigin::GitDir;
        let mut lock_src = src("https://github.com/o/l", "s/l");
        lock_src.origin = SourceOrigin::SkillLock;
        let entries = vec![
            scanned("a", Tool::Claude, SkillScope::Global, Some(src("https://github.com/o/a", ""))),
            scanned("a", Tool::Claude, proj("p1"), Some(src("https://github.com/o/a", ""))),
            scanned("a", Tool::Claude, proj("p2"), Some(src("https://github.com/o/a", ""))),
            scanned("g", Tool::Claude, SkillScope::Global, Some(git_src)),
            scanned("l", Tool::Codex, SkillScope::Global, Some(lock_src)),
            scanned("mine", Tool::Codex, SkillScope::Global, None),
            scanned("empty", Tool::Claude, SkillScope::Global, Some(src("", ""))),
            plugin,
        ];
        let projects = vec![
            ("p1".to_string(), Some("github.com/o/p".to_string())),
            ("p2".to_string(), None),
        ];
        let got = local_entries(&entries, &projects);
        assert_eq!(got.len(), 6, "{got:?}");
        assert!(got.iter().all(|s| s.installed_on.is_empty()));
        assert!(got
            .iter()
            .any(|s| s.name == "a" && s.scope == ManifestScope::Global));
        assert!(got.iter().any(
            |s| matches!(&s.scope, ManifestScope::Project { key } if key == "github.com/o/p")
        ));
        let g = got.iter().find(|s| s.name == "g").unwrap();
        assert_eq!(g.url, "https://github.com/o/r");
        assert_eq!(g.skill_path, "skills/g");
        assert!(!g.local_only);
        let l = got.iter().find(|s| s.name == "l").unwrap();
        assert_eq!((l.tool, &l.scope), (Tool::Codex, &ManifestScope::Global));
        let mine = got.iter().find(|s| s.name == "mine").unwrap();
        assert!(mine.local_only && mine.url.is_empty() && mine.commit.is_none());
        assert!(got.iter().find(|s| s.name == "empty").unwrap().local_only);
        assert!(!got.iter().any(|s| s.name == "plug"));
    }

    #[test]
    fn local_only_identity_uses_name() {
        let entries = vec![
            scanned("x", Tool::Claude, SkillScope::Global, None),
            scanned("y", Tool::Claude, SkillScope::Global, None),
            scanned("x", Tool::Claude, SkillScope::Global, None),
        ];
        let got = local_entries(&entries, &[]);
        assert_eq!(got.len(), 2);
        // merge keeps two distinct local-only entries
        let merged = merge(&Manifest::default(), &got, "A");
        assert_eq!(merged.skills.len(), 2);
        let json = merged.to_json().unwrap();
        assert!(json.contains("\"local_only\": true"));
        let back = Manifest::from_json(&json).unwrap();
        assert!(back.skills.iter().all(|s| s.local_only));
    }

    #[test]
    fn old_files_and_unknown_fields_read() {
        // no local_only -> false; unknown fields ignored; empty url -> local_only
        let t = r#"{"format":1,"extra":1,"skills":[{"name":"a","url":"https://github.com/o/a","skill_path":"","tool":"claude","scope":{"type":"global"},"future":true},{"name":"b","url":"","skill_path":"","tool":"codex","scope":{"type":"global"}}]}"#;
        let m = Manifest::from_json(t).unwrap();
        assert!(!m.skills[0].local_only);
        assert!(m.skills[1].local_only);
        // false is not written, so unchanged entries keep their old bytes
        assert!(!man(vec![sk("a", &[], &[])]).to_json().unwrap().contains("local_only"));
    }

    #[test]
    fn local_entries_from_git_clone() {
        use crate::skills::git::SystemGit;
        use crate::skills::registry::Registry;
        use crate::skills::source::infer_source;
        let git = SystemGit::new();
        if !crate::skills::git::GitRunner::available(&git) {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("cloned");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SKILL.md"), "---\nname: cloned\n---\n").unwrap();
        let run = |args: &[&str]| {
            let ok = std::process::Command::new("git")
                .args(args)
                .current_dir(&dir)
                .output()
                .unwrap()
                .status
                .success();
            assert!(ok, "git {args:?}");
        };
        run(&["init", "-q"]);
        run(&["remote", "add", "origin", "https://github.com/acme/cloned.git"]);
        let local = tmp.path().join("plain");
        std::fs::create_dir_all(&local).unwrap();
        let mut a = scanned("cloned", Tool::Claude, SkillScope::Global, None);
        a.dir = dir.to_string_lossy().into_owned();
        let mut b = scanned("plain", Tool::Claude, SkillScope::Global, None);
        b.dir = local.to_string_lossy().into_owned();
        let mut entries = vec![a, b];
        for e in entries.iter_mut() {
            e.source = infer_source(e, &Registry::default(), &[], &git);
        }
        let got = local_entries(&entries, &[]);
        assert_eq!(got.len(), 2);
        let c = got.iter().find(|s| s.name == "cloned").unwrap();
        assert_eq!(c.url, "https://github.com/acme/cloned");
        assert_eq!(c.skill_path, "");
        assert!(!c.local_only);
        let p = got.iter().find(|s| s.name == "plain").unwrap();
        assert!(p.local_only);
        assert!(p.url.is_empty());
    }

    #[test]
    fn compare_groups() {
        let mut remote = man(vec![
            sk("other", &["B"], &[]),
            sk("both", &["A", "B"], &[]),
            sk("removed_here", &["B"], &["A"]),
            sk("nobody", &[], &["B"]),
            sk("was_mine", &["A"], &[]),
            sk("from_file", &[], &[]),
            local_sk("copy", &["B"]),
        ]);
        remote.skills.push(sk("other", &["C"], &[]));
        let local = vec![sk("both", &[], &[]), sk("new", &[], &[]), local_sk("mine", &[])];
        let c = compare(&local, &remote, "A");
        let names = |v: &[ManifestSkill]| v.iter().map(|s| s.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(&c.missing_here), vec!["other", "from_file", "copy"]);
        assert!(c.missing_here.iter().find(|s| s.name == "copy").unwrap().local_only);
        assert_eq!(names(&c.in_both), vec!["both"]);
        assert_eq!(names(&c.only_here), vec!["new", "mine"]);
        // a local-only skill with the same name/tool/scope counts as present
        let c2 = compare(&[local_sk("copy", &[])], &remote, "A");
        assert!(!names(&c2.missing_here).contains(&"copy".to_string()));
        assert_eq!(names(&c2.in_both), vec!["copy"]);
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

    #[test]
    fn installed_lock_entries_rejects_unsafe_keys() {
        let root = std::env::temp_dir().join(format!("md-lock2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let home = root.join("home");
        std::fs::create_dir_all(home.join(".agents").join("skills").join("a").join("b")).unwrap();
        std::fs::create_dir_all(home.join(".agents").join("x")).unwrap();
        let abs = root.to_string_lossy().into_owned();
        let s = src("https://github.com/o/h", "");
        let lock = vec![
            ("../x".to_string(), s.clone()),
            (abs, s.clone()),
            ("a/b".to_string(), s.clone()),
            ("..".to_string(), s),
        ];
        let got = installed_lock_entries(&home, lock);
        let _ = std::fs::remove_dir_all(&root);
        assert!(got.is_empty());
    }

    #[test]
    fn lock_entries_for_sync_three_cases() {
        let home = std::env::temp_dir().join(format!("md-lock3-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join(".agents").join("skills").join("k")).unwrap();
        assert_eq!(lock_entries_for_sync(&home).unwrap().len(), 0);
        let lock = home.join(".agents").join(".skill-lock.json");
        std::fs::write(&lock, "{corrupt").unwrap();
        assert!(lock_entries_for_sync(&home).is_err());
        std::fs::write(
            &lock,
            r#"{"version":3,"skills":{"k":{"sourceUrl":"https://github.com/o/k","skillPath":"k/SKILL.md"},"gone":{"sourceUrl":"https://github.com/o/g"}}}"#,
        )
        .unwrap();
        let got = lock_entries_for_sync(&home).unwrap();
        let _ = std::fs::remove_dir_all(&home);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, "k");
    }
}
