//! The one hover text of every chart mark: what the mark is, its value with its unit, and where
//! shares make sense its share of the column total and that total ("Rust: 41,230 lines (62%)
//! of 66,500"). A column of a stacked or area chart lists its whole breakdown. Charts never
//! write `data-tip` themselves, so every tip reads the same way and none drops its total.

use crate::fmt::{count, esc, thousands};

/// What a chart counts, singular and plural.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Unit(pub(crate) &'static str, pub(crate) &'static str);

pub(crate) const COMMITS: Unit = Unit("commit", "commits");
pub(crate) const LINES: Unit = Unit("line", "lines");
pub(crate) const CHANGES: Unit = Unit("change", "changes");

/// A share as shown: whole percent, `<1%` for a sliver, never a rounded-away `0%`.
pub(crate) fn share(part: u64, total: u64) -> String {
    if total == 0 {
        return percent(0.0);
    }
    percent(part as f64 / total as f64)
}

/// The one percent formatter of chart tips, for a fraction 0..=1: exactly 0 is `0%`, a sliver
/// that would round to 0 is `<1%`, the rest is a whole percent.
pub(crate) fn percent(f: f64) -> String {
    let p = f.clamp(0.0, 1.0) * 100.0;
    if p > 0.0 && p < 0.5 {
        "<1%".to_owned()
    } else {
        format!("{p:.0}%")
    }
}

/// "mark: 12 commits", for a single-series mark whose value is its column total.
pub(crate) fn value(mark: &str, n: u64, u: Unit) -> String {
    format!("{mark}: {}", count(n, u.0, u.1))
}

/// "mark: 41,230 lines (62%) of 66,500", for one part of a column.
pub(crate) fn part(mark: &str, n: u64, total: u64, u: Unit) -> String {
    format!(
        "{mark}: {} ({}) of {}",
        count(n, u.0, u.1),
        share(n, total),
        thousands(total)
    )
}

/// "mark: 40 commits: feat 20 (50%), fix 20 (50%)", a column with every part and its share.
/// Parts at zero are left out; the total is the sum of all parts.
pub(crate) fn breakdown(mark: &str, parts: &[(&str, u64)], u: Unit) -> String {
    let total: u64 = parts.iter().map(|p| p.1).sum();
    let rows: Vec<String> = parts
        .iter()
        .filter(|p| p.1 > 0)
        .map(|(name, n)| format!("{name} {} ({})", thousands(*n), share(*n, total)))
        .collect();
    if rows.is_empty() {
        return value(mark, 0, u);
    }
    format!("{}: {}", value(mark, total, u), rows.join(", "))
}

/// "mark: 42% of changed lines replaced existing code", for a rate that is its own share.
pub(crate) fn rate(mark: &str, share: f64, of: &str) -> String {
    format!("{mark}: {} {of}", percent(share))
}

/// The attribute that carries `text`, escaped: the only writer of a chart's `data-tip`.
pub(crate) fn attr(text: &str) -> String {
    format!(" data-tip=\"{}\"", esc(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_part_names_its_share_and_total() {
        assert_eq!(
            part("Rust, May 2026", 41_230, 66_500, LINES),
            "Rust, May 2026: 41,230 lines (62%) of 66,500"
        );
        assert_eq!(part("x", 1, 1000, LINES), "x: 1 line (<1%) of 1,000");
    }

    #[test]
    fn a_breakdown_lists_every_part_with_its_share() {
        assert_eq!(
            breakdown(
                "Q1 2026",
                &[("feat", 30), ("fix", 10), ("docs", 0)],
                COMMITS
            ),
            "Q1 2026: 40 commits: feat 30 (75%), fix 10 (25%)"
        );
        assert_eq!(breakdown("Q2", &[("feat", 0)], COMMITS), "Q2: 0 commits");
    }

    #[test]
    fn rates_and_shares_read_percent_the_same_way() {
        assert_eq!(percent(0.0), "0%");
        assert_eq!(percent(0.001), "<1%");
        assert_eq!(percent(0.006), "1%");
        assert_eq!(percent(1.0), "100%");
        assert_eq!(share(0, 10), "0%");
        assert_eq!(share(1, 1000), "<1%");
        assert_eq!(rate("May", 0.0, "rework"), "May: 0% rework");
        assert_eq!(rate("May", 0.0001, "rework"), "May: <1% rework");
        assert_eq!(rate("May", 0.42, "rework"), "May: 42% rework");
    }

    #[test]
    fn the_attribute_is_escaped() {
        assert_eq!(
            attr("a \"b\" <c>"),
            " data-tip=\"a &quot;b&quot; &lt;c&gt;\""
        );
    }
}
