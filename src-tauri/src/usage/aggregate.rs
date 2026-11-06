//! Aggregation of usage events into today (hourly) / daily / monthly reports
//! in the local time zone, plus CSV export.
use std::collections::{BTreeMap, BTreeSet, HashSet};

use chrono::{Datelike, NaiveDate, NaiveDateTime, Timelike};
use serde::{Deserialize, Serialize};

use super::pricing::{cost, PriceTable};
use super::UsageEvent;
use crate::model::Tool;

/// Longest range a report covers (protects against absurd inputs).
const MAX_DAYS: i64 = 3700;

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum View {
    Today,
    Daily,
    Monthly,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Query {
    pub view: View,
    /// `YYYY-MM-DD` (local). For `today` only `from` is used.
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub tools: Vec<Tool>,
    #[serde(default)]
    pub models: Vec<String>,
}

#[derive(Serialize, Debug, Clone, Default, PartialEq)]
pub struct Totals {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub total_tokens: u64,
    /// Billed model requests (events after deduplication).
    pub requests: u64,
    /// Estimated USD for the priced part.
    pub cost: f64,
    /// Tokens of models without a known price (not included in `cost`).
    pub unpriced_tokens: u64,
}

impl Totals {
    fn add(&mut self, ev: &UsageEvent, c: Option<f64>) {
        self.input += ev.input;
        self.output += ev.output;
        self.cache_read += ev.cache_read;
        self.cache_write += ev.cache_write;
        self.total_tokens += ev.total_tokens();
        self.requests += 1;
        match c {
            Some(c) => self.cost += c,
            None => self.unpriced_tokens += ev.total_tokens(),
        }
    }
    fn merge(&mut self, o: &Totals) {
        self.input += o.input;
        self.output += o.output;
        self.cache_read += o.cache_read;
        self.cache_write += o.cache_write;
        self.total_tokens += o.total_tokens;
        self.requests += o.requests;
        self.cost += o.cost;
        self.unpriced_tokens += o.unpriced_tokens;
    }
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Breakdown {
    pub tool: Tool,
    pub model: String,
    pub totals: Totals,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Row {
    /// `HH` (today), `YYYY-MM-DD` (daily) or `YYYY-MM` (monthly).
    pub key: String,
    pub totals: Totals,
    pub claude: Totals,
    pub codex: Totals,
    pub breakdown: Vec<Breakdown>,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Peak {
    pub key: String,
    pub cost: f64,
    pub tokens: u64,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Summary {
    pub totals: Totals,
    pub days: u32,
    pub daily_avg_cost: f64,
    pub daily_avg_tokens: f64,
    /// Highest row (day / month / hour) by cost, by tokens when nothing is priced.
    pub peak: Option<Peak>,
    /// cache_read / (input + cache_read + cache_write): the share of all prompt
    /// input served from the cache. Buckets do not overlap (Codex `input` has
    /// its cached part moved to `cache_read` at parse time). `None` without input.
    pub cache_hit_rate: Option<f64>,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct UsageReport {
    pub view: View,
    pub from: String,
    pub to: String,
    pub rows: Vec<Row>,
    pub summary: Summary,
    /// Every model with usage in the range (ignores the model filter), for the filter UI.
    pub models: Vec<String>,
    /// Models in the result without a known price.
    pub unknown_models: Vec<String>,
    pub pricing_updated: String,
    /// The first full log scan is still running.
    pub scanning: bool,
}

pub fn parse_date(s: &str) -> Option<NaiveDate> {
    let mut it = s.trim().splitn(3, '-');
    let y = it.next()?.parse().ok()?;
    let m = it.next()?.parse().ok()?;
    let d = it.next()?.parse().ok()?;
    NaiveDate::from_ymd_opt(y, m, d)
}

pub fn fmt_date(d: NaiveDate) -> String {
    format!("{:04}-{:02}-{:02}", d.year(), d.month(), d.day())
}

fn fmt_month(d: NaiveDate) -> String {
    format!("{:04}-{:02}", d.year(), d.month())
}

/// Epoch ms -> local wall-clock time.
pub fn local_time(ts: i64) -> Option<NaiveDateTime> {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_millis_opt(ts)
        .single()
        .map(|d| d.naive_local())
}

#[derive(Default)]
struct Acc {
    totals: Totals,
    claude: Totals,
    codex: Totals,
    by: BTreeMap<(u8, String), (Tool, Totals)>,
}

/// Builds the report. `to_local` converts epoch ms to local time (injected for tests).
pub fn build_report<'a>(
    events: impl Iterator<Item = &'a UsageEvent>,
    q: &Query,
    prices: &PriceTable,
    to_local: impl Fn(i64) -> Option<NaiveDateTime>,
) -> Result<UsageReport, String> {
    let from = parse_date(&q.from).ok_or_else(|| format!("invalid date: {}", q.from))?;
    let to = match q.view {
        View::Today => from,
        _ => parse_date(&q.to).ok_or_else(|| format!("invalid date: {}", q.to))?,
    };
    let (from, to) = if to < from { (to, from) } else { (from, to) };
    if (to - from).num_days() > MAX_DAYS {
        return Err("range too long".into());
    }
    let model_filter: HashSet<&str> = q.models.iter().map(|s| s.as_str()).collect();

    // Row keys in order, all present (zero rows keep the chart continuous).
    let mut keys: Vec<String> = Vec::new();
    match q.view {
        View::Today => keys.extend((0..24).map(|h| format!("{h:02}"))),
        View::Daily => {
            let mut d = from;
            while d <= to {
                keys.push(fmt_date(d));
                d = d.succ_opt().ok_or("date overflow")?;
            }
        }
        View::Monthly => {
            let mut d = from.with_day(1).ok_or("bad date")?;
            while d <= to {
                keys.push(fmt_month(d));
                d = if d.month() == 12 {
                    NaiveDate::from_ymd_opt(d.year() + 1, 1, 1)
                } else {
                    NaiveDate::from_ymd_opt(d.year(), d.month() + 1, 1)
                }
                .ok_or("date overflow")?;
            }
        }
    }
    let mut accs: BTreeMap<String, Acc> = keys.iter().map(|k| (k.clone(), Acc::default())).collect();
    let mut seen: HashSet<&str> = HashSet::new();
    let mut models: BTreeSet<String> = BTreeSet::new();
    let mut unknown: BTreeSet<String> = BTreeSet::new();

    for ev in events {
        if let Some(k) = ev.key.as_deref() {
            if !seen.insert(k) {
                continue;
            }
        }
        let Some(local) = to_local(ev.ts) else { continue };
        let day = local.date();
        if day < from || day > to {
            continue;
        }
        if !q.tools.is_empty() && !q.tools.contains(&ev.tool) {
            continue;
        }
        models.insert(ev.model.clone());
        if !model_filter.is_empty() && !model_filter.contains(ev.model.as_str()) {
            continue;
        }
        let key = match q.view {
            View::Today => format!("{:02}", local.hour()),
            View::Daily => fmt_date(day),
            View::Monthly => fmt_month(day),
        };
        let Some(acc) = accs.get_mut(&key) else { continue };
        let c = cost(prices, ev);
        if c.is_none() {
            unknown.insert(ev.model.clone());
        }
        acc.totals.add(ev, c);
        match ev.tool {
            Tool::Claude => acc.claude.add(ev, c),
            Tool::Codex => acc.codex.add(ev, c),
        }
        let tool_ord = if ev.tool == Tool::Claude { 0 } else { 1 };
        acc.by
            .entry((tool_ord, ev.model.clone()))
            .or_insert_with(|| (ev.tool, Totals::default()))
            .1
            .add(ev, c);
    }

    let mut total = Totals::default();
    let mut rows = Vec::with_capacity(keys.len());
    for k in keys {
        let acc = accs.remove(&k).unwrap_or_default();
        total.merge(&acc.totals);
        let mut breakdown: Vec<Breakdown> = acc
            .by
            .into_iter()
            .map(|((_, model), (tool, totals))| Breakdown { tool, model, totals })
            .collect();
        breakdown.sort_by(|a, b| {
            b.totals
                .cost
                .total_cmp(&a.totals.cost)
                .then(b.totals.total_tokens.cmp(&a.totals.total_tokens))
        });
        rows.push(Row {
            key: k,
            totals: acc.totals,
            claude: acc.claude,
            codex: acc.codex,
            breakdown,
        });
    }

    let any_cost = rows.iter().any(|r| r.totals.cost > 0.0);
    let peak = rows
        .iter()
        .filter(|r| r.totals.total_tokens > 0)
        .max_by(|a, b| {
            if any_cost {
                a.totals.cost.total_cmp(&b.totals.cost)
            } else {
                a.totals.total_tokens.cmp(&b.totals.total_tokens)
            }
        })
        .map(|r| Peak {
            key: r.key.clone(),
            cost: r.totals.cost,
            tokens: r.totals.total_tokens,
        });
    let days = ((to - from).num_days() + 1).max(1) as u32;
    let denom = total.input + total.cache_read + total.cache_write;
    let summary = Summary {
        daily_avg_cost: total.cost / f64::from(days),
        daily_avg_tokens: total.total_tokens as f64 / f64::from(days),
        cache_hit_rate: (denom > 0).then(|| total.cache_read as f64 / denom as f64),
        totals: total,
        days,
        peak,
    };
    Ok(UsageReport {
        view: q.view,
        from: fmt_date(from),
        to: fmt_date(to),
        rows,
        summary,
        models: models.into_iter().collect(),
        unknown_models: unknown.into_iter().collect(),
        pricing_updated: prices.updated.clone(),
        scanning: false,
    })
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// One line per period x tool x model; unknown-price lines leave `cost_usd` empty.
pub fn to_csv(r: &UsageReport) -> String {
    let mut out = String::from(
        "period,tool,model,input,output,cache_read,cache_write,total_tokens,cost_usd,price_known\n",
    );
    for row in &r.rows {
        for b in &row.breakdown {
            let t = &b.totals;
            let known = t.unpriced_tokens == 0;
            let tool = match b.tool {
                Tool::Claude => "Claude Code",
                Tool::Codex => "Codex",
            };
            out.push_str(&format!(
                "{},{},{},{},{},{},{},{},{},{}\n",
                csv_field(&row.key),
                tool,
                csv_field(&b.model),
                t.input,
                t.output,
                t.cache_read,
                t.cache_write,
                t.total_tokens,
                if known || t.cost > 0.0 { format!("{:.6}", t.cost) } else { String::new() },
                known
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::pricing::table;
    use chrono::{FixedOffset, TimeZone};

    fn ev(ts: &str, tool: Tool, model: &str, input: u64, cache_read: u64, key: Option<&str>) -> UsageEvent {
        UsageEvent {
            ts: crate::usage::parse_ts(ts).unwrap(),
            tool,
            model: model.into(),
            input,
            output: 1000,
            cache_read,
            cache_write: 0,
            cache_write_1h: 0,
            key: key.map(String::from),
        }
    }

    /// UTC+8 like the user's machine.
    fn cst(ts: i64) -> Option<NaiveDateTime> {
        FixedOffset::east_opt(8 * 3600)?
            .timestamp_millis_opt(ts)
            .single()
            .map(|d| d.naive_local())
    }

    fn q(view: View, from: &str, to: &str) -> Query {
        Query { view, from: from.into(), to: to.into(), tools: vec![], models: vec![] }
    }

    fn sample() -> Vec<UsageEvent> {
        vec![
            // 2026-11-04 23:30 local (UTC+8) -> Nov 4, not Nov 5
            ev("2026-11-04T15:30:00Z", Tool::Claude, "claude-opus-5-5", 1_000_000, 0, Some("a")),
            // 2026-11-05 00:10 local -> Nov 5
            ev("2026-11-04T16:10:00Z", Tool::Claude, "claude-opus-5-5", 0, 1_000_000, Some("b")),
            ev("2026-11-04T16:10:00Z", Tool::Claude, "claude-opus-5-5", 0, 1_000_000, Some("b")),
            ev("2026-11-05T03:00:00Z", Tool::Codex, "gpt-9-unknown", 500, 500, None),
            ev("2026-11-05T04:00:00Z", Tool::Codex, "gpt-5-codex", 1_000_000, 0, None),
        ]
    }

    #[test]
    fn daily_respects_local_day_boundary_and_dedup() {
        let evs = sample();
        let r = build_report(evs.iter(), &q(View::Daily, "2026-11-03", "2026-11-05"), table(), cst).unwrap();
        assert_eq!(r.rows.iter().map(|x| x.key.as_str()).collect::<Vec<_>>(), ["2026-11-03", "2026-11-04", "2026-11-05"]);
        let d4 = &r.rows[1];
        assert_eq!(d4.totals.input, 1_000_000);
        // 1M input * $4 + 1000 output * $20/M
        assert!((d4.totals.cost - 4.02).abs() < 1e-9);
        let d5 = &r.rows[2];
        assert_eq!(d5.claude.cache_read, 1_000_000, "duplicate key counted once");
        assert_eq!(d5.codex.total_tokens, 1000 + 1000 + 1_000_000 + 1000);
        assert_eq!(d5.codex.unpriced_tokens, 2000);
        assert_eq!(r.unknown_models, vec!["gpt-9-unknown".to_string()]);
        assert_eq!(d5.breakdown.len(), 3);
        assert_eq!(r.summary.days, 3);
        let peak = r.summary.peak.as_ref().unwrap();
        assert_eq!(peak.key, "2026-11-04");
        // hit rate = read / (input + read + write)
        let s = &r.summary;
        assert_eq!(s.totals.requests, 4, "duplicate key counted once");
        assert_eq!(d4.totals.requests, 1);
        assert_eq!(d5.totals.requests, 3);
        let expect = 1_000_500.0 / (2_000_500.0 + 1_000_500.0);
        assert!((s.cache_hit_rate.unwrap() - expect).abs() < 1e-12);
        assert!((s.daily_avg_cost - s.totals.cost / 3.0).abs() < 1e-12);
    }

    #[test]
    fn filters_today_hours_and_months() {
        let evs = sample();
        let mut qq = q(View::Daily, "2026-11-05", "2026-11-05");
        qq.tools = vec![Tool::Codex];
        let r = build_report(evs.iter(), &qq, table(), cst).unwrap();
        assert_eq!(r.rows[0].claude.total_tokens, 0);
        assert_eq!(r.models, vec!["gpt-5-codex".to_string(), "gpt-9-unknown".to_string()]);
        qq.models = vec!["gpt-5-codex".into()];
        let r = build_report(evs.iter(), &qq, table(), cst).unwrap();
        assert_eq!(r.rows[0].totals.input, 1_000_000);
        assert!(r.unknown_models.is_empty());
        assert_eq!(r.models.len(), 2, "model list ignores the model filter");

        let r = build_report(evs.iter(), &q(View::Today, "2026-11-05", ""), table(), cst).unwrap();
        assert_eq!(r.rows.len(), 24);
        assert_eq!(r.rows[0].claude.cache_read, 1_000_000); // 00:10
        assert_eq!(r.rows[11].codex.input, 500); // 03:00Z = 11:00
        assert_eq!(r.rows[12].codex.input, 1_000_000);

        let r = build_report(evs.iter(), &q(View::Monthly, "2026-10-15", "2026-11-30"), table(), cst).unwrap();
        assert_eq!(r.rows.iter().map(|x| x.key.as_str()).collect::<Vec<_>>(), ["2026-10", "2026-11"]);
        assert_eq!(r.rows[1].totals.input, 2_000_500);
        assert!(build_report(evs.iter(), &q(View::Daily, "x", "y"), table(), cst).is_err());
    }

    #[test]
    fn hit_rate_counts_cache_writes_and_requests_per_day() {
        let mut a = ev("2026-11-05T03:00:00Z", Tool::Claude, "claude-opus-5-5", 100, 600, Some("p"));
        a.cache_write = 300;
        let b = ev("2026-11-05T05:00:00Z", Tool::Claude, "claude-opus-5-5", 0, 0, Some("q"));
        let evs = vec![a.clone(), a, b];
        let r = build_report(evs.iter(), &q(View::Daily, "2026-11-05", "2026-11-05"), table(), cst).unwrap();
        assert_eq!(r.rows[0].totals.requests, 2);
        assert_eq!(r.rows[0].claude.requests, 2);
        assert_eq!(r.rows[0].breakdown[0].totals.requests, 2);
        assert!((r.summary.cache_hit_rate.unwrap() - 0.6).abs() < 1e-12);
        let empty: Vec<UsageEvent> = vec![];
        let r = build_report(empty.iter(), &q(View::Daily, "2026-11-05", "2026-11-05"), table(), cst).unwrap();
        assert_eq!(r.summary.cache_hit_rate, None);
        assert_eq!(r.summary.totals.requests, 0);
    }

    #[test]
    fn csv_has_one_line_per_model_and_blank_unknown_cost() {
        let evs = sample();
        let r = build_report(evs.iter(), &q(View::Daily, "2026-11-05", "2026-11-05"), table(), cst).unwrap();
        let csv = to_csv(&r);
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines.len(), 4);
        assert!(lines.iter().any(|l| l.starts_with("2026-11-05,Codex,gpt-9-unknown,") && l.ends_with(",,false")));
        assert!(lines.iter().any(|l| l.starts_with("2026-11-05,Claude Code,claude-opus-5-5,") && l.ends_with(",true")));
    }
}
