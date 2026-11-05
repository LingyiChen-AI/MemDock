use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::skills::frontmatter::parse_frontmatter;
use crate::skills::git::COMPLETE_MARKER;
use crate::skills::source::{normalize_url, parse_url};

/// Cache entries older than this are removed after install / update.
pub const CACHE_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ParsedSource {
    pub url: String,
    pub clone_url: String,
    pub git_ref: Option<String>,
    pub skill_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RepoSkill {
    pub skill_path: String,
    pub name: String,
    pub description: String,
}

fn is_name_part(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Git ref names that are safe to hand to git as a positional argument.
fn valid_ref(r: &str) -> bool {
    !r.is_empty()
        && !r.starts_with('-')
        && !r.contains("..")
        && !r.contains("@{")
        && !r
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || "~^:?*[\\".contains(c))
}

fn valid_subpath(p: &str) -> bool {
    !p.chars().any(|c| c.is_control() || c == '\\') && p.split('/').all(|s| s != "..")
}

/// Drops credentials from a URL but keeps the user's scheme. http(s): the whole
/// userinfo goes; `ssh://`: the password goes, the user stays; scp-style is kept.
fn clone_url_of(s: &str) -> String {
    let s = s.trim().trim_end_matches('/');
    let Some((scheme, rest)) = s.split_once("://") else {
        return s.to_string();
    };
    let (auth, path) = match rest.split_once('/') {
        Some((a, p)) => (a, format!("/{p}")),
        None => (rest, String::new()),
    };
    let auth = match auth.rsplit_once('@') {
        None => auth.to_string(),
        Some((user, host)) if scheme.eq_ignore_ascii_case("ssh") => {
            format!("{}@{host}", user.split(':').next().unwrap_or(""))
        }
        Some((_, host)) => host.to_string(),
    };
    format!("{scheme}://{auth}{path}")
}

/// Accepts `owner/repo`, GitHub repo / `tree/<ref>/<path>` URLs, scp-style ssh
/// addresses and other `https://` git URLs. `url` is the normalized identity,
/// `clone_url` keeps the user's scheme (credentials stripped) for network calls.
pub fn parse_github_input(input: &str) -> Result<ParsedSource, String> {
    let s = input.trim();
    let err = || format!("无法识别的来源: {input}");
    if s.is_empty() || s.contains(char::is_whitespace) || s.starts_with('-') {
        return Err(err());
    }
    let plain = |url: String, clone_url: String| ParsedSource {
        url,
        clone_url,
        git_ref: None,
        skill_path: None,
    };
    // owner/repo shorthand.
    if !s.contains("://") && !s.contains(':') && !s.contains('@') {
        let parts: Vec<&str> = s.trim_matches('/').split('/').collect();
        if parts.len() == 2 && parts.iter().all(|p| is_name_part(p)) {
            let repo = parts[1].strip_suffix(".git").unwrap_or(parts[1]);
            let url = format!("https://github.com/{}/{repo}", parts[0]);
            return Ok(plain(url.clone(), url));
        }
        return Err(err());
    }
    let is_http = s.starts_with("https://") || s.starts_with("http://");
    let is_ssh = s.starts_with("ssh://") || (!s.contains("://") && s.contains('@'));
    if !is_http && !is_ssh {
        return Err(err());
    }
    let (host, path) = parse_url(s).ok_or_else(err)?;
    if is_http && host == "github.com" {
        let segs: Vec<&str> = path.split('/').collect();
        if segs.len() >= 4 && segs[2] == "tree" {
            let skill_path = segs[4..].join("/");
            if !valid_ref(segs[3]) || !valid_subpath(&skill_path) {
                return Err(err());
            }
            let url = format!("https://{host}/{}/{}", segs[0], segs[1]);
            return Ok(ParsedSource {
                clone_url: url.clone(),
                url,
                git_ref: Some(segs[3].to_string()),
                skill_path: Some(skill_path).filter(|p| !p.is_empty()),
            });
        }
        if segs.len() != 2 {
            return Err(err());
        }
    }
    Ok(plain(normalize_url(s), clone_url_of(s)))
}

/// A cache directory counts only when a clone finished into it.
pub fn is_complete_cache(dir: &Path) -> bool {
    dir.join(COMPLETE_MARKER).is_file()
}

/// Finds every `SKILL.md` in a cloned repo (skipping `.git` / `node_modules`),
/// sorted by path. `skill_path` is the skill directory relative to the repo
/// root, `/`-separated (`""` for the root).
pub fn list_repo_skills(repo_dir: &Path) -> Vec<RepoSkill> {
    let walker = walkdir::WalkDir::new(repo_dir)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            let n = e.file_name().to_string_lossy();
            !(e.file_type().is_dir() && (n == ".git" || n == "node_modules"))
        });
    let mut out = Vec::new();
    for e in walker.flatten() {
        if !e.file_type().is_file() || e.file_name() != "SKILL.md" {
            continue;
        }
        let Some(dir) = e.path().parent() else {
            continue;
        };
        let rel = dir
            .strip_prefix(repo_dir)
            .map(|r| {
                r.components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/")
            })
            .unwrap_or_default();
        let text = fs::read_to_string(e.path()).unwrap_or_default();
        let fm = parse_frontmatter(&text);
        let dir_name = dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        out.push(RepoSkill {
            skill_path: rel,
            name: fm.name.unwrap_or(dir_name),
            description: fm.description.unwrap_or_default(),
        });
    }
    out.sort_by(|a, b| a.skill_path.cmp(&b.skill_path));
    out
}

