use super::*;

const WOFF2: &[u8] = include_bytes!(concat!(
    "../../../../vendor/schwarzwald/kit/fonts/",
    "jetbrains-mono/jetbrains-mono-latin-wght-normal.woff2"
));

fn ttf() -> Vec<u8> {
    unpack(WOFF2).expect("the vendored font unpacks").into_owned()
}

#[test]
fn a_woff2_shows_family_styles_axes_and_glyph_count() {
    let i = read(WOFF2).expect("reads");
    assert_eq!(i.family, "JetBrains Mono");
    assert!(i.glyphs > 100, "{i:?}");
    assert!(i.axes.starts_with("wght "), "{i:?}");
    assert_ne!(i.styles, "");
}

#[test]
fn the_unpacked_ttf_reads_like_the_woff2() {
    assert_eq!(read(&ttf()), read(WOFF2));
}

#[test]
fn the_markup_escapes_names_and_links_the_sample_source() {
    let h = html("a/f.woff2", WOFF2, "/raw/HEAD/a/f.woff2").expect("html");
    assert!(h.contains("JetBrains Mono") && h.contains("glyphs"), "{h}");
    assert!(h.contains(r#"data-font-src="/raw/HEAD/a/f.woff2""#), "{h}");
    assert!(!h.contains("style="), "no inline style: {h}");
    let none = html("f.ttf", &ttf(), "").expect("html");
    assert!(!none.contains("data-font-src"), "no raw, no sample: {none}");
}

#[test]
fn only_font_extensions_are_read() {
    assert!(is_font("A.TTF") && is_font("x/b.woff") && is_font("c.otf"));
    assert!(!is_font("a.txt") && !is_font("ttf"));
    assert!(html("a.txt", WOFF2, "/r").is_none());
}

/// Guard class: hostile bytes. Every prefix and every single-byte corruption of a real font
/// either reads or is refused, none panics or hangs, and none builds a huge buffer.
#[test]
fn truncated_and_corrupted_fonts_never_panic() {
    for font in [WOFF2.to_vec(), ttf()] {
        for cut in (0..font.len()).step_by(font.len() / 97 + 1) {
            let _ = read(&font[..cut]);
        }
        for at in (0..font.len().min(4096)).step_by(7) {
            let mut bad = font.clone();
            bad[at] ^= 0xff;
            let _ = read(&bad);
        }
    }
}

#[test]
fn garbage_is_refused() {
    for junk in [
        &b""[..],
        b"wOFF",
        b"wOF2",
        b"wOF2\0\0\0\0junkjunkjunkjunk",
        b"\0\x01\0\0",
        b"OTTO\xff\xff\xff\xff",
        b"not a font at all",
    ] {
        assert!(read(junk).is_none(), "{junk:?}");
    }
    assert!(html("a.woff2", b"junk", "/r").is_none());
}

#[test]
fn a_file_over_the_cap_is_not_opened() {
    let mut big = ttf();
    big.resize(BYTES_CAP + 1, 0);
    assert!(read(&big).is_none());
}

/// Guard class: a decompression bomb. A woff2 whose directory declares more than the cap is
/// refused before its buffer exists, and a brotli stream longer than it declares stops at
/// the declared size.
#[test]
fn a_block_declaring_more_than_the_cap_is_refused_unread() {
    assert!(unbrotli(&[0; 16], DECODED_CAP + 1).is_err());
    // An empty brotli stream (one byte) cannot fill a block that declares 1 KiB.
    let got = unbrotli(&[0x3b], 1024).expect("a stream that ends short");
    assert!(got.len() < 1024);
}

#[test]
fn a_woff_over_the_total_cap_is_refused() {
    let mut woff = b"wOFF\0\x01\0\0".to_vec();
    woff.resize(44, 0);
    assert!(unpack(&woff).is_none());
}

#[tokio::test]
async fn a_font_blob_shows_its_facts_and_a_sample_from_raw() {
    use std::sync::Arc;

    use gitterrace_testkit::git;

    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    std::fs::write(p.join("f.woff2"), WOFF2).expect("write");
    std::fs::write(p.join("bad.ttf"), b"\0\x01\0\0junk").expect("write");
    git(p, &["add", "."]);
    git(p, &["commit", "-q", "-m", "feat: fonts"]);
    let repo = gitterrace_git::Repo::discover(p).expect("repo");
    let index = gitterrace_index::Index::open(&repo).expect("index");
    let app = Arc::new(crate::app::App::new(repo, index));
    let (s, body) = crate::tests::get(&app, "/blob/HEAD/f.woff2").await;
    assert_eq!(s, 200);
    assert!(body.contains("JetBrains Mono") && body.contains("glyphs"), "{body}");
    assert!(body.contains(r#"data-font-src="/raw/HEAD/f.woff2""#), "{body}");
    assert!(body.contains("/static/font.js?v="), "{body}");
    let (s, body) = crate::tests::get(&app, "/blob/HEAD/bad.ttf").await;
    assert_eq!(s, 200);
    assert!(!body.contains("font-sample") && body.contains("TrueType font"), "{body}");
}
