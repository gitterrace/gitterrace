use std::io;

use gitterrace_git::SnapFile;

use super::{FILE, id_of};
use crate::Index;

fn snap(path: &str, text: &str) -> SnapFile {
    SnapFile {
        path: path.to_owned(),
        hash: gitterrace_git::content_id(text.as_bytes()),
        text: Some(text.as_bytes().to_vec()),
    }
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
    let (id, new) = idx.checkpoint_add(&files, "abc", false, "first", 1 << 20, 100)?;
    assert!(new && id.starts_with("cp-") && id.len() == 15);
    let (again, new) = idx.checkpoint_add(&files, "abc", true, "", 1 << 20, 200)?;
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
        idx.checkpoint_add(&[snap("a", t)], "h", false, "", 1 << 20, i as i64)?;
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
        ids.push(idx.checkpoint_add(&[snap("f", &text)], "h", false, "", 3000, i64::from(i))?.0);
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
    let (id, _) = idx.checkpoint_add(&[snap("a", "1\n")], "h", false, "", 1 << 20, 1)?;
    let path = idx.dir().join(FILE);
    let whole = std::fs::read(&path)?;
    let mut torn = whole.clone();
    torn.extend_from_slice(&whole[..whole.len() - 3]);
    std::fs::write(&path, &torn)?;
    assert_eq!(idx.checkpoint_list()?.items.len(), 1);
    assert!(idx.checkpoint_files(&id)?.is_some());
    idx.checkpoint_add(&[snap("b", "2\n")], "h", false, "", 1 << 20, 2)?;
    assert_eq!(idx.checkpoint_list()?.items.len(), 2);
    assert!(std::fs::read(&path)?.starts_with(&whole));
    Ok(())
}

#[test]
fn a_flipped_byte_in_a_copy_reads_as_a_gone_copy() -> io::Result<()> {
    let (_t, idx) = open()?;
    let (id, _) =
        idx.checkpoint_add(&[snap("a", "hello hello hello\n")], "h", false, "", 1 << 20, 1)?;
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
    assert_eq!(id_of(&a), id_of(&b));
    assert_ne!(id_of(&a), id_of(&[snap("a", "1"), snap("b", "3")]));
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
    assert!(idx.checkpoint_list()?.items.is_empty());
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
    assert!(idx.checkpoint_list()?.items.is_empty());
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
    let err = idx.checkpoint_add(&[snap("a", "1\n")], "h", false, "", 1 << 20, 2).unwrap_err();
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
