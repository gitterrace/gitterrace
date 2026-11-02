//! Shared by `unreadable.rs` and `hostile.rs`: a small valid segment store written through the
//! public writer, so a test corrupts real files rather than hand-made bytes.
#![allow(dead_code, reason = "each test binary uses a part")]

use std::path::Path;

use gitterrace_index::seg::format::{Kind, StringBase, section};
use gitterrace_index::seg::manifest::{FileEntry, Manifest, Step};
use gitterrace_index::seg::strings::{Interner, write_delta};
use gitterrace_index::seg::write::{ContainerWriter, write_order};
use gitterrace_index::seg::{SegResult, fingerprint};

pub const OIDS: [[u8; 20]; 3] = [[1; 20], [2; 20], [3; 20]];

/// Writes strings, one segment, the order file and the manifest into `dir`. `hook` sees each
/// step and stops the sequence by returning false (a crash right after that step).
pub fn write_store(dir: &Path, hook: &mut dyn FnMut(Step) -> bool) -> SegResult<Manifest> {
    let mut i = Interner::new(StringBase::default());
    for p in ["src/a.rs", "src/b.rs", "docs/x.md"] {
        i.path(p)?;
    }
    i.identity("Jane Doe <jane@x.io>")?;
    i.label("fix")?;
    let (strings, _) = write_delta(dir, "str-1-0.gts", StringBase::default(), &i.into_delta())?;
    let mut w = ContainerWriter::new(dir, "seg-1-0.gts", Kind::Segment, StringBase::default())?;
    // Three blocks over two sections, so a flip can land in any of them.
    w.block(section::COMMITS, 3, &[7; 300])?;
    w.block(section::FILE_ROWS, 2, &[9; 200])?;
    w.block(section::FILE_ROWS, 1, &[5; 100])?;
    let seg = w.finish_with(hook)?;
    let order = write_order(dir, "order-1.gts", &OIDS)?;
    let mut m = Manifest {
        generation: 1,
        fingerprint: fingerprint(OIDS.iter()),
        commits: 3,
        shallow: 0,
        live: vec![strings, seg, order],
        retired: Vec::new(),
    };
    m.commit_with(dir, hook)?;
    Ok(m)
}

/// A store written whole.
pub fn valid(dir: &Path) -> Manifest {
    write_store(dir, &mut |_| true).expect("write store")
}

/// The live entry named `name`.
pub fn entry<'a>(m: &'a Manifest, name: &str) -> &'a FileEntry {
    m.live.iter().find(|f| f.name == name).expect("live entry")
}
