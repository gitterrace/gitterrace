use std::io;

use gitterrace_git::{SnapFile, Snapshot};

use super::{FILE, id_of};
use crate::Index;

fn snap(path: &str, text: &str) -> SnapFile {
    SnapFile {
        path: path.to_owned(),
        hash: gitterrace_git::content_id(text.as_bytes()),
        text: Some(text.as_bytes().to_vec()),
    }
}

/// A checkpoint of `files` whose scan was complete.
fn add(
    idx: &Index,
    files: &[SnapFile],
    head: &str,
    auto: bool,
    label: &str,
    cap: u64,
    now: i64,
) -> io::Result<(String, bool)> {
    let snap = Snapshot { files: files.to_vec(), files_omitted: false };
    idx.checkpoint_add(&snap, head, auto, label, cap, now)
}

fn open() -> io::Result<(tempfile::TempDir, Index)> {
    let tmp = tempfile::tempdir()?;
    let idx = Index::open_dir(tmp.path().join("gitterrace"))?;
    Ok((tmp, idx))
}

#[test]
fn a_checkpoint_round_trips_and_the_same_state_dedupes() -> io::Result<()> {
    let (_t, idx) = open()?;
    let files = [snap("a.txt", "one\n"), snap("b/c.txt", "two\n")];
    let (id, new) = add(&idx, &files, "abc", false, "first", 1 << 20, 100)?;
    assert!(new && id.starts_with("cp-") && id.len() == 15);
    let (again, new) = add(&idx, &files, "abc", true, "", 1 << 20, 200)?;
    assert_eq!((again.as_str(), new), (id.as_str(), false));
    let list = idx.checkpoint_list()?;
    assert_eq!(list.items.len(), 1);
    assert_eq!((list.items[0].files, list.items[0].created), (2, 100));
    assert!(list.bytes_used > 0);
    let back = idx.checkpoint_files(&id)?.expect("known");
    assert_eq!(back, files);
    assert!(idx.checkpoint_files("cp-000000000000")?.is_none());
    Ok(())
}

#[test]
fn the_list_is_newest_first() -> io::Result<()> {
    let (_t, idx) = open()?;
    for (i, t) in ["1", "2", "3"].iter().enumerate() {
        add(&idx, &[snap("a", t)], "h", false, "", 1 << 20, i as i64)?;
    }
    let times: Vec<i64> = idx.checkpoint_list()?.items.iter().map(|c| c.created).collect();
    assert_eq!(times, [2, 1, 0]);
    Ok(())
}

#[test]
fn the_cap_drops_the_oldest_copies_and_keeps_their_names() -> io::Result<()> {
    let (_t, idx) = open()?;
    let mut ids = Vec::new();
    for i in 0..3 {
        // Random-looking text so zstd cannot shrink it below the cap.
        let mut x = 12_345_u32 + u32::try_from(i).unwrap_or(0);
        let text: String = (0..2000)
            .map(|_| {
                x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                char::from(b'a' + u8::try_from((x >> 16) % 26).unwrap_or(0))
            })
            .collect();
        ids.push(add(&idx, &[snap("f", &text)], "h", false, "", 3000, i64::from(i))?.0);
    }
    let list = idx.checkpoint_list()?;
    assert_eq!(list.items.len(), 3, "a dropped copy keeps its checkpoint");
    assert!(list.bytes_used <= 3000, "{}", list.bytes_used);
    let oldest = idx.checkpoint_files(&ids[0])?.expect("kept");
    assert!(oldest[0].text.is_none() && !oldest[0].hash.is_empty());
    let newest = idx.checkpoint_files(&ids[2])?.expect("kept");
    assert!(newest[0].text.is_some());
    Ok(())
}

#[test]
fn a_torn_tail_is_ignored_and_the_next_write_repairs_it() -> io::Result<()> {
    let (_t, idx) = open()?;
    let (id, _) = add(&idx, &[snap("a", "1\n")], "h", false, "", 1 << 20, 1)?;
    let path = idx.dir().join(FILE);
    let whole = std::fs::read(&path)?;
    let mut torn = whole.clone();
    torn.extend_from_slice(&whole[..whole.len() - 3]);
    std::fs::write(&path, &torn)?;
    assert_eq!(idx.checkpoint_list()?.items.len(), 1);
    assert!(idx.checkpoint_files(&id)?.is_some());
    add(&idx, &[snap("b", "2\n")], "h", false, "", 1 << 20, 2)?;
    assert_eq!(idx.checkpoint_list()?.items.len(), 2);
    assert!(std::fs::read(&path)?.starts_with(&whole));
    Ok(())
}

