//! zip, jar, tar and tar.gz blobs as a list of their entries: name, size and date. Only the
//! directory is read, nothing is extracted, and the count is capped, so a zip bomb costs no
//! more than its central directory and a tar.gz no more than `INFLATE_CAP` inflated bytes,
//! counted by [`Bounded`], which also stops when the request that asked is gone.

use std::io::{self, Cursor, Read};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use askama::Template;

/// Entries past this many are counted out, not listed. Not configurable: a read bound on a
/// file from the repository, which may be hostile.
pub(crate) const ENTRY_CAP: usize = 1000;

/// Inflated bytes a tar.gz listing may read, headers and skipped members together. Not
/// configurable, as `ENTRY_CAP`: a hostile archive inflates a thousandfold.
pub(crate) const INFLATE_CAP: u64 = 64 << 20;

/// What one listing may spend: inflated bytes, and a flag the request raises when it is gone.
pub(crate) struct Work {
    cap: u64,
    cancel: Arc<AtomicBool>,
    inflated: Arc<AtomicU64>,
    stopped: Arc<AtomicBool>,
}

impl Work {
    pub(crate) fn new(cap: u64, cancel: Arc<AtomicBool>) -> Self {
        Self { cap, cancel, inflated: Arc::default(), stopped: Arc::default() }
    }
}

/// A reader that counts what it hands out and fails at the cap or once `cancel` is raised.
/// Every inflated byte a tar walk skips passes through here, so no member size, declared or
/// real, can make the walk longer than the cap.
struct Bounded<R> {
    inner: R,
    work: Arc<Work>,
}

impl<R: Read> Read for Bounded<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let w = &self.work;
        if w.cancel.load(Ordering::Relaxed) {
            w.stopped.store(true, Ordering::Relaxed);
            return Err(io::Error::other("preview cancelled"));
        }
        let left = w.cap.saturating_sub(w.inflated.load(Ordering::Relaxed));
        if left == 0 {
            // At the cap exactly: only more data is over it, the end of the stream is not.
            let mut probe = [0u8; 1];
            if self.inner.read(&mut probe)? == 0 {
                return Ok(0);
            }
            w.stopped.store(true, Ordering::Relaxed);
            return Err(io::Error::other("inflate cap reached"));
        }
        let take = buf.len().min(usize::try_from(left).unwrap_or(usize::MAX));
        let n = self.inner.read(&mut buf[..take])?;
        w.inflated.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
}

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
    pub(crate) more: bool,
    /// The walk stopped at the inflate cap or on cancel: the entries are the ones read so far.
    pub(crate) truncated: bool,
}

#[derive(Template)]
#[template(path = "archive.html")]
struct ListingMarkup<'a> {
    name: &'a str,
    l: &'a Listing,
    cap: usize,
}

pub(crate) fn kind(path: &str) -> Option<Kind> {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".tar.gz") {
        return Some(Kind::TarGz);
    }
    match crate::fmt::ext(path)?.as_str() {
        "tgz" => Some(Kind::TarGz),
        "zip" | "jar" => Some(Kind::Zip),
        "tar" => Some(Kind::Tar),
        _ => None,
    }
}

/// The first `cap` entries of the archive in `bytes`, or `None` when it does not read.
pub(crate) fn list(kind: Kind, bytes: &[u8], cap: usize) -> Option<Listing> {
    list_within(kind, bytes, cap, Work::new(INFLATE_CAP, crate::app::cancel_flag()))
}

