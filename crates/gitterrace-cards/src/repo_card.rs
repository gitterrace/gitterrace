use std::fmt::Write;

use gitterrace_index::CommitRecord;

use crate::badges::lang_color;
use crate::palette::Theme;
use crate::stats::repo_summary;
use crate::svg::{MONO, SANS, ago, area, clip, compact, esc, group, n};

const W: u32 = 495;
const H: u32 = 195;

/// A 495x195 README card: name, totals, languages bar and activity sparkline.
pub fn repo_card_svg(repo: &str, records: &[CommitRecord], now: i64, theme: Theme) -> String {
    let s = repo_summary(records);
    let p = theme.palette();
    let mut o = String::new();
    let _ = write!(
        o,
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}" role="img" aria-label="{al}"><title>{al}</title><style>.s{{font-family:{SANS}}}.m{{font-family:{MONO}}}</style><rect x="0.5" y="0.5" width="{}" height="{}" rx="8" fill="{}" stroke="{}"/><rect x="0.5" y="0.5" width="4" height="{}" fill="{ac}"/>"##,
        W - 1,
        H - 1,
        p.surface,
        p.line,
        H - 1,
        ac = p.accent,
        al = esc(&format!(
            "{repo}: {} commits, {} contributors",
            s.commits, s.contributors
        )),
    );
    let t = |o: &mut String,
             x: f64,
             y: f64,
             size: u32,
             w: u32,
             fill: &str,
             class: &str,
             v: &str| {
        let _ = write!(
            o,
            r##"<text x="{}" y="{}" font-size="{size}" font-weight="{w}" fill="{fill}" class="{class}">{}</text>"##,
            n(x),
            n(y),
            esc(v)
        );
    };
    t(&mut o, 24.0, 36.0, 18, 700, p.accent, "s", &clip(repo, 30));
    let last = s.last_commit.map_or("no commits".to_owned(), |l| {
        format!("last commit {}", ago(now, l))
    });
    t(&mut o, 24.0, 54.0, 11, 500, p.faint, "s", &last);
    let stats = [
        ("COMMITS", group(s.commits as u64)),
        ("CONTRIBUTORS", s.contributors.to_string()),
        ("LINES ADDED", compact(s.additions)),
        ("BUS FACTOR", s.bus_factor.to_string()),
    ];
    for (i, (l, v)) in stats.iter().enumerate() {
        let x = 24.0 + i as f64 * 112.0;
        t(&mut o, x, 86.0, 9, 700, p.faint, "m", l);
        t(&mut o, x, 108.0, 20, 700, p.fg, "s", v);
    }
    // Languages bar with legend.
    let (bx, bw) = (24.0, 447.0);
    let total: f64 = s.languages.iter().map(|l| l.1).sum::<f64>();
    if total > 0.0 {
        let _ = write!(
            o,
            r##"<clipPath id="lb"><rect x="{bx}" y="124" width="{bw}" height="6" rx="3"/></clipPath><g clip-path="url(#lb)">"##
        );
        let mut x = bx;
        for (i, (_, share)) in s.languages.iter().enumerate() {
            let sw = bw * share / total;
            let _ = write!(
                o,
                r##"<rect x="{}" y="124" width="{}" height="6" fill="{}"/>"##,
                n(x),
                n(sw),
                lang_color(p, i)
            );
            x += sw;
        }
        o.push_str("</g>");
        let mut x = bx;
        for (i, (name, share)) in s.languages.iter().take(4).enumerate() {
            let label = format!("{name} {:.0}%", share / total * 100.0);
            let _ = write!(
                o,
                r##"<circle cx="{}" cy="145" r="3.5" fill="{}"/>"##,
                n(x + 3.5),
                lang_color(p, i)
            );
            t(&mut o, x + 12.0, 149.0, 11, 500, p.muted, "s", &label);
            x += 12.0 + label.chars().count() as f64 * 6.2 + 14.0;
        }
    }
    // Sparkline of the last 26 weeks.
    let (fill, line) = area(&s.weekly, 24.0, 162.0, 447.0, 14.0);
    let _ = write!(
        o,
        r##"<path d="{fill}" fill="{c}" fill-opacity="0.22"/><path d="{line}" fill="none" stroke="{c}" stroke-width="1.5" stroke-linejoin="round"/></svg>"##,
        c = p.accent
    );
    o
}

#[cfg(test)]
mod tests {
    use crate::stats::fixtures::sample;
    use crate::{Theme, repo_card_svg};

    #[test]
    fn card_has_numbers_and_parses() {
        let r = sample();
        let now = r.iter().map(|x| x.time).max().unwrap_or(0) + 86_400;
        for t in [Theme::Light, Theme::Dark] {
            let a = repo_card_svg("demo", &r, now, t);
            assert_eq!(a, repo_card_svg("demo", &r, now, t));
            assert!(a.contains(">41<"));
            assert!(a.contains("Rust"));
            usvg::Tree::from_str(&a, &usvg::Options::default()).expect("parses");
        }
    }
}
