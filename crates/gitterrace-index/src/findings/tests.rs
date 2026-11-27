//! Unit tests for the review basket.

use std::io;

use super::{
    FILE, FIXED, MAGIC, MAX_NOTE, OP_ADD, StoredFinding, clean_note, cut_note, hash_budget, scan,
};
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
    let found = scan(&head)?;
    assert!(found.items.is_empty() && found.damaged == 1);
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

/// Three findings with the middle frame's checksum byte flipped, as the bytes of the file.
fn with_damaged_middle(idx: &Index) -> io::Result<(Vec<StoredFinding>, Vec<u8>)> {
    let all: Vec<_> = (0..3).map(|i| finding(&format!("n{i}"), i)).collect();
    for f in &all {
        idx.findings_add(f, 500)?;
    }
    let path = idx.dir().join(FILE);
    let mut bytes = std::fs::read(&path)?;
    let second = super::encode(&all[0]).len() + FIXED;
    bytes[second] ^= 0xff;
    std::fs::write(&path, &bytes)?;
    Ok((all, bytes))
}

#[test]
fn a_damaged_middle_frame_is_skipped_and_the_later_entries_are_listed() -> io::Result<()> {
    let (_t, idx) = open()?;
    let (all, _) = with_damaged_middle(&idx)?;
    assert_eq!(idx.findings_list()?, [all[0].clone(), all[2].clone()]);
    Ok(())
}

/// Class guard: no write may rewrite a file from a partial read. It would also catch a frame of
/// an unknown magic or a lying length in the middle, which scan reports the same way.
#[test]
fn a_damaged_frame_keeps_every_byte_through_add_remove_and_a_cap_trim() -> io::Result<()> {
    let (_t, idx) = open()?;
    let (all, bytes) = with_damaged_middle(&idx)?;
    let path = idx.dir().join(FILE);
    assert!(idx.findings_remove(&[all[0].id.clone()]).is_err());
    assert!(idx.findings_remove(&[]).is_err());
    assert!(idx.findings_add(&finding("x", 9), 2).is_err(), "a cap trim would rewrite");
    assert_eq!(std::fs::read(&path)?, bytes);
    let extra = finding("four", 4);
    assert!(idx.findings_add(&extra, 500)?, "an append keeps the old bytes");
    let after = std::fs::read(&path)?;
    assert!(after.starts_with(&bytes));
    assert_eq!(idx.findings_list()?, [all[0].clone(), all[2].clone(), extra]);
    Ok(())
}

#[test]
fn garbage_between_frames_is_skipped_to_the_next_whole_frame() {
    let (a, b) = (super::encode(&finding("a", 1)), super::encode(&finding("b", 2)));
    let bytes = [a, b"junk-junk-junk-junk".to_vec(), b].concat();
    let found = scan(&bytes).expect("scan");
    assert_eq!((found.items.len(), found.damaged, found.valid), (2, 1, bytes.len()));
}

/// A file of one valid-looking header repeated, each claiming the largest frame with a bad
/// checksum: every offset is a plausible candidate overlapping the others.
fn planted_headers(len: usize) -> Vec<u8> {
    let mut head = MAGIC.to_vec();
    head.extend((super::MAX_FRAME as u32).to_le_bytes());
    head.extend(0xdead_beef_u32.to_le_bytes());
    head.iter().copied().cycle().take(len).collect()
}

/// Class guard: the checksum work of a pass is bounded by the file size, whatever the density of
/// candidates. It measures hashed bytes, not time, and would catch any resync that hashes per
/// candidate without a shared budget.
#[test]
fn planted_headers_cost_a_bounded_checksum_work_and_report_incomplete() {
    for len in [1 << 20, 4 << 20] {
        let bytes = planted_headers(len);
        let found = scan(&bytes).expect("scan");
        assert!(found.incomplete, "the budget must run out");
        assert!(found.damaged >= 1);
        assert!(found.hashed <= hash_budget(len), "hashed {} of {len}", found.hashed);
        assert!(found.valid < bytes.len());
    }
}

/// Class guard: an unread rest is never rewritten or extended, so every original byte stays
/// through add and remove.
#[test]
fn an_incomplete_scan_keeps_every_byte_through_add_and_remove() -> io::Result<()> {
    let (_t, idx) = open()?;
    idx.findings_add(&finding("a", 1), 500)?;
    let path = idx.dir().join(FILE);
    let mut bytes = std::fs::read(&path)?;
    bytes.extend(planted_headers(1 << 20));
    std::fs::write(&path, &bytes)?;
    assert!(idx.findings_add(&finding("b", 2), 500).is_err());
    assert!(idx.findings_remove(&[]).is_err());
    assert_eq!(std::fs::read(&path)?, bytes);
    Ok(())
}

#[test]
fn a_damaged_middle_costs_checksum_work_within_the_budget() {
    let (a, b) = (super::encode(&finding("a", 1)), super::encode(&finding("b", 2)));
    let bytes = [a, vec![0xff; 5000], b].concat();
    let found = scan(&bytes).expect("scan");
    assert_eq!((found.items.len(), found.damaged, found.incomplete), (2, 1, false));
    assert!(found.hashed <= hash_budget(bytes.len()));
}

/// Class guard: the stored note is the cleaned input or a char-boundary prefix of it, at and
/// across the limit, for one-, two-, three- and four-byte characters. It would also catch a cut
/// that splits a character or a limit counted in anything but bytes.
#[test]
fn a_note_at_and_across_the_byte_limit_is_stored_whole_or_cut_on_a_boundary() {
    for unit in ["a", "\u{e9}", "\u{20ac}", "\u{1f600}"] {
        let fit = MAX_NOTE / unit.len();
        let whole = unit.repeat(fit);
        assert_eq!(cut_note(&whole), whole, "{unit:?} at the limit");
        let over = unit.repeat(fit + 1);
        let cut = cut_note(&over);
        assert!(cut.len() <= MAX_NOTE && over.starts_with(&cut), "{unit:?} across it");
        assert_eq!(clean_note(&over).len(), over.len(), "the refusal sees the true size");
    }
    assert_eq!(clean_note("a\u{1b}b\n"), "ab\n");
}