#[test]
fn a_flipped_byte_in_a_copy_reads_as_a_gone_copy() -> io::Result<()> {
    let (_t, idx) = open()?;
    let (id, _) = add(&idx, &[snap("a", "hello hello hello\n")], "h", false, "", 1 << 20, 1)?;
    let path = idx.dir().join(FILE);
    let mut b = std::fs::read(&path)?;
    let last = b.len() - 1;
    b[last] ^= 0xff;
    std::fs::write(&path, &b)?;
    let files = idx.checkpoint_files(&id)?.expect("header intact");
    assert!(files[0].text.is_none());
    Ok(())
}

#[test]
fn the_id_does_not_depend_on_file_order() {
    let a = [snap("a", "1"), snap("b", "2")];
    let b = [snap("b", "2"), snap("a", "1")];
    assert_eq!(id_of(&a, ""), id_of(&b, ""));
    assert_ne!(id_of(&a, ""), id_of(&[snap("a", "1"), snap("b", "3")], ""));
}

/// One hand-built frame: the way a planted log gets past the checksums.
fn frame(files: Vec<super::FileMeta>, blobs: &[u8]) -> io::Result<Vec<u8>> {
    let meta = super::Meta {
        id: "cp-planted00000".to_owned(),
        created: 1,
        head: "h".to_owned(),
        auto: false,
        label: String::new(),
        files,
        omitted: false,
    };
    super::encode(&meta, blobs)
}

fn planted(idx: &Index, bytes: &[u8]) -> io::Result<std::path::PathBuf> {
    let path = idx.dir().join(FILE);
    std::fs::write(&path, bytes)?;
    Ok(path)
}

fn file_with(copy: super::Copy) -> super::FileMeta {
    super::FileMeta { path: "a".to_owned(), hash: "x".to_owned(), copy: Some(copy) }
}

/// A fixed part that declares `blob_len` blob bytes after a one-byte header.
fn fixed_claiming(blob_len: u64) -> Vec<u8> {
    let hdr = b"{";
    let mut b = super::MAGIC.to_vec();
    b.extend_from_slice(&1u32.to_le_bytes());
    b.extend_from_slice(&crc32fast::hash(hdr).to_le_bytes());
    b.extend_from_slice(&blob_len.to_le_bytes());
    b.extend_from_slice(hdr);
    b
}

#[test]
fn a_length_that_wraps_the_end_offset_is_damage_not_a_panic() -> io::Result<()> {
    let (_t, idx) = open()?;
    planted(&idx, &fixed_claiming(u64::MAX - 5))?;
    assert_eq!(idx.checkpoint_list()?.items.len(), 0);
    assert!(idx.checkpoint_files("cp-planted00000")?.is_none());
    Ok(())
}

#[test]
fn a_sparse_file_claiming_a_huge_blob_is_refused_before_any_allocation() -> io::Result<()> {
    let (_t, idx) = open()?;
    let claim = 3u64 << 30;
    let path = planted(&idx, &fixed_claiming(claim))?;
    let f = std::fs::OpenOptions::new().write(true).open(&path)?;
    f.set_len(super::FIXED + 1 + claim)?;
    drop(f);
    assert_eq!(idx.checkpoint_list()?.items.len(), 0);
    assert!(idx.checkpoint_files("cp-planted00000")?.is_none());
    Ok(())
}

#[test]
fn a_log_with_too_many_frames_lists_only_the_cap() -> io::Result<()> {
    let (_t, idx) = open()?;
    let one = frame(Vec::new(), &[])?;
    planted(&idx, &one.repeat(super::MAX_CHECKPOINTS + 50))?;
    assert_eq!(idx.checkpoint_list()?.items.len(), super::MAX_CHECKPOINTS);
    Ok(())
}

#[test]
fn a_write_refuses_a_log_it_cannot_read_and_leaves_it_alone() -> io::Result<()> {
    let (_t, idx) = open()?;
    let mut bytes = frame(Vec::new(), &[])?;
    bytes.extend_from_slice(&fixed_claiming(u64::MAX));
    let path = planted(&idx, &bytes)?;
    let err = add(&idx, &[snap("a", "1\n")], "h", false, "", 1 << 20, 2).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    assert_eq!(std::fs::read(&path)?, bytes, "the user's bytes are not rewritten");
    assert_eq!(idx.checkpoint_list()?.items.len(), 1, "the readable frame still lists");
    Ok(())
}

