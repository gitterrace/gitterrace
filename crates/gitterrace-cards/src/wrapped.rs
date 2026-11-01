use std::fmt::Write;

use crate::palette::{Palette, Theme};
use crate::stats::WrappedStats;
use crate::svg::{MONO, SANS, area, clip, count, esc, group, n};

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const DAYS: [&str; 7] = [
    "Mondays",
    "Tuesdays",
    "Wednesdays",
    "Thursdays",
    "Fridays",
    "Saturdays",
    "Sundays",
];

/// Canvas shape of the Repo Wrapped card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// 1200x630, the social preview size.
    Landscape,
    /// 1080x1350, for stories and phone feeds.
    Portrait,
}

impl Layout {
    /// `landscape` or `portrait`; anything else is `None`.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "landscape" => Some(Self::Landscape),
            "portrait" => Some(Self::Portrait),
            _ => None,
        }
    }
}

impl Layout {
    pub fn size(self) -> (u32, u32) {
        match self {
            Layout::Landscape => (1200, 630),
            Layout::Portrait => (1080, 1350),
        }
    }
}

/// The year-in-review poster as a self-contained SVG.
pub fn wrapped_svg(w: &WrappedStats, layout: Layout, theme: Theme) -> String {
    let p = theme.palette();
    let (cw, ch) = layout.size();
    let mut s = String::with_capacity(12_000);
    let _ = write!(
        s,
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{cw}" height="{ch}" viewBox="0 0 {cw} {ch}" role="img" aria-label="{label}"><title>{label}</title>"##,
        label = esc(&format!(
            "{} in {}: {}",
            w.repo,
            w.year,
            count(w.commits as u64, "commit", "commits")
        )),
    );
    let _ = write!(
        s,
        r##"<defs><linearGradient id="bg" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="{bg}"/><stop offset="1" stop-color="{bg2}"/></linearGradient><radialGradient id="glow" cx="0.5" cy="0.5" r="0.5"><stop offset="0" stop-color="{ac}" stop-opacity="0.28"/><stop offset="1" stop-color="{ac}" stop-opacity="0"/></radialGradient><linearGradient id="fill" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="{ac}" stop-opacity="0.35"/><stop offset="1" stop-color="{ac}" stop-opacity="0"/></linearGradient></defs><style>.s{{font-family:{SANS}}}.m{{font-family:{MONO}}}</style>"##,
        bg = p.bg,
        bg2 = p.bg2,
        ac = p.accent,
    );
    let _ = write!(s, r##"<rect width="{cw}" height="{ch}" fill="url(#bg)"/>"##);
    // Contour rings echo the "gitterrace" layers behind everything.
    let (gx, gy) = if layout == Layout::Landscape {
        (1020.0, 80.0)
    } else {
        (940.0, 120.0)
    };
    for i in 0..5 {
        let _ = write!(
            s,
            r##"<circle cx="{gx}" cy="{gy}" r="{}" fill="none" stroke="{}" stroke-opacity="{}" stroke-width="1.5"/>"##,
            120 + i * 70,
            p.accent,
            n(0.16 - f64::from(i) * 0.028),
        );
    }
    match layout {
        Layout::Landscape => landscape(&mut s, w, p),
        Layout::Portrait => portrait(&mut s, w, p),
    }
    s.push_str("</svg>");
    s
}

fn text(s: &mut String, x: f64, y: f64, size: u32, weight: u32, fill: &str, class: &str, t: &str) {
    let _ = write!(
        s,
        r##"<text x="{}" y="{}" font-size="{size}" font-weight="{weight}" fill="{fill}" class="{class}">{}</text>"##,
        n(x),
        n(y),
        esc(t),
    );
}

fn header(
    s: &mut String,
    w: &WrappedStats,
    p: &Palette,
    x: f64,
    y: f64,
    name_size: u32,
    max: usize,
) {
    text(
        s,
        x,
        y,
        20,
        700,
        p.accent,
        "m",
        &format!("GITTERRACE  /  REPO WRAPPED {}", w.year),
    );
    text(
        s,
        x,
        y + f64::from(name_size) + 14.0,
        name_size,
        800,
        p.fg,
        "s",
        &clip(&w.repo, max),
    );
}

fn hero(s: &mut String, w: &WrappedStats, p: &Palette, x: f64, y: f64, size: u32) {
    text(s, x, y, size, 800, p.accent, "s", &group(w.commits as u64));
    text(s, x, y + 40.0, 26, 500, p.muted, "s", "commits this year");
}

fn tile(s: &mut String, x: f64, y: f64, label: &str, value: &str, p: &Palette, big: u32) {
    text(s, x, y, 15, 700, p.faint, "m", &label.to_uppercase());
    text(s, x, y + f64::from(big) + 8.0, big, 700, p.fg, "s", value);
}

fn facts(w: &WrappedStats) -> Vec<(&'static str, String)> {
    let mut v = vec![
        ("Active days", group(w.active_days as u64)),
        (
            "Longest streak",
            count(w.longest_streak as u64, "day", "days"),
        ),
    ];
    if let Some(m) = w.busiest_month {
        v.push(("Busiest month", MONTHS[m].to_owned()));
    }
    if let Some(d) = w.busiest_weekday {
        v.push(("Busiest weekday", DAYS[d].to_owned()));
    }
    if let Some(h) = w.busiest_hour {
        v.push(("Busiest hour", format!("{h:02}:00")));
    }
    if let Some(b) = &w.biggest_commit {
        v.push(("Biggest commit", count(b.lines, "line", "lines")));
    }
    v
}

fn focus(w: &WrappedStats) -> Option<(&'static str, String)> {
    if let Some((l, share)) = &w.top_language {
        Some(("Top language", format!("{l} ({:.0}%)", share * 100.0)))
    } else {
        w.top_dir
            .as_ref()
            .map(|(d, c)| ("Top directory", format!("{} ({c})", clip(d, 22))))
    }
}

fn authors(s: &mut String, w: &WrappedStats, p: &Palette, x: f64, y: f64, width: f64, row: f64) {
    text(s, x, y, 15, 700, p.faint, "m", "TOP AUTHORS");
    let max = w.top_authors.first().map_or(1, |a| a.1.max(1)) as f64;
    for (i, (name, c)) in w.top_authors.iter().enumerate() {
        let ry = y + 36.0 + row * i as f64;
        text(s, x, ry, 22, 600, p.fg, "s", &clip(name, 24));
        let _ = write!(
            s,
            r##"<text x="{}" y="{}" font-size="20" font-weight="600" fill="{}" class="m" text-anchor="end">{}</text>"##,
            n(x + width),
            n(ry),
            p.muted,
            group(*c as u64)
        );
        let bw = (width * *c as f64 / max).max(6.0);
        let _ = write!(
            s,
            r##"<rect x="{x}" y="{}" width="{width}" height="6" rx="3" fill="{}"/><rect x="{x}" y="{}" width="{}" height="6" rx="3" fill="{}" fill-opacity="{}"/>"##,
            n(ry + 10.0),
            p.line,
            n(ry + 10.0),
            n(bw),
            p.accent,
            n(1.0 - i as f64 * 0.25),
            x = n(x),
            width = n(width),
        );
    }
}

fn spark(s: &mut String, w: &WrappedStats, p: &Palette, x: f64, y: f64, width: f64, h: f64) {
    let (fill, line) = area(&w.weekly, x, y, width, h);
    let _ = write!(
        s,
        r##"<line x1="{x}" y1="{b}" x2="{r}" y2="{b}" stroke="{}" stroke-width="1.5"/><path d="{fill}" fill="url(#fill)"/><path d="{line}" fill="none" stroke="{}" stroke-width="3" stroke-linejoin="round" stroke-linecap="round"/>"##,
        p.line,
        p.accent,
        b = n(y + h),
        r = n(x + width),
        x = n(x),
    );
    text(
        s,
        x,
        y + h + 28.0,
        15,
        700,
        p.faint,
        "m",
        &format!("JAN {}", w.year),
    );
    let _ = write!(
        s,
        r##"<text x="{}" y="{}" font-size="15" font-weight="700" fill="{}" class="m" text-anchor="end">DEC {}</text>"##,
        n(x + width),
        n(y + h + 28.0),
        p.faint,
        w.year
    );
}

fn grid(
    s: &mut String,
    w: &WrappedStats,
    p: &Palette,
    x: f64,
    y: f64,
    cols: usize,
    dx: f64,
    dy: f64,
    big: u32,
    clip_at: usize,
) {
    let mut f = facts(w);
    if let Some(focus_fact) = focus(w) {
        f.push(focus_fact);
    }
    for (i, (l, v)) in f.iter().enumerate() {
        let col = (i % cols) as f64;
        let row = (i / cols) as f64;
        tile(s, x + col * dx, y + row * dy, l, &clip(v, clip_at), p, big);
    }
}

fn landscape(s: &mut String, w: &WrappedStats, p: &Palette) {
    header(s, w, p, 64.0, 84.0, 52, 22);
    hero(s, w, p, 64.0, 290.0, 128);
    grid(s, w, p, 520.0, 200.0, 3, 220.0, 76.0, 26, 15);
    authors(s, w, p, 64.0, 392.0, 380.0, 50.0);
    spark(s, w, p, 520.0, 440.0, 616.0, 96.0);
}

fn portrait(s: &mut String, w: &WrappedStats, p: &Palette) {
    header(s, w, p, 72.0, 110.0, 68, 18);
    hero(s, w, p, 72.0, 440.0, 190);
    grid(s, w, p, 72.0, 560.0, 2, 470.0, 104.0, 36, 20);
    authors(s, w, p, 72.0, 990.0, 936.0, 62.0);
    spark(s, w, p, 72.0, 1200.0, 936.0, 70.0);
}

#[cfg(test)]
mod tests {
    use crate::stats::fixtures::sample;
    use crate::{Layout, Theme, wrapped_stats, wrapped_svg};

    #[test]
    fn contains_numbers_and_is_deterministic() {
        let w = wrapped_stats("demo", &sample(), 2025);
        for layout in [Layout::Landscape, Layout::Portrait] {
            for theme in [Theme::Light, Theme::Dark] {
                let a = wrapped_svg(&w, layout, theme);
                assert_eq!(a, wrapped_svg(&w, layout, theme));
                assert!(a.contains(">41<"));
                assert!(a.contains("demo"));
                assert!(a.contains("alice"));
                // The card is inert: no links, only the xmlns namespace URI.
                assert!(!a.contains("href"));
                usvg::Tree::from_str(&a, &usvg::Options::default()).expect("parses");
            }
        }
        let (l, d) = (
            wrapped_svg(&w, Layout::Landscape, Theme::Light),
            wrapped_svg(&w, Layout::Landscape, Theme::Dark),
        );
        assert_ne!(l, d);

        // Hostile names are escaped, and a year with no commits still renders.
        let hostile = wrapped_svg(
            &wrapped_stats("<b>&", &sample(), 2025),
            Layout::Landscape,
            Theme::Light,
        );
        assert!(hostile.contains("&lt;b&gt;&amp;"));
        usvg::Tree::from_str(&hostile, &usvg::Options::default()).expect("parses");
        let empty = wrapped_svg(
            &wrapped_stats("demo", &sample(), 1999),
            Layout::Portrait,
            Theme::Dark,
        );
        usvg::Tree::from_str(&empty, &usvg::Options::default()).expect("parses");
    }
}
