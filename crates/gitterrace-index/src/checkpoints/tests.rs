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
