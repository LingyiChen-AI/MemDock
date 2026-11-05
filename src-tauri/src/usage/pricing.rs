//! Bundled public price table (`resources/pricing.json`).
//!
//! Model names are normalized (lower case, provider prefixes, date / version /
//! context suffixes removed) and then matched exactly, so an unknown newer
//! model never silently inherits an older model's price.
use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::Deserialize;

use super::UsageEvent;

/// USD per million tokens.
#[derive(Deserialize, Debug, Clone, PartialEq)]
pub struct Price {
    pub input: f64,
    pub output: f64,
    pub cache_write: f64,
    /// 1-hour cache write; defaults to 2x input.
    pub cache_write_1h: Option<f64>,
    pub cache_read: f64,
}

#[derive(Deserialize, Debug, Clone)]
pub struct PriceTable {
    pub updated: String,
    pub models: BTreeMap<String, Price>,
}

const BUNDLED: &str = include_str!("../../resources/pricing.json");

pub fn table() -> &'static PriceTable {
    static T: OnceLock<PriceTable> = OnceLock::new();
    T.get_or_init(|| serde_json::from_str(BUNDLED).expect("bundled pricing.json is valid"))
}

fn strip_suffix_digits<'a>(s: &'a str, pattern: &[usize]) -> Option<&'a str> {
    // pattern: lengths of '-'-separated numeric groups at the end, e.g. [8] or [4,2,2].
    let mut rest = s;
    for len in pattern.iter().rev() {
        let (head, tail) = rest.rsplit_once('-')?;
        if tail.len() != *len || !tail.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        rest = head;
    }
    Some(rest)
}

/// `us.anthropic.claude-sonnet-4-5-20250929-v1:0` -> `claude-sonnet-4-5`,
/// `claude-opus-5-5[1m]` -> `claude-opus-5-5`, `openai/gpt-5-2025-08-07` -> `gpt-5`.
pub fn normalize(model: &str) -> String {
    let mut s = model.trim().to_ascii_lowercase();
    if let Some(i) = s.find('[') {
        s.truncate(i);
    }
    if let Some(i) = s.find('@') {
        s.truncate(i);
    }
    if let Some((_, tail)) = s.rsplit_once('/') {
        s = tail.to_string();
    }
    if let Some(i) = s.find("anthropic.") {
        s = s[i + "anthropic.".len()..].to_string();
    }
    // Bedrock version suffix `-v1:0` / `-v2`.
    if let Some((head, tail)) = s.rsplit_once("-v") {
        if !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit() || c == ':') {
            s = head.to_string();
        }
    }
    if let Some(h) = s.strip_suffix("-latest") {
        s = h.to_string();
    }
    if let Some(h) = strip_suffix_digits(&s, &[8]).or_else(|| strip_suffix_digits(&s, &[4, 2, 2])) {
        s = h.to_string();
    }
    s
}

pub fn price_for<'a>(t: &'a PriceTable, model: &str) -> Option<&'a Price> {
    t.models.get(&normalize(model))
}

/// Estimated USD cost, `None` when the model has no known price.
pub fn cost(t: &PriceTable, ev: &UsageEvent) -> Option<f64> {
    let p = price_for(t, &ev.model)?;
    let w1h = ev.cache_write_1h.min(ev.cache_write);
    let w5m = ev.cache_write - w1h;
    let per = |n: u64, price: f64| n as f64 * price / 1_000_000.0;
    Some(
        per(ev.input, p.input)
            + per(ev.output, p.output)
            + per(ev.cache_read, p.cache_read)
            + per(w5m, p.cache_write)
            + per(w1h, p.cache_write_1h.unwrap_or(p.input * 2.0)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Tool;

    fn ev(model: &str) -> UsageEvent {
        UsageEvent {
            ts: 0,
            tool: Tool::Claude,
            model: model.into(),
            input: 1_000_000,
            output: 1_000_000,
            cache_read: 1_000_000,
            cache_write: 2_000_000,
            cache_write_1h: 1_000_000,
            key: None,
        }
    }

    #[test]
    fn normalizes_ids() {
        assert_eq!(normalize("claude-sonnet-4-5-20250929"), "claude-sonnet-4-5");
        assert_eq!(normalize("us.anthropic.claude-sonnet-4-5-20250929-v1:0"), "claude-sonnet-4-5");
        assert_eq!(normalize("claude-opus-5-5[1m]"), "claude-opus-5-5");
        assert_eq!(normalize("claude-opus-4-5@20251101"), "claude-opus-4-5");
        assert_eq!(normalize("openai/gpt-5-2025-08-07"), "gpt-5");
        assert_eq!(normalize("codex-mini-latest"), "codex-mini");
        assert_eq!(normalize("GPT-5.1-Codex"), "gpt-5.1-codex");
        assert_eq!(normalize("claude-opus-4-1"), "claude-opus-4-1");
    }

    #[test]
    fn prices_known_and_leaves_unknown() {
        let t = table();
        assert!(!t.updated.is_empty());
        // opus 5.5: 4 + 20 + 0.2 + 5 (5m) + 8 (1h)
        let c = cost(t, &ev("claude-opus-5-5")).unwrap();
        assert!((c - 37.2).abs() < 1e-9, "{c}");
        // missing 1h price falls back to 2x input: gpt-5 1.25 + 10 + 0.125 + 0 + 2.5
        let c = cost(t, &ev("gpt-5-codex")).unwrap();
        assert!((c - 13.875).abs() < 1e-9, "{c}");
        assert!(cost(t, &ev("gpt-6.1-sol")).is_none());
        // a newer minor version must not inherit an older price
        assert!(cost(t, &ev("claude-opus-4-9")).is_none());
        assert!(price_for(t, "claude-haiku-4-5-20251001").is_some());
    }
}
