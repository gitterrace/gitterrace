#[cfg(unix)]
use std::io;

#[cfg(unix)]
use super::{CHAIN_STATE_FILE, StateFile, load, save};
use super::{decode, encode};

/// Guard class: a state file read back is exactly what was saved under that key, and no
/// other key, truncation, bit flip or trailing byte reads as state. Red when the checksum, the
/// length or the key is not checked: a torn or foreign file would steer a build.
#[test]
fn a_state_file_reads_back_only_whole_and_under_its_own_key() {
    let b = encode(7, b"payload");
    assert_eq!(decode(&b, 7).as_deref(), Some(&b"payload"[..]));
    assert_eq!(decode(&b, 8), None, "another key");
    for cut in 0..b.len() {
        assert_eq!(decode(&b[..cut], 7), None, "cut at {cut}");
    }
    for at in 0..b.len() {
        let mut bad = b.clone();
        bad[at] ^= 1;
        assert_eq!(decode(&bad, 7), None, "flip at {at}");
    }
    let mut long = b;
    long.push(0);
    assert_eq!(decode(&long, 7), None, "a trailing byte");
}

/// Guard class: persisted state is written through the cache's no-follow atomic writer. A
/// link planted at the name is replaced, never written through, and its target keeps its
/// bytes. Red when the file is opened by path with `File::create`.
#[cfg(unix)]
#[test]
fn a_link_at_the_state_file_is_replaced_and_its_target_kept() -> io::Result<()> {
    let tmp = tempfile::tempdir()?;
    let dir = tmp.path().join("gitterrace");
    std::fs::create_dir(&dir)?;
    let victim = tmp.path().join("victim");
    std::fs::write(&victim, "keep")?;
    std::os::unix::fs::symlink(&victim, dir.join(CHAIN_STATE_FILE))?;
    save(&dir, StateFile::Chain, 1, b"x")?;
    assert_eq!(std::fs::read_to_string(&victim)?, "keep");
    assert!(std::fs::symlink_metadata(dir.join(CHAIN_STATE_FILE))?.is_file(), "the link is gone");
    assert_eq!(load(&dir, StateFile::Chain, 1).as_deref(), Some(&b"x"[..]));
    Ok(())
}

/// Guard class: a write that fails publishes nothing and keeps the old state whole. The
/// folder refuses new files, as a full disk does, and the state saved before still reads
/// back. Red when the file is written in place: that rewrites the old state without needing
/// a new entry in the folder.
#[cfg(unix)]
#[test]
fn a_failed_write_keeps_the_old_state_whole() -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir()?;
    let dir = tmp.path().join("gitterrace");
    std::fs::create_dir(&dir)?;
    save(&dir, StateFile::Chain, 1, b"old")?;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500))?;
    let failed = save(&dir, StateFile::Chain, 2, b"new");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    assert!(failed.is_err(), "a folder that takes no new file");
    assert_eq!(load(&dir, StateFile::Chain, 1).as_deref(), Some(&b"old"[..]));
    assert_eq!(load(&dir, StateFile::Chain, 2), None);
    Ok(())
}
