//! Shared secret rules: redaction (used for model prompts) and scanning
//! (used to block syncing files that contain secrets).

use std::sync::OnceLock;

use regex::Regex;

struct Rules {
    private_key: Regex,
    private_key_open: Regex,
    password: Regex,
    password_zh: Regex,
    /// (rule name, regex)
    tokens: Vec<(&'static str, Regex)>,
}

fn rules() -> &'static Rules {
    static R: OnceLock<Rules> = OnceLock::new();
    R.get_or_init(|| Rules {
        private_key: Regex::new(
            r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z0-9 ]*PRIVATE KEY-----",
        )
        .unwrap(),
        private_key_open: Regex::new(r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----[\s\S]*").unwrap(),
        password: Regex::new(concat!(
            r#"(?i)(["']?[A-Za-z0-9_\-]*(?:pass(?:word)?|passwd|pwd|secret|api[_-]?key|access[_-]?key|token)[A-Za-z0-9_\-]*["']?\s*[:=]\s*)"#,
            r#"("[^"\n]*"|'[^'\n]*'|[^\s"']\S*)"#
        ))
        .unwrap(),
        password_zh: Regex::new(concat!(
            "((?:密码|口令|秘钥|密钥)\\s*(?:[:：=]|是|为)\\s*)",
            "(\"[^\"\\n]*\"|'[^'\\n]*'|“[^”\\n]*”|[^\\s，。,;；]+)"
        ))
        .unwrap(),
        tokens: [
            ("anthropic_key", r"\bsk-ant-[A-Za-z0-9_\-]{16,}"),
            ("openai_key", r"\bsk-[A-Za-z0-9_\-]{16,}"),
            ("github_token", r"\bgh[pousr]_[A-Za-z0-9]{20,}"),
            ("github_token", r"\bgithub_pat_[A-Za-z0-9_]{20,}"),
            ("slack_token", r"\bxox[bpas]-[A-Za-z0-9\-]{10,}"),
            ("aws_access_key", r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b"),
            ("bearer_token", r"(?i)\bbearer\s+[A-Za-z0-9._\-]{20,}"),
        ]
        .iter()
        .map(|(n, p)| (*n, Regex::new(p).unwrap()))
        .collect(),
    })
}

/// Replaces API keys, tokens, private keys and `password=...` values by
/// `[REDACTED]`. Token-like matches keep their first 4 characters.
pub fn redact(text: &str) -> String {
    let r = rules();
    let mut out = r.private_key.replace_all(text, "[REDACTED]").into_owned();
    out = r.private_key_open.replace_all(&out, "[REDACTED]").into_owned();
    out = r.password.replace_all(&out, "${1}[REDACTED]").into_owned();
    out = r.password_zh.replace_all(&out, "${1}[REDACTED]").into_owned();
    for (_, re) in &r.tokens {
        out = re
            .replace_all(&out, |c: &regex::Captures| {
                let head: String = c[0].chars().take(4).collect();
                format!("{head}…[REDACTED]")
            })
            .into_owned();
    }
    out
}

/// One secret finding. Never carries the matched text.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SecretHit {
    /// 1-based line number.
    pub line: usize,
    pub rule: &'static str,
}

/// Scans `text` line by line with the same rules as `redact`; each rule is
/// reported at most once per line.
pub fn scan(text: &str) -> Vec<SecretHit> {
    let r = rules();
    let mut hits = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let n = i + 1;
        let mut push = |rule: &'static str| {
            if !hits.contains(&SecretHit { line: n, rule }) {
                hits.push(SecretHit { line: n, rule });
            }
        };
        // Only the opening marker is on a single line; the body lines follow it.
        if r.private_key_open.is_match(line) {
            push("private_key");
        }
        let value_is_real = |re: &Regex| {
            re.captures_iter(line)
                .any(|c| c.get(2).is_some_and(|v| !v.as_str().contains("[REDACTED]")))
        };
        if value_is_real(&r.password) {
            push("password");
        }
        if value_is_real(&r.password_zh) {
            push("password_zh");
        }
        for (name, re) in &r.tokens {
            let found = re.find_iter(line).any(|m| {
                // `sk-ant-` keys are reported as anthropic_key only.
                *name != "openai_key" || !m.as_str().starts_with("sk-ant-")
            });
            if found {
                push(name);
            }
        }
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_reports_line_and_rule() {
        let hits = scan("a\nkey sk-ant-api03-xxxxxxxxxxxxxxxxxxxx\n");
        assert_eq!(hits, vec![SecretHit { line: 2, rule: "anthropic_key" }]);
        assert_eq!(scan("password: hunter2"), vec![SecretHit { line: 1, rule: "password" }]);
        assert!(scan("just some ordinary text\nnothing here").is_empty());
    }

    #[test]
    fn scan_names_each_rule() {
        let cases = [
            ("-----BEGIN RSA PRIVATE KEY-----", "private_key"),
            ("密码：abc123", "password_zh"),
            ("sk-abcdefghijklmnopqrst", "openai_key"),
            ("ghp_abcdefghijklmnopqrstuvwxyz0123", "github_token"),
            ("xoxb-1234567890-abcdef", "slack_token"),
            ("AKIAABCDEFGHIJKLMNOP", "aws_access_key"),
        ];
        for (text, rule) in cases {
            let hits = scan(text);
            assert!(hits.iter().any(|h| h.rule == rule), "{text} -> {hits:?}");
        }
        // an already-redacted value is not a finding
        assert!(scan("password: [REDACTED]").is_empty());
    }

    #[test]
    fn redact_still_masks() {
        assert_eq!(redact("password: hunter2"), "password: [REDACTED]");
        assert!(redact("sk-ant-api03-abcdefghijklmnop").contains("sk-a…[REDACTED]"));
    }
}
