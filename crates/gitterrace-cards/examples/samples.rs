//! Writes sample cards for a repo: `cargo run -p gitterrace-cards --features png --example samples -- <repo> <out>`.
#![allow(clippy::unwrap_used)]
use std::path::PathBuf;

use gitterrace_cards::{Badge, Layout, Theme};

fn main() {
    let mut a = std::env::args().skip(1);
    let repo = PathBuf::from(a.next().unwrap_or_else(|| ".".into()));
    let out = PathBuf::from(a.next().unwrap_or_else(|| "out".into()));
    std::fs::create_dir_all(&out).unwrap();
    let r = gitterrace_git::Repo::discover(&repo).unwrap();
    let idx = gitterrace_index::Index::open(&r).unwrap();
    let recs = idx.records(&r, "HEAD").unwrap();
    let year: i32 = a.next().and_then(|y| y.parse().ok()).unwrap_or(2026);
    let now = recs.iter().map(|r| r.time).max().unwrap_or(0) + 3600;
    let name = repo
        .canonicalize()
        .unwrap()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let w = gitterrace_cards::wrapped_stats(&name, &recs, year);
    let mut files: Vec<(String, String)> = Vec::new();
    for (tn, t) in [("light", Theme::Light), ("dark", Theme::Dark)] {
        for (ln, l) in [
            ("landscape", Layout::Landscape),
            ("portrait", Layout::Portrait),
        ] {
            files.push((
                format!("wrapped-{ln}-{tn}"),
                gitterrace_cards::wrapped_svg(&w, l, t),
            ));
        }
        files.push((
            format!("card-{tn}"),
            gitterrace_cards::repo_card_svg(&name, &recs, now, t),
        ));
        for b in Badge::ALL {
            files.push((
                format!("badge-{}-{tn}", b.name()),
                gitterrace_cards::badge_svg(b, &recs, now, t),
            ));
        }
    }
    for (n, svg) in files {
        std::fs::write(out.join(format!("{n}.svg")), &svg).unwrap();
        #[cfg(feature = "png")]
        if n.starts_with("badge") {
            std::fs::write(
                out.join(format!("{n}.png")),
                gitterrace_cards::png::render_scaled(&svg, 4.0).unwrap(),
            )
            .unwrap();
        } else {
            std::fs::write(
                out.join(format!("{n}.png")),
                gitterrace_cards::png::render(&svg).unwrap(),
            )
            .unwrap();
        }
    }
}