/// [`list`] under `work`.
pub(crate) fn list_within(kind: Kind, bytes: &[u8], cap: usize, work: Work) -> Option<Listing> {
    match kind {
        Kind::Zip => zip_list(bytes, cap),
        Kind::Tar => tar_list(bytes, cap, None),
        Kind::TarGz => {
            let work = Arc::new(work);
            let r = Bounded { inner: flate2::read::GzDecoder::new(bytes), work: Arc::clone(&work) };
            tar_list(r, cap, Some(&work))
        }
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
    Some(Listing { entries, more: n > cap, truncated: false })
}

fn tar_list(r: impl Read, cap: usize, work: Option<&Work>) -> Option<Listing> {
    let stopped = || work.is_some_and(|w| w.stopped.load(Ordering::Relaxed));
    let mut a = tar::Archive::new(r);
    let mut entries = Vec::new();
    let mut more = false;
    let mut truncated = false;
    let it = match a.entries() {
        Ok(it) => it,
        Err(_) if stopped() => return Some(Listing { entries, more, truncated: true }),
        Err(_) => return None,
    };
    for e in it {
        let e = match e {
            Ok(e) => e,
            Err(_) if stopped() => {
                truncated = true;
                break;
            }
            Err(_) => return None,
        };
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
    Some(Listing { entries, more, truncated })
}

/// The listing's markup, every name escaped by the template.
pub(crate) fn html(name: &str, l: &Listing) -> String {
    ListingMarkup { name, l, cap: ENTRY_CAP }.render().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use proptest::prelude::*;

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

    fn gz(b: &[u8]) -> Vec<u8> {
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        e.write_all(b).expect("gz");
        e.finish().expect("gz")
    }

    /// A tar of one member declaring `declared` bytes, holding `real` zero bytes, gzipped.
    fn zero_member_tgz(declared: u64, real: usize) -> Vec<u8> {
        let mut h = tar::Header::new_gnu();
        h.set_path("big.bin").expect("path");
        h.set_size(declared);
        h.set_cksum();
        let mut t = h.as_bytes().to_vec();
        t.resize(t.len() + real, 0);
        gz(&t)
    }

    fn work(cap: u64) -> (Work, Arc<AtomicU64>) {
        let w = Work::new(cap, Arc::default());
        let n = Arc::clone(&w.inflated);
        (w, n)
    }

    #[test]
    fn a_gzip_bomb_inflates_no_more_than_the_cap() {
        let bomb = zero_member_tgz(8 << 20, 8 << 20);
        let (w, n) = work(1 << 20);
        let l = list_within(Kind::TarGz, &bomb, 10, w).expect("listing");
        assert!(l.truncated);
        assert_eq!(n.load(Ordering::Relaxed), 1 << 20);
    }

    #[test]
    fn a_huge_declared_member_is_cut_at_the_cap() {
        let bomb = zero_member_tgz(1 << 40, 4 << 20);
        let (w, n) = work(1 << 20);
        let l = list_within(Kind::TarGz, &bomb, 10, w).expect("listing");
        assert!(l.truncated);
        assert_eq!(n.load(Ordering::Relaxed), 1 << 20);
    }

    #[test]
    fn an_archive_inflating_exactly_to_the_cap_lists_whole() {
        let tar = tar_of(&["a.txt", "d/b.txt"]);
        let (w, n) = work(tar.len() as u64);
        let l = list_within(Kind::TarGz, &gz(&tar), 10, w).expect("listing");
        assert!(!l.truncated);
        assert_eq!(l.entries.len(), 2);
        assert!(n.load(Ordering::Relaxed) <= tar.len() as u64);
    }

    #[test]
    fn a_raised_cancel_flag_stops_the_walk_before_it_inflates() {
        let (w, n) = work(INFLATE_CAP);
        w.cancel.store(true, Ordering::Relaxed);
        let l = list_within(Kind::TarGz, &zero_member_tgz(8 << 20, 8 << 20), 10, w);
        assert!(l.is_none_or(|l| l.truncated));
        assert_eq!(n.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn a_flag_raised_mid_walk_stops_the_count_where_it_was() {
        let (w, n) = work(INFLATE_CAP);
        let cancel = Arc::clone(&w.cancel);
        let w = Arc::new(w);
        let tgz = zero_member_tgz(0, 4 << 20);
        let mut z = Bounded { inner: flate2::read::GzDecoder::new(&tgz[..]), work: Arc::clone(&w) };
        let mut buf = [0u8; 4096];
        z.read_exact(&mut buf).expect("first read");
        cancel.store(true, Ordering::Relaxed);
        let at = n.load(Ordering::Relaxed);
        while z.read(&mut buf).is_ok_and(|k| k > 0) {}
        assert!(w.stopped.load(Ordering::Relaxed));
        assert!(n.load(Ordering::Relaxed) <= at + 64 * 1024);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(128))]

        /// Bytes mutated or cut from a valid archive of any kind never panic, and a listing
        /// that reads holds no more than the cap.
        #[test]
        fn mutated_archives_list_within_the_cap_or_not_at_all(
            flips in prop::collection::vec((any::<usize>(), any::<u8>()), 0..8),
            cut in any::<usize>(),
            k in 0u8..3,
        ) {
            let names = ["a", "b/c.txt", "d", "e", "f"];
            let (kind, mut b) = match k {
                0 => (Kind::Zip, zip_of(5)),
                1 => (Kind::Tar, tar_of(&names)),
                _ => (Kind::TarGz, gz(&tar_of(&names))),
            };
            for (at, v) in flips {
                let n = b.len();
                b[at % n] = v;
            }
            b.truncate(cut % (b.len() + 1));
            if let Some(l) = list(kind, &b, 3) {
                prop_assert!(l.entries.len() <= 3);
            }
        }
    }
}
