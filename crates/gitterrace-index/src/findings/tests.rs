//! Unit tests for the review basket.

use std::io;

use super::{FILE, FIXED, MAGIC, MAX_NOTE, OP_ADD, StoredFinding, cut_note, scan};
use crate::Index;

fn open() -> io::Result<(tempfile::TempDir, Index)> {
    let tmp = tempfile::tempdir()?;
    let idx = Index::open_dir(tmp.path().join("gitterrace"))?;
    Ok((tmp, idx))
}

fn finding(note: &str, added: i64) -> StoredFinding {
    StoredFinding {
        id: String::new(),
        kind: "line".into(),
        rev: "main".into(),
        sha: "a".repeat(40),
        path: Some("src/a.rs".into()),
        lines: Some((10, 12)),
        query: None,
        note: note.into(),
        added,
    }
    .sealed()
}

/// A frame around `payload` with a valid checksum.
fn frame(payload: &[u8]) -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    out.extend((payload.len() as u32).to_le_bytes());
    out.extend(crc32fast::hash(payload).to_le_bytes());
    out.extend(payload);
    out
}

#[test]
fn a_finding_round_trips_with_its_id_and_order() -> io::Result<()> {
    let (_t, idx) = open()?;
    assert_eq!(idx.findings_list()?.len(), 0);
    let (a, b) = (finding("first", 1), finding("second", 2));
    assert!(a.id.starts_with("f-") && a.id.len() == 10);
    assert!(idx.findings_add(&a, 500)?);
    assert!(idx.findings_add(&b, 500)?);
    assert!(!idx.findings_add(&a, 500)?, "the same finding twice");
    assert_eq!(idx.findings_list()?, [a, b]);
    Ok(())
}

#[test]
fn the_cap_drops_the_oldest_entries() -> io::Result<()> {
    let (_t, idx) = open()?;
    for i in 0..5 {
        idx.findings_add(&finding("n", i), 3)?;
    }
    let added: Vec<i64> = idx.findings_list()?.iter().map(|f| f.added).collect();
    assert_eq!(added, [2, 3, 4]);
    Ok(())
}

#[test]
fn remove_takes_the_named_ids_or_all() -> io::Result<()> {
    let (_t, idx) = open()?;
    let all: Vec<_> = (0..3).map(|i| finding("n", i)).collect();
    for f in &all {
        idx.findings_add(f, 500)?;
    }
    assert_eq!(idx.findings_remove(&[all[1].id.clone(), "f-nope".into()])?, 1);
    assert_eq!(idx.findings_list()?.len(), 2);
    assert_eq!(idx.findings_remove(&[])?, 2);
    assert_eq!(idx.findings_list()?.len(), 0);
    Ok(())
}

#[test]
fn a_note_is_cut_on_a_character_boundary_and_stripped_of_control_bytes() {
    let long = "é".repeat(MAX_NOTE);
    let cut = cut_note(&long);
    assert!(cut.len() <= MAX_NOTE && cut.len() > MAX_NOTE - 2);
    assert_eq!(cut_note("a\u{1b}[31mb\nc\t"), "a[31mb\nc\t");
    assert!(finding(&long, 1).note.len() <= MAX_NOTE);
}

#[test]
fn a_torn_tail_is_ignored_and_the_next_write_replaces_it() -> io::Result<()> {
    let (_t, idx) = open()?;
    let (a, b) = (finding("a", 1), finding("b", 2));
    idx.findings_add(&a, 500)?;
    let path = idx.dir().join(FILE);
    let mut bytes = std::fs::read(&path)?;
    bytes.extend_from_slice(&super::encode(&b)[..20]);
    std::fs::write(&path, &bytes)?;
    assert_eq!(idx.findings_list()?, std::slice::from_ref(&a));
    idx.findings_add(&b, 500)?;
    assert_eq!(idx.findings_list()?, [a, b]);
    Ok(())
}

#[test]
fn a_corrupt_checksum_stops_the_scan_without_an_error() -> io::Result<()> {
    let (_t, idx) = open()?;
    idx.findings_add(&finding("a", 1), 500)?;
    let path = idx.dir().join(FILE);
    let mut bytes = std::fs::read(&path)?;
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    std::fs::write(&path, &bytes)?;
    assert_eq!(idx.findings_list()?.len(), 0);
    Ok(())
}

/// Class guard: a length field that lies sizes no buffer. A frame with a valid checksum whose
/// payload claims a huge string, or a huge count of bytes, is an error and not an abort or an
/// allocation, through the same bounded cursor the index decoders use.
#[test]
fn a_huge_length_with_a_valid_checksum_is_an_error_not_an_abort() -> io::Result<()> {
    let (_t, idx) = open()?;
    let varint = |v: u64| {
        let mut b = Vec::new();
        crate::seg::bounds::put_varint(&mut b, v);
        b
    };
    for claim in [u64::MAX, u64::from(u32::MAX), 1 << 40, 70_000] {
        let mut payload = vec![OP_ADD];
        payload.extend(varint(claim));
        payload.extend(b"short");
        std::fs::write(idx.dir().join(FILE), frame(&payload))?;
        let err = idx.findings_list().expect_err("a lying length must not decode");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData, "{claim}");
    }
    // A frame header that claims more than the bound is a torn tail, not an allocation.
    let mut head = MAGIC.to_vec();
    head.extend(u32::MAX.to_le_bytes());
    head.extend(0u32.to_le_bytes());
    assert_eq!(scan(&head)?.1, 0);
    assert_eq!(head.len(), FIXED);
    Ok(())
}

#[test]
fn an_op_from_a_newer_build_is_an_error_and_the_file_is_left_alone() -> io::Result<()> {
    let (_t, idx) = open()?;
    let bytes = frame(&[9, 0, 0]);
    std::fs::write(idx.dir().join(FILE), &bytes)?;
    assert!(idx.findings_list().is_err());
    assert!(idx.findings_add(&finding("a", 1), 500).is_err());
    assert_eq!(std::fs::read(idx.dir().join(FILE))?, bytes);
    Ok(())
}
