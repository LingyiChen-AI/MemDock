use std::path::{Path, PathBuf};

use serde::Serialize;

/// Characters that often trail an `@path` token in prose and are not part of the path.
const TRAILING_PUNCT: &[char] = &[',', '.', ';', ':', ')', '」', '）', '。', '，'];

/// One `@path` import found in a memory file.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ImportRef {
    pub raw: String,
    pub resolved: String,
    pub exists: bool,
    /// 1-based line number.
    pub line: u32,
}

/// Estimates the token count: ASCII characters / 4 rounded up, plus one per non-ASCII character.
pub fn estimate_tokens(text: &str) -> u32 {
    let mut ascii: u32 = 0;
    let mut other: u32 = 0;
    for c in text.chars() {
        if c.is_ascii() {
            ascii += 1;
        } else {
            other += 1;
        }
    }
    ascii.div_ceil(4) + other
}

/// Finds `@path` imports outside fenced code blocks and inline backtick spans.
pub fn find_imports(text: &str, base_dir: &Path, home: &Path) -> Vec<ImportRef> {
    let mut imports = Vec::new();
    let mut in_fence = false;
    for (index, line) in text.lines().enumerate() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        let visible = strip_inline_code(line);
        for raw in import_tokens(&visible) {
            let resolved = resolve(&raw, base_dir, home);
            imports.push(ImportRef {
                exists: resolved.exists(),
                resolved: resolved.to_string_lossy().into_owned(),
                raw,
                line: index as u32 + 1,
            });
        }
    }
    imports
}

/// Removes inline backtick spans (including the backticks themselves) from a line.
fn strip_inline_code(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut in_code = false;
    for c in line.chars() {
        if c == '`' {
            in_code = !in_code;
        } else if !in_code {
            out.push(c);
        }
    }
    out
}

/// Extracts candidate import paths: `@` at line start or after whitespace, up to the next whitespace.
fn import_tokens(line: &str) -> Vec<String> {
    let chars: Vec<char> = line.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '@' && (i == 0 || chars[i - 1].is_whitespace()) {
            let start = i + 1;
            let mut end = start;
            while end < chars.len() && !chars[end].is_whitespace() {
                end += 1;
            }
            let token: String = chars[start..end].iter().collect();
            let token = token.trim_end_matches(TRAILING_PUNCT);
            if qualifies(token) {
                tokens.push(token.to_string());
            }
            i = end;
        } else {
            i += 1;
        }
    }
    tokens
}

fn qualifies(token: &str) -> bool {
    token.starts_with('/')
        || token.starts_with("./")
        || token.starts_with("../")
        || token.starts_with("~/")
        || is_windows_drive(token)
        || token.contains(".md")
}

fn is_windows_drive(token: &str) -> bool {
    let b = token.as_bytes();
    b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'/' || b[2] == b'\\')
}

fn resolve(raw: &str, base_dir: &Path, home: &Path) -> PathBuf {
    if let Some(rest) = raw.strip_prefix("~/") {
        home.join(rest)
    } else if raw.starts_with('/') || is_windows_drive(raw) || Path::new(raw).is_absolute() {
        PathBuf::from(raw)
    } else {
        base_dir.join(raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn estimate_ascii() {
        assert_eq!(estimate_tokens("abcdefgh"), 2);
    }

    #[test]
    fn estimate_cjk() {
        assert_eq!(estimate_tokens("中文"), 2);
    }

    #[test]
    fn finds_relative_and_home_imports() {
        let base = tempdir().unwrap();
        let home = tempdir().unwrap();
        fs::create_dir_all(base.path().join("docs")).unwrap();
        fs::write(base.path().join("docs").join("a.md"), "x").unwrap();

        let imports = find_imports("see @./docs/a.md and @~/x.md", base.path(), home.path());

        assert_eq!(imports.len(), 2);
        assert_eq!(imports[0].raw, "./docs/a.md");
        assert_eq!(
            imports[0].resolved,
            base.path().join("./docs/a.md").to_string_lossy().into_owned()
        );
        assert!(imports[0].exists);
        assert_eq!(imports[0].line, 1);
        assert_eq!(imports[1].raw, "~/x.md");
        assert_eq!(
            imports[1].resolved,
            home.path().join("x.md").to_string_lossy().into_owned()
        );
        assert!(!imports[1].exists);
    }

    #[test]
    fn ignores_imports_in_code_blocks() {
        let text = "```\n@./a.md\n```\n`@./b.md`\n@./c.md\n";
        let imports = find_imports(text, Path::new("/base"), Path::new("/home/u"));

        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "./c.md");
        assert_eq!(imports[0].line, 5);
    }

    #[test]
    fn ignores_email_like_text() {
        let imports = find_imports("mail a@b.com", Path::new("/base"), Path::new("/home/u"));
        assert!(imports.is_empty());
    }

    #[test]
    fn strips_trailing_punctuation() {
        let imports = find_imports(
            "load (@./docs/a.md), then @./b.md.",
            Path::new("/base"),
            Path::new("/home/u"),
        );
        // "(@" is not preceded by whitespace, so only the second token qualifies.
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw, "./b.md");
    }
}