#[test]
fn copies_that_point_outside_their_blob_read_as_gone() -> io::Result<()> {
    let (_t, idx) = open()?;
    let z = zstd::bulk::compress(b"hello\n", 3)?;
    let ok = super::Copy { off: 0, len: z.len() as u64, size: 6, crc: crc32fast::hash(&z) };
    let wild = |off, len| super::Copy { off, len, ..ok.clone() };
    let files = vec![
        file_with(ok.clone()),
        file_with(wild(u64::MAX, 4)),
        file_with(wild(2, u64::MAX)),
        file_with(wild(0, super::MAX_COPY + 1)),
        file_with(wild(z.len() as u64, 1)),
    ];
    planted(&idx, &frame(files, &z)?)?;
    let got = idx.checkpoint_files("cp-planted00000")?.expect("header sound");
    let texts: Vec<bool> = got.iter().map(|f| f.text.is_some()).collect();
    assert_eq!(texts, [true, false, false, false, false]);
    Ok(())
}

#[test]
fn a_copy_that_unpacks_past_its_stated_size_or_the_cap_reads_as_gone() -> io::Result<()> {
    let (_t, idx) = open()?;
    let big = vec![b'a'; 100_000];
    let z = zstd::bulk::compress(&big, 3)?;
    let crc = crc32fast::hash(&z);
    let len = z.len() as u64;
    let lies = super::Copy { off: 0, len, size: 10, crc };
    let huge = super::Copy { off: 0, len, size: super::MAX_COPY + 1, crc };
    planted(&idx, &frame(vec![file_with(lies), file_with(huge)], &z)?)?;
    let got = idx.checkpoint_files("cp-planted00000")?.expect("header sound");
    assert!(got.iter().all(|f| f.text.is_none()));
    Ok(())
}

/// Text zstd cannot shrink, so a snapshot of a few of them is bigger than a small cap.
fn noise(seed: u32, n: usize) -> String {
    let mut x = seed;
    (0..n)
        .map(|_| {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            char::from(b'a' + u8::try_from((x >> 16) % 26).unwrap_or(0))
        })
        .collect()
}

#[test]
fn the_first_checkpoint_keeps_within_the_cap_and_names_what_it_dropped() -> io::Result<()> {
    let (_t, idx) = open()?;
    let files: Vec<SnapFile> =
        (0..6).map(|i| snap(&format!("f{i}"), &noise(i + 1, 4000))).collect();
    let (id, _) = add(&idx, &files, "h", false, "", 3000, 1)?;
    let list = idx.checkpoint_list()?;
    assert!(list.bytes_used <= 3000, "{}", list.bytes_used);
    let back = idx.checkpoint_files(&id)?.expect("kept");
    assert_eq!(back.len(), 6, "every path stays");
    assert!(back.iter().all(|f| !f.hash.is_empty()));
    assert!(back.iter().all(|f| f.text.is_none()), "a frame over the cap keeps no copy");
    Ok(())
}

#[test]
fn a_zero_cap_keeps_names_and_hashes_but_no_copies() -> io::Result<()> {
    let (_t, idx) = open()?;
    let (id, _) = add(&idx, &[snap("a", "x\n")], "h", false, "", 0, 1)?;
    assert_eq!(idx.checkpoint_list()?.bytes_used, 0);
    assert!(idx.checkpoint_files(&id)?.expect("kept")[0].text.is_none());
    Ok(())
}

#[test]
fn a_cap_stored_log_survives_a_lower_cap_unread_until_the_next_write() -> io::Result<()> {
    let (_t, idx) = open()?;
    let (id, _) = add(&idx, &[snap("a", "x\n")], "h", false, "", 1 << 20, 1)?;
    let before = std::fs::read(idx.dir().join(FILE))?;
    assert!(idx.checkpoint_files(&id)?.is_some(), "a reader never deletes");
    assert_eq!(std::fs::read(idx.dir().join(FILE))?, before);
    Ok(())
}

