//! Image targets: in-repository ones go to `/raw`, and none loads from another host when it is
//! off.

use crate::markdown::{Base, image_url, render_with, rewrite_url};

fn base() -> Base {
    Base { rev: "r".into(), dir: String::new(), highlight_bytes: crate::highlight::HIGHLIGHT_BYTES }
}

#[test]
fn relative_images_go_to_raw() {
    let out = render_with(
        "![logo](img/l.svg)\n\n<img src=\"../p.png\" alt=\"p\">",
        Base::new("abc123", "docs"),
    );
    assert!(out.contains("src=\"/raw/abc123/docs/img/l.svg\""));
    assert!(out.contains("src=\"/raw/abc123/p.png\""));
    assert_eq!(rewrite_url(&base(), "https://x/y.png", true), "https://x/y.png");
}

#[test]
fn protocol_relative_image_src_is_dropped_when_raw_is_off() {
    let mut cfg = gitterrace_config::Config::default();
    cfg.features.raw_enabled = false;
    let off = crate::features::Effective::of(&cfg, None);
    let src = crate::features::with(off, || image_url(&base(), "//cdn.example.com/x.png"));
    assert_eq!(src, None, "an image from another host is never loaded");
}