/// `<cache_root>/<sha256(normalized url)[..16]>-<commit[..8]>`.
pub fn cache_dir_for(cache_root: &Path, url: &str, commit: &str) -> PathBuf {
    let digest = Sha256::digest(normalize_url(url).as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    let short: String = commit.chars().take(8).collect();
    cache_root.join(format!("{}-{short}", &hex[..16]))
}

/// Removes cache entries whose mtime is older than `older_than`.
pub fn cleanup_cache(cache_root: &Path, older_than: Duration) {
    let Ok(rd) = fs::read_dir(cache_root) else {
        return;
    };
    let now = SystemTime::now();
    for e in rd.flatten() {
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| now.duration_since(t).ok())
            .is_some_and(|age| age >= older_than);
        if old {
            let p = e.path();
            if p.is_dir() {
                let _ = fs::remove_dir_all(&p);
            } else {
                let _ = fs::remove_file(&p);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(input: &str) -> ParsedSource {
        parse_github_input(input).unwrap()
    }

    #[test]
    fn parse_owner_repo() {
        let s = p("alchaincyf/huashu-motion");
        assert_eq!(s.url, "https://github.com/alchaincyf/huashu-motion");
        assert_eq!(s.git_ref, None);
        assert_eq!(s.skill_path, None);
        assert_eq!(p(" a/b ").url, "https://github.com/a/b");
        assert_eq!(
            p("https://github.com/a/b.git").url,
            "https://github.com/a/b"
        );
        assert_eq!(p("https://github.com/a/b/").url, "https://github.com/a/b");
        assert_eq!(
            p("https://gitlab.com/g/r.git").url,
            "https://gitlab.com/g/r"
        );
    }

    #[test]
    fn parse_tree_url_with_ref_and_path() {
        let s = p("https://github.com/o/r/tree/main/skills/demo");
        assert_eq!(s.url, "https://github.com/o/r");
        assert_eq!(s.git_ref.as_deref(), Some("main"));
        assert_eq!(s.skill_path.as_deref(), Some("skills/demo"));
        let s = p("https://github.com/o/r/tree/v1.2");
        assert_eq!(s.git_ref.as_deref(), Some("v1.2"));
        assert_eq!(s.skill_path, None);
    }

    #[test]
    fn parse_ssh_url() {
        assert_eq!(p("git@github.com:o/r.git").url, "https://github.com/o/r");
        assert_eq!(
            p("ssh://git@github.com/o/r.git").url,
            "https://github.com/o/r"
        );
    }

    #[test]
    fn clone_url_keeps_scheme_and_strips_credentials() {
        let s = p("git@github.com:o/r.git");
        assert_eq!(s.url, "https://github.com/o/r");
        assert_eq!(s.clone_url, "git@github.com:o/r.git");
        let s = p("ssh://git@github.com:22/o/r.git");
        assert_eq!(s.url, "https://github.com/o/r");
        assert_eq!(s.clone_url, "ssh://git@github.com:22/o/r.git");
        let s = p("ssh://git:secret@github.com/o/r.git");
        assert_eq!(s.clone_url, "ssh://git@github.com/o/r.git");
        let s = p("https://user:tok@github.com/o/r.git");
        assert_eq!(s.url, "https://github.com/o/r");
        assert_eq!(s.clone_url, "https://github.com/o/r.git");
        let s = p("https://github.com/o/r/tree/main/x");
        assert_eq!(s.clone_url, "https://github.com/o/r");
        assert_eq!(p("o/r").clone_url, "https://github.com/o/r");
        assert_eq!(
            p("https://gitlab.com/g/r.git").clone_url,
            "https://gitlab.com/g/r.git"
        );
    }

    #[test]
    fn parse_rejects_injected_refs_and_paths() {
        for bad in [
            "https://github.com/o/r/tree/--upload-pack=x/skills",
            "https://github.com/o/r/tree/a~b/skills",
            "https://github.com/o/r/tree/main/../etc",
            "--upload-pack=x",
            "-o/r",
        ] {
            assert!(parse_github_input(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn incomplete_cache_dir_is_absent() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!is_complete_cache(tmp.path()));
        fs::write(tmp.path().join(COMPLETE_MARKER), "x").unwrap();
        assert!(is_complete_cache(tmp.path()));
    }

    #[test]
    fn parse_rejects_garbage() {
        for bad in [
            "",
            "   ",
            "justone",
            "a/b/c",
            "hello world",
            "ftp://github.com/a/b",
            "https://",
            "https://github.com/onlyowner",
            "git@github.com",
            "C:/x/y",
        ] {
            assert!(parse_github_input(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn lists_multiple_skills_in_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let r = tmp.path();
        let w = |rel: &str, body: &str| {
            let f = r.join(rel);
            fs::create_dir_all(f.parent().unwrap()).unwrap();
            fs::write(f, body).unwrap();
        };
        w(
            "skills/b/SKILL.md",
            "---\nname: bee\ndescription: B skill\n---\n",
        );
        w("skills/a/SKILL.md", "no frontmatter");
        w("SKILL.md", "---\nname: root\n---\n");
        w("node_modules/x/SKILL.md", "---\nname: x\n---\n");
        w(".git/y/SKILL.md", "---\nname: y\n---\n");
        let l = list_repo_skills(r);
        let paths: Vec<_> = l.iter().map(|s| s.skill_path.as_str()).collect();
        assert_eq!(paths, ["", "skills/a", "skills/b"]);
        assert_eq!(l[0].name, "root");
        assert_eq!(l[1].name, "a");
        assert_eq!(l[2].name, "bee");
        assert_eq!(l[2].description, "B skill");
    }

    #[test]
    fn cache_dir_is_stable_per_url_and_commit() {
        let root = Path::new("/c");
        let a = cache_dir_for(root, "https://github.com/o/r.git", "0123456789abcdef");
        let b = cache_dir_for(root, "git@github.com:o/r", "0123456789abcdef");
        assert_eq!(a, b);
        let name = a.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(name.len(), 16 + 1 + 8);
        assert!(name.ends_with("-01234567"));
        assert_ne!(a, cache_dir_for(root, "https://github.com/o/r", "ffffffff"));
    }

    #[test]
    fn cleanup_removes_only_old_entries() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("e1/sub")).unwrap();
        fs::create_dir_all(tmp.path().join("e2")).unwrap();
        cleanup_cache(tmp.path(), CACHE_MAX_AGE);
        assert!(tmp.path().join("e1").exists() && tmp.path().join("e2").exists());
        cleanup_cache(tmp.path(), Duration::ZERO);
        assert!(!tmp.path().join("e1").exists() && !tmp.path().join("e2").exists());
        cleanup_cache(&tmp.path().join("missing"), Duration::ZERO);
    }
}
