//! zip, jar, tar and tar.gz blobs as a list of their entries: name, size and date. Only the
//! directory is read, nothing is extracted, and the count is capped, so a zip bomb costs no
//! more than its central directory and a tar.gz no more than one pass of inflate.

use std::io::{Cursor, Read};

use askama::Template;

/// Entries past this many are counted out, not listed.
pub(crate) const ENTRY_CAP: usize = 1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Zip,
    Tar,
    TarGz,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Entry {
    pub(crate) name: String,
    pub(crate) size: String,
    /// `YYYY-MM-DD HH:MM`, empty when the archive holds none.
    pub(crate) date: String,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Listing {
    pub(crate) entries: Vec<Entry>,
    /// More entries exist past the cap.
    pub(crate) more: bool,
}

#[derive(Template)]
#[template(path = "archive.html")]
struct ListingMarkup<'a> {
    name: &'a str,
    l: &'a Listing,
    cap: usize,
}

/// The archive kind of `path`, by extension.
pub(crate) fn kind(path: &str) -> Option<Kind> {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".tar.gz") {
        return Some(Kind::TarGz);
    }
    match lower.rsplit_once('.')?.1 {
        "tgz" => Some(Kind::TarGz),
        "zip" | "jar" => Some(Kind::Zip),
        "tar" => Some(Kind::Tar),
        _ => None,
    }
}

/// The first `cap` entries of the archive in `bytes`, or `None` when it does not read.
pub(crate) fn list(kind: Kind, bytes: &[u8], cap: usize) -> Option<Listing> {
    match kind {
        Kind::Zip => zip_list(bytes, cap),
        Kind::Tar => tar_list(bytes, cap),
        Kind::TarGz => tar_list(flate2::read::GzDecoder::new(bytes), cap),
    }
}

fn zip_list(bytes: &[u8], cap: usize) -> Option<Listing> {
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).ok()?;
    let n = z.len();
    let mut entries = Vec::with_capacity(n.min(cap));
    for i in 0..n.min(cap) {
        // `by_index_raw`: the header only, never a decompressor.
        let f = z.by_index_raw(i).ok()?;
        let date = f.last_modified().map_or_else(String::new, |d| {
            format!(
                "{:04}-{:02}-{:02} {:02}:{:02}",
                d.year(),
                d.month(),
                d.day(),
                d.hour(),
                d.minute()
            )
        });
        entries.push(Entry {
            name: f.name().ok()?.into_owned(),
            size: crate::fmt::size(f.size()),
            date,
        });
    }
    Some(Listing {
        entries,
        more: n > cap,
    })
}

fn tar_list(r: impl Read, cap: usize) -> Option<Listing> {
    let mut a = tar::Archive::new(r);
    let mut entries = Vec::new();
    let mut more = false;
    for e in a.entries().ok()? {
        let e = e.ok()?;
        if entries.len() == cap {
            more = true;
            break;
        }
        let h = e.header();
        let date = h
            .mtime()
            .ok()
            .and_then(|t| i64::try_from(t).ok())
            .map_or_else(String::new, |t| crate::fmt::date(t, 0));
        entries.push(Entry {
            name: String::from_utf8_lossy(&e.path_bytes()).into_owned(),
            size: crate::fmt::size(e.size()),
            date,
        });
    }
    Some(Listing { entries, more })
}

/// The listing's markup, every name escaped by the template.
pub(crate) fn html(name: &str, l: &Listing) -> String {
    ListingMarkup {
        name,
        l,
        cap: ENTRY_CAP,
    }
    .render()
    .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::*;

    fn zip_of(n: usize) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for i in 0..n {
            w.start_file(
                format!("&{i}<b.txt"),
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored),
            )
            .expect("start");
            w.write_all(b"hello").expect("write");
        }
        w.finish().expect("finish").into_inner()
    }

    fn tar_of(names: &[&str]) -> Vec<u8> {
        let mut b = tar::Builder::new(Vec::new());
        for n in names {
            let mut h = tar::Header::new_gnu();
            h.set_size(3);
            h.set_mtime(86_400);
            h.set_cksum();
            b.append_data(&mut h, n, &b"abc"[..]).expect("append");
        }
        b.into_inner().expect("tar")
    }

    #[test]
    fn a_zip_lists_names_and_sizes() {
        let l = list(Kind::Zip, &zip_of(2), 10).expect("zip");
        assert_eq!(l.entries.len(), 2);
        assert_eq!(l.entries[0].name, "&0<b.txt");
        assert_eq!(l.entries[0].size, crate::fmt::size(5));
        assert!(!l.more);
    }

    #[test]
    fn a_zip_past_the_cap_lists_the_cap_and_says_more() {
        let l = list(Kind::Zip, &zip_of(5), 3).expect("zip");
        assert_eq!(l.entries.len(), 3);
        assert!(l.more);
    }

    #[test]
    fn a_tar_gz_lists_its_entries_with_dates() {
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(&tar_of(&["a.txt", "d/b.txt"])).expect("gz");
        let l = list(Kind::TarGz, &gz.finish().expect("gz"), 10).expect("tgz");
        let names: Vec<_> = l.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["a.txt", "d/b.txt"]);
        assert_eq!(l.entries[0].date, "1970-01-02 00:00");
    }

    #[test]
    fn a_tar_past_the_cap_says_more() {
        let l = list(Kind::Tar, &tar_of(&["a", "b", "c"]), 2).expect("tar");
        assert_eq!(l.entries.len(), 2);
        assert!(l.more);
    }

    #[test]
    fn garbage_lists_nothing_and_never_panics() {
        assert!(list(Kind::Zip, b"PK\x03\x04junk", 10).is_none());
        assert!(list(Kind::TarGz, b"\x1f\x8bjunk", 10).is_none());
    }

    #[test]
    fn names_are_escaped() {
        let l = list(Kind::Zip, &zip_of(1), 10).expect("zip");
        let h = html("x.zip", &l);
        assert!(!h.contains("0<b") && h.contains("&#38;0&#60;b.txt"));
    }

    #[test]
    fn kinds_by_extension() {
        assert_eq!(kind("a/x.JAR"), Some(Kind::Zip));
        assert_eq!(kind("x.tar.gz"), Some(Kind::TarGz));
        assert_eq!(kind("x.tgz"), Some(Kind::TarGz));
        assert_eq!(kind("x.tar"), Some(Kind::Tar));
        assert_eq!(kind("x.gz"), None);
    }
}
