//! `MANIFEST`: the only entry point of the cache.
//!
//! Names every live file
//! with its length and footer crc, the generation, the order fingerprint and the retired list.
//! Replaced by write, fsync, rename, directory sync, so a crash leaves the old one whole.

use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;

use crate::seg::bounds::{Cursor, MAX_MANIFEST, MAX_TOC};
use crate::seg::format::{FOOTER_LEN, VERSION};
use crate::seg::io::{read_exact_at, sync_dir};
use crate::seg::write::crashed;
use crate::seg::{SegError, SegResult};

pub const MANIFEST: &str = "MANIFEST";
const MAGIC: [u8; 4] = *b"GTM1";
const MAX_NAME: usize = 64;

/// One step of the write sequence, seen by a test hook after it happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    FileWritten(String),
    FileRenamed(String),
    ManifestWritten,
    ManifestRenamed,
    Retired(String),
}

/// A live file. `kind` is the container kind, or 0 for a plain file such as `lines.json`,
/// whose `footer_crc` is then the crc of the whole file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub kind: u8,
    pub name: String,
    pub len: u64,
    pub footer_crc: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Manifest {
    pub generation: u64,
    pub fingerprint: u64,
    pub commits: u32,
    /// The shallow set the order was built against, [`shallow_fingerprint`]. A deepened or
    /// grafted clone changes parents and so topo ranks: a different value means rebuild.
    pub shallow: u64,
    /// Live files in canonical order: segments and their string deltas, then order and others.
    pub live: Vec<FileEntry>,
    /// Superseded files whose delete has not succeeded yet.
    pub retired: Vec<String>,
}

/// Names a manifest may hold: `[a-z0-9.-]{1,64}`, no `..`, so never a path outside the dir.
pub(crate) fn check_name(name: &str) -> SegResult<()> {
    let ok = !name.is_empty()
        && name.len() <= MAX_NAME
        && !name.contains("..")
        && name != "."
        && name
            .bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'.' | b'-'));
    if ok {
        Ok(())
    } else {
        Err(SegError::BadName(name.to_owned()))
    }
}

fn put_name(b: &mut Vec<u8>, name: &str) -> SegResult<()> {
    check_name(name)?;
    b.push(u8::try_from(name.len()).map_err(|_| SegError::BadName(name.to_owned()))?);
    b.extend_from_slice(name.as_bytes());
    Ok(())
}

fn count(n: usize) -> SegResult<u32> {
    u32::try_from(n)
        .ok()
        .filter(|&n| n as usize <= MAX_TOC)
        .ok_or(SegError::TooLong {
            kind: "manifest list",
            len: n,
            max: MAX_TOC,
        })
}

impl Manifest {
    pub fn encode(&self) -> SegResult<Vec<u8>> {
        let mut b = Vec::new();
        b.extend_from_slice(&MAGIC);
        b.extend_from_slice(&VERSION.to_le_bytes());
        b.extend_from_slice(&self.generation.to_le_bytes());
        b.extend_from_slice(&self.fingerprint.to_le_bytes());
        b.extend_from_slice(&self.shallow.to_le_bytes());
        b.extend_from_slice(&self.commits.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&count(self.live.len())?.to_le_bytes());
        for f in &self.live {
            b.push(f.kind);
            put_name(&mut b, &f.name)?;
            b.extend_from_slice(&f.len.to_le_bytes());
            b.extend_from_slice(&f.footer_crc.to_le_bytes());
        }
        b.extend_from_slice(&count(self.retired.len())?.to_le_bytes());
        for r in &self.retired {
            put_name(&mut b, r)?;
        }
        let crc = crc32fast::hash(&b);
        b.extend_from_slice(&crc.to_le_bytes());
        Ok(b)
    }

