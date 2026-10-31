use std::fmt::Write;

use gitterrace_index::CommitRecord;

use crate::Error;
use crate::palette::Theme;
use crate::stats::{RepoSummary, repo_summary};
use crate::svg::{SANS, ago, area, compact, esc, n, text_width};

const FONT: f64 = 11.0;

/// The README badges. `name()` is the route segment, e.g. `/badges/commits.svg`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Badge {
    Commits,
    Contributors,
    LastCommit,
    Activity,
    BusFactor,
    Languages,
}

impl Badge {
    pub const ALL: [Badge; 6] = [
        Badge::Commits,
        Badge::Contributors,
        Badge::LastCommit,
        Badge::Activity,
        Badge::BusFactor,
        Badge::Languages,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Badge::Commits => "commits",
            Badge::Contributors => "contributors",
            Badge::LastCommit => "last-commit",
            Badge::Activity => "activity",
            Badge::BusFactor => "bus-factor",
            Badge::Languages => "languages",
        }
    }

    /// Parses a route segment, with or without the `.svg` suffix.
    pub fn parse(s: &str) -> Result<Self, Error> {
        let s = s.strip_suffix(".svg").unwrap_or(s);
        Self::ALL
            .into_iter()
            .find(|b| b.name() == s)
            .ok_or_else(|| Error::UnknownBadge(s.to_owned()))
    }
}

/// A flat 20px badge. `now` (epoch seconds) is only used for "last commit" and is a parameter
/// so output stays deterministic.
pub fn badge_svg(badge: Badge, records: &[CommitRecord], now: i64, theme: Theme) -> String {
    badge_from_summary(badge, &repo_summary("", records), now, theme)
}

pub(crate) fn badge_from_summary(badge: Badge, s: &RepoSummary, now: i64, theme: Theme) -> String {
    let p = theme.palette();
    let label = match badge {
        Badge::Commits => "commits",
        Badge::Contributors => "contributors",
        Badge::LastCommit => "last commit",
        Badge::Activity => "commits/week",
        Badge::BusFactor => "bus factor",
        Badge::Languages => "languages",
    };
    let value = match badge {
        Badge::Commits => compact(s.commits as u64),
        Badge::Contributors => s.contributors.to_string(),
        Badge::LastCommit => s.last_commit.map_or("never".to_owned(), |t| ago(now, t)),
        Badge::BusFactor => s.bus_factor.to_string(),
        Badge::Activity | Badge::Languages => String::new(),
    };
    let (label_bg, label_fg) = match theme {
        Theme::Light => ("#55534e", "#ffffff"),
        Theme::Dark => ("#2a2d33", "#c9c7c1"),
    };
    let lw = (text_width(label, FONT) + 14.0).ceil();
    let vw = match badge {
        Badge::Activity => 92.0,
        Badge::Languages => 120.0,
        _ => (text_width(&value, FONT) + 14.0).ceil(),
    };
    let total = lw + vw;
    let mut o = String::new();
    let _ = write!(
        o,
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{t}" height="20" viewBox="0 0 {t} 20" role="img" aria-label="{al}"><title>{al}</title><clipPath id="r"><rect width="{t}" height="20" rx="3"/></clipPath><g clip-path="url(#r)"><rect width="{lw}" height="20" fill="{lb}"/><rect x="{lw}" width="{vw}" height="20" fill="{ac}"/></g>"##,
        t = n(total),
        lw = n(lw),
        vw = n(vw),
        lb = label_bg,
        ac = if badge == Badge::Languages {
            p.bg2
        } else {
            p.accent
        },
        al = esc(&format!("{label}: {value}")),
    );
    let ty = 14.0;
    let _ = write!(
        o,
        r##"<g font-family="{SANS}" font-size="{FONT}" font-weight="500" text-anchor="middle"><text x="{}" y="{ty}" fill="{label_fg}">{}</text>"##,
        n(lw / 2.0),
        esc(label),
    );
    match badge {
        Badge::Activity => {
            let (fill, line) = area(&s.weekly, lw + 6.0, 4.0, vw - 12.0, 12.0);
            let _ = write!(
                o,
                r##"<path d="{fill}" fill="{c}" fill-opacity="0.35"/><path d="{line}" fill="none" stroke="{c}" stroke-width="1.5" stroke-linejoin="round"/>"##,
                c = p.on_accent,
            );
        }
        Badge::Languages => {
            let mut x = lw + 6.0;
            let w = vw - 12.0;
            let _ = write!(
                o,
                r##"<clipPath id="b"><rect x="{}" y="6" width="{}" height="8" rx="4"/></clipPath><g clip-path="url(#b)">"##,
                n(x),
                n(w)
            );
            let total: f64 = s
                .languages
                .iter()
                .map(|l| l.1)
                .sum::<f64>()
                .max(f64::EPSILON);
            for (i, (_, share)) in s.languages.iter().enumerate() {
                let sw = w * share / total;
                let _ = write!(
                    o,
                    r##"<rect x="{}" y="6" width="{}" height="8" fill="{}"/>"##,
                    n(x),
                    n(sw),
                    lang_color(&p, i)
                );
                x += sw;
            }
            o.push_str("</g>");
        }
        _ => {
            let _ = write!(
                o,
                r##"<text x="{}" y="{ty}" fill="{}">{}</text>"##,
                n(lw + vw / 2.0),
                p.on_accent,
                esc(&value),
            );
        }
    }
    o.push_str("</g></svg>");
    o
}

pub(crate) fn lang_color(p: &crate::palette::Palette, i: usize) -> &'static str {
    p.series[i.min(7)]
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use crate::stats::fixtures::sample;
    use crate::{Badge, Theme, badge_svg};

    #[test]
    fn all_badges_parse_and_are_stable() {
        let r = sample();
        let now = r.iter().map(|x| x.time).max().unwrap_or(0) + 3 * 86_400;
        for b in Badge::ALL {
            for t in [Theme::Light, Theme::Dark] {
                let a = badge_svg(b, &r, now, t);
                assert_eq!(a, badge_svg(b, &r, now, t));
                assert!(a.contains("height=\"20\""));
                usvg::Tree::from_str(&a, &usvg::Options::default()).expect("parses");
            }
        }
    }

    #[test]
    fn values() {
        let r = sample();
        let now = r.iter().map(|x| x.time).max().unwrap_or(0) + 3 * 86_400;
        assert!(badge_svg(Badge::Commits, &r, now, Theme::Light).contains(">41<"));
        assert!(badge_svg(Badge::Contributors, &r, now, Theme::Light).contains(">3<"));
        assert!(badge_svg(Badge::LastCommit, &r, now, Theme::Light).contains("3 days ago"));
        assert!(badge_svg(Badge::BusFactor, &r, now, Theme::Light).contains(">1<"));
        assert_eq!(
            Badge::parse("last-commit.svg").ok(),
            Some(Badge::LastCommit)
        );
        assert!(Badge::parse("nope").is_err());
    }
}
