use yaml_rust2::YamlLoader;

/// The fields Memdock reads from a SKILL.md frontmatter block.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Frontmatter {
    pub name: Option<String>,
    pub description: Option<String>,
}

/// Parses the YAML between the leading `---` and the next `---`.
/// Returns empty fields when there is no (valid) frontmatter.
pub fn parse_frontmatter(text: &str) -> Frontmatter {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = text.lines();
    match lines.next() {
        Some(first) if first.trim_end() == "---" => {}
        _ => return Frontmatter::default(),
    }
    let mut yaml = String::new();
    let mut closed = false;
    for line in lines {
        if line.trim_end() == "---" {
            closed = true;
            break;
        }
        yaml.push_str(line);
        yaml.push('\n');
    }
    if !closed {
        return Frontmatter::default();
    }
    let docs = match YamlLoader::load_from_str(&yaml) {
        Ok(d) => d,
        Err(_) => return fallback_parse(&yaml),
    };
    let Some(doc) = docs.first() else {
        return Frontmatter::default();
    };
    let field = |key: &str| {
        doc[key]
            .as_str()
            .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|s| !s.is_empty())
    };
    Frontmatter {
        name: field("name"),
        description: field("description"),
    }
}

fn squash(s: &str) -> Option<String> {
    Some(s.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|s| !s.is_empty())
}

fn unquote(v: &str) -> &str {
    let v = v.trim();
    for q in ['"', '\''] {
        if v.len() >= 2 && v.starts_with(q) && v.ends_with(q) {
            return &v[1..v.len() - 1];
        }
    }
    v
}

/// Line-based reader used when strict YAML rejects the block (for example an
/// unquoted value containing ": "). Reads top-level `name` and `description`.
fn fallback_parse(yaml: &str) -> Frontmatter {
    let lines: Vec<&str> = yaml.lines().collect();
    let mut fm = Frontmatter::default();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        i += 1;
        let Some((key, rest)) = line.split_once(':') else {
            continue;
        };
        if key != "name" && key != "description" {
            continue;
        }
        let rest = rest.trim();
        let value = if matches!(rest.chars().next(), Some('>' | '|')) {
            let mut parts = Vec::new();
            while i < lines.len()
                && (lines[i].starts_with(' ')
                    || lines[i].starts_with('\t')
                    || lines[i].trim().is_empty())
            {
                parts.push(lines[i].trim());
                i += 1;
            }
            squash(&parts.join(" "))
        } else {
            squash(unquote(rest))
        };
        if key == "name" {
            fm.name = value;
        } else {
            fm.description = value;
        }
    }
    fm
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_folded_description() {
        let text = "---\nname: fireworks-tech-graph\ndescription: >-\n  Create precise SVG\n  supported semantic SVGs\n---\n# Body\n";
        let fm = parse_frontmatter(text);
        assert_eq!(fm.name.as_deref(), Some("fireworks-tech-graph"));
        assert_eq!(
            fm.description.as_deref(),
            Some("Create precise SVG supported semantic SVGs")
        );
    }

    #[test]
    fn frontmatter_literal_and_quoted() {
        let text = "---\nname: \"quoted\"\ndescription: |\n  line one\n  line two\n---\n";
        let fm = parse_frontmatter(text);
        assert_eq!(fm.name.as_deref(), Some("quoted"));
        assert_eq!(fm.description.as_deref(), Some("line one line two"));
    }

    #[test]
    fn frontmatter_unquoted_colon_falls_back() {
        let text = "---\nname: my-skill\ndescription: Use when X: do Y\n---\n";
        let fm = parse_frontmatter(text);
        assert_eq!(fm.name.as_deref(), Some("my-skill"));
        assert_eq!(fm.description.as_deref(), Some("Use when X: do Y"));
    }

    #[test]
    fn frontmatter_fallback_block_and_quotes() {
        let text =
            "---\nname: \"q: x\"\ndescription: >-\n  first: part\n  second\nother: a: b\n---\n";
        let fm = parse_frontmatter(text);
        assert_eq!(fm.name.as_deref(), Some("q: x"));
        assert_eq!(fm.description.as_deref(), Some("first: part second"));
    }

    #[test]
    fn frontmatter_name_only() {
        let fm = parse_frontmatter("---\nname: solo\n---\n");
        assert_eq!(fm.name.as_deref(), Some("solo"));
        assert_eq!(fm.description, None);
    }

    #[test]
    fn frontmatter_bom_and_crlf() {
        let text = "\u{feff}---\r\nname: win\r\ndescription: Use when X: do Y\r\n---\r\nbody\r\n";
        let fm = parse_frontmatter(text);
        assert_eq!(fm.name.as_deref(), Some("win"));
        assert_eq!(fm.description.as_deref(), Some("Use when X: do Y"));
    }

    #[test]
    fn frontmatter_missing_returns_none() {
        assert_eq!(
            parse_frontmatter("# Just a title\n"),
            Frontmatter::default()
        );
        assert_eq!(
            parse_frontmatter("---\nname: x\nno closing"),
            Frontmatter::default()
        );
    }
}