    pub fn decode(bytes: &[u8]) -> SegResult<Self> {
        let bad = |what| SegError::unreadable(MANIFEST, 0, what);
        let (body, crc) = bytes
            .split_at_checked(bytes.len().saturating_sub(4))
            .ok_or_else(|| bad("short"))?;
        if bytes.len() < 4 || crc32fast::hash(body).to_le_bytes() != crc {
            return Err(bad("manifest crc"));
        }
        let mut c = Cursor::new(body, MANIFEST, 0);
        if c.array::<4>()? != MAGIC || c.u32()? != VERSION {
            return Err(bad("manifest magic or version"));
        }
        let generation = c.u64()?;
        let fingerprint = c.u64()?;
        let shallow = c.u64()?;
        let commits = c.u32()?;
        if c.u32()? != 0 {
            return Err(bad("manifest reserved field"));
        }
        let name = |c: &mut Cursor<'_>| -> SegResult<String> {
            let n = usize::from(c.u8()?);
            let s = std::str::from_utf8(c.bytes(n)?).map_err(|_| c.err("name not utf-8"))?;
            check_name(s).map_err(|_| c.err("bad file name"))?;
            Ok(s.to_owned())
        };
        let n = c.u32()? as usize;
        if n > MAX_TOC {
            return Err(c.err("live count over limit"));
        }
        let mut live = Vec::with_capacity(n);
        for _ in 0..n {
            let kind = c.u8()?;
            live.push(FileEntry {
                kind,
                name: name(&mut c)?,
                len: c.u64()?,
                footer_crc: c.u32()?,
            });
        }
        let n = c.u32()? as usize;
        if n > MAX_TOC {
            return Err(c.err("retired count over limit"));
        }
        let mut retired = Vec::with_capacity(n);
        for _ in 0..n {
            retired.push(name(&mut c)?);
        }
        if !c.is_empty() {
            return Err(c.err("trailing bytes"));
        }
        Ok(Self {
            generation,
            fingerprint,
            commits,
            shallow,
            live,
            retired,
        })
    }

    /// Reads `MANIFEST`. `Ok(None)` when there is none (a fresh cache).
    pub fn load(dir: &Path) -> SegResult<Option<Self>> {
        let file = match File::open(dir.join(MANIFEST)) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        if file.metadata()?.len() > MAX_MANIFEST {
            return Err(SegError::unreadable(MANIFEST, 0, "manifest over 1 MiB"));
        }
        let mut bytes = Vec::new();
        file.take(MAX_MANIFEST + 1).read_to_end(&mut bytes)?;
        Self::decode(&bytes).map(Some)
    }

    /// Checks every live file against its length and footer crc. Plain files are read whole.
    pub fn verify(&self, dir: &Path) -> SegResult<()> {
        for f in &self.live {
            let bad = |what| SegError::unreadable(&f.name, 0, what);
            let file = File::open(dir.join(&f.name)).map_err(|_| bad("named file missing"))?;
            if file.metadata()?.len() != f.len {
                return Err(bad("length differs from manifest"));
            }
            let crc = if f.kind == 0 {
                let mut h = crc32fast::Hasher::new();
                let mut buf = vec![0; 64 * 1024];
                let mut r = io::BufReader::new(file);
                loop {
                    let n = r.read(&mut buf)?;
                    if n == 0 {
                        break;
                    }
                    h.update(&buf[..n]);
                }
                h.finalize()
            } else {
                if f.len < FOOTER_LEN {
                    return Err(bad("shorter than a footer"));
                }
                let mut b = [0; FOOTER_LEN as usize];
                read_exact_at(&file, &mut b, f.len - FOOTER_LEN).map_err(|_| bad("footer read"))?;
                crc32fast::hash(&b)
            };
            if crc != f.footer_crc {
                return Err(bad("footer crc differs from manifest"));
            }
        }
        Ok(())
    }

    /// Replaces `MANIFEST` with `self`, then deletes retired files. Names that fail to delete
    /// stay in `retired` (Windows, still open by a reader) and are retried next time.
    pub fn commit(&mut self, dir: &Path) -> SegResult<()> {
        self.commit_with(dir, &mut |_| true)
    }

    /// As `commit`, with a hook after each step that stops the sequence by returning false.
    pub fn commit_with(&mut self, dir: &Path, hook: &mut dyn FnMut(Step) -> bool) -> SegResult<()> {
        let live: Vec<&str> = self.live.iter().map(|f| f.name.as_str()).collect();
        self.retired.retain(|r| !live.contains(&r.as_str()));
        self.retired.sort();
        self.retired.dedup();
        let tmp = dir.join(format!("{MANIFEST}.tmp"));
        {
            let mut f = File::create(&tmp)?;
            io::Write::write_all(&mut f, &self.encode()?)?;
            f.sync_all()?;
        }
        if !hook(Step::ManifestWritten) {
            return Err(crashed());
        }
        fs::rename(&tmp, dir.join(MANIFEST))?;
        sync_dir(dir)?;
        if !hook(Step::ManifestRenamed) {
            return Err(crashed());
        }
        self.sweep_retired(dir, hook)
    }

    /// Deletes retired files, keeping the ones whose delete failed. A missing file is done.
    /// Rewrites the manifest only when the list changed.
    fn sweep_retired(&mut self, dir: &Path, hook: &mut dyn FnMut(Step) -> bool) -> SegResult<()> {
        let before = self.retired.len();
        let mut kept = Vec::new();
        for name in std::mem::take(&mut self.retired) {
            match fs::remove_file(dir.join(&name)) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(_) => {
                    kept.push(name);
                    continue;
                }
            }
            if !hook(Step::Retired(name)) {
                return Err(crashed());
            }
        }
        self.retired = kept;
        if self.retired.len() != before {
            crate::seg::io::write_atomic(dir, MANIFEST, &self.encode()?)?;
        }
        Ok(())
    }
}