#[test]
fn the_same_files_at_another_head_are_another_checkpoint() -> io::Result<()> {
    let (_t, idx) = open()?;
    let (one, _) = add(&idx, &[], "aaa", false, "", 1 << 20, 1)?;
    let (two, new) = add(&idx, &[], "bbb", false, "", 1 << 20, 2)?;
    assert!(new && one != two);
    assert_eq!(idx.checkpoint(&two)?.expect("kept").head, "bbb");
    Ok(())
}

/// Guard class: any state that matches an older frame but not the newest. The newest checkpoint
/// must be the state last saved, and the id returned must be the one the list shows first.
#[test]
fn returning_to_an_older_state_is_a_new_newest_checkpoint() -> io::Result<()> {
    let (_t, idx) = open()?;
    let a = [snap("f", "A\n")];
    let b = [snap("f", "B\n")];
    let (ida, _) = add(&idx, &a, "h", false, "", 1 << 20, 1)?;
    add(&idx, &b, "h", false, "", 1 << 20, 2)?;
    let (again, new) = add(&idx, &a, "h", false, "", 1 << 20, 3)?;
    assert!(new, "A after B is not nothing-changed");
    assert_eq!(again, ida);
    let list = idx.checkpoint_list()?;
    assert_eq!(list.items.len(), 3);
    assert_eq!((list.items[0].id.as_str(), list.items[0].created), (ida.as_str(), 3));
    let (_, new) = add(&idx, &a, "h", false, "", 1 << 20, 4)?;
    assert!(!new, "the same state as the newest still dedupes");
    Ok(())
}

#[test]
fn an_id_with_two_frames_reads_the_newest_one() -> io::Result<()> {
    let (_t, idx) = open()?;
    let a = [snap("f", "A\n")];
    let (id, _) = add(&idx, &a, "h", false, "", 0, 1)?;
    add(&idx, &[snap("f", "B\n")], "h", false, "", 1 << 20, 2)?;
    add(&idx, &a, "h", false, "", 1 << 20, 3)?;
    let back = idx.checkpoint_files(&id)?.expect("kept");
    assert!(back[0].text.is_some(), "the old frame lost its copy, the newest has it");
    Ok(())
}

#[test]
fn a_checkpoint_remembers_that_its_file_list_was_cut() -> io::Result<()> {
    let (_t, idx) = open()?;
    let cut = Snapshot { files: vec![snap("a", "1\n")], files_omitted: true };
    let (id, _) = idx.checkpoint_add(&cut, "h", false, "", 1 << 20, 1)?;
    let (whole, _) = add(&idx, &[snap("b", "2\n")], "h", false, "", 1 << 20, 2)?;
    let flags: Vec<bool> = idx.checkpoint_list()?.items.iter().map(|c| c.files_omitted).collect();
    assert_eq!(flags, [false, true]);
    assert!(idx.checkpoint(&id)?.expect("kept").files_omitted);
    assert!(!idx.checkpoint(&whole)?.expect("kept").files_omitted);
    Ok(())
}

/// A log written before the field existed has no `omitted` and reads as complete.
#[test]
fn a_header_without_the_omitted_field_reads_as_complete() {
    let m: super::Meta = serde_json::from_str(
        r#"{"id":"cp-x","created":1,"head":"h","auto":false,"label":"","files":[]}"#,
    )
    .expect("parse");
    assert!(!m.omitted);
}

#[test]
fn a_long_unicode_label_is_cut_to_200_characters_at_the_index() -> io::Result<()> {
    let (_t, idx) = open()?;
    let long = "\u{e9}\u{1f600}".repeat(150);
    add(&idx, &[snap("a.txt", "one\n")], "abc", false, &long, 1 << 20, 100)?;
    let label = &idx.checkpoint_list()?.items[0].label;
    assert_eq!(label.chars().count(), 200);
    assert!(long.starts_with(label.as_str()));
    Ok(())
}

#[test]
fn the_label_limit_is_defined_once() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let api = std::fs::read_to_string(root.join("gitterrace-api/src/core/checkpoints.rs"))
        .unwrap_or_default();
    assert!(!api.contains("take("), "the api must pass labels through to the index limit");
    let own = std::fs::read_to_string(root.join("gitterrace-index/src/checkpoints.rs"))
        .unwrap_or_default();
    assert_eq!(own.matches("take(MAX_LABEL)").count(), 1);
    assert_eq!(own.matches("take(200)").count(), 0);
}
