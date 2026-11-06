//! Parses the GitHub link the user pasted.

/// What the link points at.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LinkKind {
    /// The whole repository (default branch).
    Repo,
    /// `tree/<ref>/<dir>`: a branch / tag / commit, optionally a subdirectory.
    Tree,
    /// `blob/<ref>/<path>`: a single file.
    Blob,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct GhLink {
    pub owner: String,
    pub repo: String,
    pub kind: LinkKind,
    /// Segments after `tree/` or `blob/`: the ref followed by the path. Where
    /// the ref ends is unknown (branch names may contain `/`); the fetch step
    /// tries the shortest prefix first.
    pub tail: Vec<String>,
}

impl GhLink {
    pub fn full_name(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }

    pub fn repo_url(&self) -> String {
        format!("https://github.com/{}/{}", self.owner, self.repo)
    }

    /// `(ref, path)` splits of `tail`, shortest ref first. At most `max` splits.
    pub fn ref_candidates(&self, max: usize) -> Vec<(String, String)> {
        let n = self.tail.len();
        (1..=n.min(max))
            .map(|k| (self.tail[..k].join("/"), self.tail[k..].join("/")))
            .collect()
    }
}

fn is_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 100
        && s != "."
        && s != ".."
        && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

fn is_segment(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && !s.chars().any(|c| c.is_control() || matches!(c, '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
}

/// Minimal percent-decoding (`%20` and friends); invalid escapes stay as-is.
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = std::str::from_utf8(&b[i + 1..i + 3]).ok();
            if let Some(v) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Accepts `owner/repo`, `https://github.com/owner/repo[.git]`,
/// `.../tree/<ref>[/<dir>]` and `.../blob/<ref>/<path>`. Query strings and
/// fragments are ignored. Error text is a short English reason.
pub fn parse_link(input: &str) -> Result<GhLink, String> {
    let s = input.trim();
    let s = s.split(['?', '#']).next().unwrap_or("").trim();
    if s.is_empty() || s.contains(char::is_whitespace) {
        return Err("empty or invalid link".into());
    }
    let path = if let Some(rest) = s
        .strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"))
    {
        let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
        let host = host.to_ascii_lowercase();
        if host != "github.com" && host != "www.github.com" {
            return Err("only github.com links are supported".into());
        }
        path.to_string()
    } else if let Some(rest) = s.strip_prefix("github.com/") {
        rest.to_string()
    } else if !s.contains("://") && !s.contains(':') && !s.contains('@') {
        s.to_string()
    } else {
        return Err("only github.com links are supported".into());
    };
    let segs: Vec<String> = path
        .trim_matches('/')
        .split('/')
        .filter(|p| !p.is_empty())
        .map(percent_decode)
        .collect();
    if segs.len() < 2 {
        return Err("the link must name a repository (owner/repo)".into());
    }
    let owner = segs[0].clone();
    let repo = segs[1].strip_suffix(".git").unwrap_or(&segs[1]).to_string();
    if !is_name(&owner) || !is_name(&repo) {
        return Err("invalid owner or repository name".into());
    }
    let (kind, tail) = match segs.get(2).map(String::as_str) {
        None => (LinkKind::Repo, Vec::new()),
        Some("tree") => (LinkKind::Tree, segs[3..].to_vec()),
        Some("blob") => (LinkKind::Blob, segs[3..].to_vec()),
        Some(_) => return Err("unsupported GitHub link (use the repository, tree or blob URL)".into()),
    };
    if kind == LinkKind::Tree && tail.is_empty() {
        return Err("the tree link has no branch".into());
    }
    if kind == LinkKind::Blob && tail.len() < 2 {
        return Err("the blob link has no file path".into());
    }
    if tail.iter().any(|t| !is_segment(t)) {
        return Err("invalid path in the link".into());
    }
    Ok(GhLink { owner, repo, kind, tail })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_forms() {
        for s in [
            "octo/demo",
            "https://github.com/octo/demo",
            "https://github.com/octo/demo.git",
            "https://www.github.com/octo/demo/",
            "github.com/octo/demo?tab=readme#top",
        ] {
            let l = parse_link(s).unwrap_or_else(|e| panic!("{s}: {e}"));
            assert_eq!((l.owner.as_str(), l.repo.as_str(), l.kind), ("octo", "demo", LinkKind::Repo), "{s}");
            assert!(l.tail.is_empty());
        }
    }

    #[test]
    fn tree_and_blob() {
        let l = parse_link("https://github.com/o/r/tree/feature/x/docs/mem").unwrap();
        assert_eq!(l.kind, LinkKind::Tree);
        assert_eq!(l.tail, vec!["feature", "x", "docs", "mem"]);
        let c = l.ref_candidates(10);
        assert_eq!(c[0], ("feature".to_string(), "x/docs/mem".to_string()));
        assert_eq!(c[1], ("feature/x".to_string(), "docs/mem".to_string()));
        assert_eq!(c.len(), 4);
        let l = parse_link("https://github.com/o/r/blob/main/My%20Notes/CLAUDE.md").unwrap();
        assert_eq!(l.kind, LinkKind::Blob);
        assert_eq!(l.tail, vec!["main", "My Notes", "CLAUDE.md"]);
        let l = parse_link("https://github.com/o/r/tree/main").unwrap();
        assert_eq!(l.ref_candidates(10), vec![("main".to_string(), String::new())]);
    }

    #[test]
    fn rejects_bad_links() {
        for s in [
            "",
            "octo",
            "https://gitlab.com/o/r",
            "git@github.com:o/r.git",
            "https://github.com/o/r/issues/1",
            "https://github.com/o/r/blob/main",
            "https://github.com/o/r/tree",
            "https://github.com/o/r/tree/main/../x",
            "https://github.com/../r",
            "a b/c",
        ] {
            assert!(parse_link(s).is_err(), "{s}");
        }
    }
}