/// The shallow set as the manifest stores it: FNV-1a 64 over the sorted boundary oids, 0 when
/// the clone is not shallow (and at least 1 when it is, so deepening to full always differs).
pub fn shallow_fingerprint(oids: &[crate::seg::Oid]) -> u64 {
    if oids.is_empty() {
        return 0;
    }
    let mut sorted = oids.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    crate::seg::fingerprint(&sorted).max(1)
}

/// Files in `dir` that are container names (`.gts`, `.tmp`) the manifest does not name: left by
/// a crash before a rename. The caller removes them through the safe cache remove.
pub fn strays(dir: &Path, manifest: Option<&Manifest>) -> SegResult<Vec<String>> {
    let mut out = Vec::new();
    for e in fs::read_dir(dir)? {
        let name = e?.file_name();
        let Some(name) = name.to_str() else { continue };
        let ours = is_gts(name) || name == "MANIFEST.tmp";
        let live = manifest.is_some_and(|m| m.live.iter().any(|f| f.name == name));
        // MANIFEST.tmp is uppercase, which check_name refuses, so it is named explicitly.
        let sweepable = name == "MANIFEST.tmp" || check_name(name).is_ok();
        if ours && !live && sweepable {
            out.push(name.to_owned());
        }
    }
    out.sort();
    Ok(out)
}

