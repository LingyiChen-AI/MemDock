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
        Err(_) => return Frontmatter::default(),
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