/// A container or its temp name. Case-sensitive on purpose: every name the cache writes is
/// lowercase (`check_name`), so `X.GTS` is not ours and the sweep must leave it.
#[expect(
    clippy::case_sensitive_file_extension_comparisons,
    reason = "cache names are lowercase by check_name, another case is not ours"
)]
fn is_gts(name: &str) -> bool {
    name.ends_with(".gts") || name.ends_with(".gts.tmp")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Manifest {
        Manifest {
            generation: 3,
            fingerprint: 0xdead_beef,
            commits: 42,
            shallow: 7,
            live: vec![FileEntry {
                kind: 1,
                name: "seg-3-0.gts".into(),
                len: 100,
                footer_crc: 9,
            }],
            retired: vec!["seg-2-0.gts".into()],
        }
    }

    #[test]
    fn the_shallow_fingerprint_round_trips_and_tracks_the_set() -> SegResult<()> {
        let m = sample();
        assert_eq!(Manifest::decode(&m.encode()?)?.shallow, 7);
        assert_eq!(shallow_fingerprint(&[]), 0);
        let a = shallow_fingerprint(&[[1; 20], [2; 20]]);
        assert_eq!(a, shallow_fingerprint(&[[2; 20], [1; 20]]));
        assert_ne!(a, 0);
        assert_ne!(a, shallow_fingerprint(&[[1; 20]]));
        Ok(())
    }

    #[test]
    fn round_trips() -> SegResult<()> {
        let m = sample();
        assert_eq!(Manifest::decode(&m.encode()?)?, m);
        Ok(())
    }

    #[test]
    fn every_flip_and_cut_is_unreadable() -> SegResult<()> {
        let b = sample().encode()?;
        for i in 0..b.len() {
            let mut x = b.clone();
            x[i] ^= 1;
            assert!(
                Manifest::decode(&x).is_err_and(|e| e.is_unreadable()),
                "flip {i}"
            );
        }
        for n in 0..b.len() {
            assert!(
                Manifest::decode(&b[..n]).is_err_and(|e| e.is_unreadable()),
                "cut {n}"
            );
        }
        Ok(())
    }

    #[test]
    fn hostile_counts_and_names_with_valid_crc() {
        let seal = |mut b: Vec<u8>| {
            let crc = crc32fast::hash(&b);
            b.extend_from_slice(&crc.to_le_bytes());
            b
        };
        let mut head = Vec::new();
        head.extend_from_slice(&MAGIC);
        head.extend_from_slice(&VERSION.to_le_bytes());
        // generation, order fingerprint, shallow fingerprint, commit count, zero.
        head.extend_from_slice(&[0; 32]);
        let mut huge = head.clone();
        huge.extend_from_slice(&u32::MAX.to_le_bytes());
        assert!(Manifest::decode(&seal(huge)).is_err_and(|e| e.is_unreadable()));
        let mut escape = head;
        escape.extend_from_slice(&1u32.to_le_bytes());
        escape.push(1);
        escape.push(5);
        escape.extend_from_slice(b"../xx");
        escape.extend_from_slice(&[0; 12]);
        escape.extend_from_slice(&0u32.to_le_bytes());
        assert!(Manifest::decode(&seal(escape)).is_err_and(|e| e.is_unreadable()));
        for n in ["", "..", "a/b", "A", "x".repeat(65).as_str()] {
            assert!(check_name(n).is_err(), "{n}");
        }
    }

    #[test]
    fn crash_at_each_step_keeps_a_whole_manifest() -> SegResult<()> {
        for stop in 0..4 {
            let dir = tempfile::tempdir()?;
            let mut old = Manifest {
                generation: 1,
                ..Manifest::default()
            };
            old.commit(dir.path())?;
            fs::write(dir.path().join("seg-2-0.gts"), b"x")?;
            let mut new = sample();
            let mut n = 0;
            let r = new.commit_with(dir.path(), &mut |_| {
                n += 1;
                n <= stop
            });
            let loaded = Manifest::load(dir.path())?;
            let g = loaded.map(|m| m.generation);
            if stop == 0 {
                assert!(r.is_err());
                assert_eq!(g, Some(1));
            } else {
                assert_eq!(g, Some(3), "stop {stop}");
            }
        }
        Ok(())
    }

    #[test]
    fn retired_delete_and_strays() -> SegResult<()> {
        let dir = tempfile::tempdir()?;
        fs::write(dir.path().join("seg-2-0.gts"), b"x")?;
        fs::write(dir.path().join("seg-9-0.gts.tmp"), b"x")?;
        let mut m = sample();
        m.live.clear();
        m.commit(dir.path())?;
        let mut stop = |s: Step| s != Step::ManifestWritten;
        assert!(m.clone().commit_with(dir.path(), &mut stop).is_err());
        assert!(m.retired.is_empty());
        assert!(!dir.path().join("seg-2-0.gts").exists());
        assert_eq!(
            Manifest::load(dir.path())?.map(|m| m.retired.len()),
            Some(0)
        );
        assert_eq!(
            strays(dir.path(), Some(&m))?,
            ["MANIFEST.tmp", "seg-9-0.gts.tmp"]
        );
        Ok(())
    }

    #[test]
    fn verify_catches_length_and_footer() -> SegResult<()> {
        let dir = tempfile::tempdir()?;
        let body = vec![7u8; 100];
        fs::write(dir.path().join("seg-3-0.gts"), &body)?;
        let mut m = sample();
        m.live[0].footer_crc = crc32fast::hash(&body[36..]);
        m.verify(dir.path())?;
        m.live[0].len = 99;
        assert!(m.verify(dir.path()).is_err_and(|e| e.is_unreadable()));
        fs::remove_file(dir.path().join("seg-3-0.gts"))?;
        assert!(
            sample()
                .verify(dir.path())
                .is_err_and(|e| e.is_unreadable())
        );
        Ok(())
    }
}
