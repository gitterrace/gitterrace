//! Guards on saving a settings file: a refused save leaves the file exactly as it was, a stale
//! stamp and a symlinked target are refused, and every save leaves one backup.

use std::fs;
use std::path::Path;

use crate::{SaveError, Stamp, save_config};

const GOOD: &str = "# mine\n[watch]\n# fast\nrescan_secs = 50\n";

fn dir_with(text: &str) -> tempfile::TempDir {
    let d = tempfile::tempdir().expect("tmp");
    fs::write(d.path().join("config.toml"), text).expect("write");
    d
}

fn stamp(d: &Path) -> Option<Stamp> {
    Stamp::of(&d.join("config.toml")).expect("stamp")
}

fn bytes(d: &Path) -> Vec<u8> {
    fs::read(d.join("config.toml")).expect("read")
}

/// Every entry of the folder, to prove a refusal leaves no temp file behind.
fn entries(d: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(d)
        .expect("ls")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

#[test]
fn invalid_toml_is_refused_and_the_file_is_unchanged() {
    let d = dir_with(GOOD);
    let before = (bytes(d.path()), stamp(d.path()), entries(d.path()));
    for bad in [
        "[watch\n",
        "[watch]\nrescan_secs = 1\n",
        "[server]\naddr = \"nope\"\n",
    ] {
        let e = save_config(d.path(), bad, before.1).expect_err(bad);
        assert!(matches!(e, SaveError::Invalid(_)), "{bad}: {e}");
        assert_eq!(
            (bytes(d.path()), stamp(d.path()), entries(d.path())),
            before,
            "{bad}"
        );
    }
}

#[test]
fn a_bad_value_names_its_line() {
    let d = dir_with(GOOD);
    let text = "[watch]\nenabled = true\nrescan_secs = 1\n";
    let Err(SaveError::Invalid(l)) = save_config(d.path(), text, stamp(d.path())) else {
        panic!("accepted");
    };
    assert_eq!((l.line, l.key.as_deref()), (3, Some("watch.rescan_secs")));
}

#[test]
fn a_stale_stamp_is_refused() {
    let d = dir_with(GOOD);
    let old = stamp(d.path());
    fs::write(d.path().join("config.toml"), "[watch]\nenabled = false\n").expect("edit");
    let edited = bytes(d.path());
    let e = save_config(d.path(), GOOD, old).expect_err("stale");
    assert!(matches!(e, SaveError::Stale), "{e}");
    assert_eq!(bytes(d.path()), edited);
    let e = save_config(d.path(), GOOD, None).expect_err("a file exists");
    assert!(matches!(e, SaveError::Stale), "{e}");
}

#[test]
fn a_first_save_creates_the_file() {
    let d = tempfile::tempdir().expect("tmp");
    save_config(d.path(), GOOD, None).expect("save");
    assert_eq!(bytes(d.path()), GOOD.as_bytes());
    assert_eq!(entries(d.path()), ["config.toml"]);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let m = fs::metadata(d.path().join("config.toml")).expect("meta");
        assert_eq!(m.permissions().mode() & 0o777, 0o600);
    }
}

#[cfg(unix)]
#[test]
fn a_symlinked_config_is_not_written() {
    let d = tempfile::tempdir().expect("tmp");
    let elsewhere = d.path().join("elsewhere.toml");
    fs::write(&elsewhere, GOOD).expect("write");
    std::os::unix::fs::symlink(&elsewhere, d.path().join("config.toml")).expect("link");
    let s = stamp(d.path());
    let e = save_config(d.path(), "[watch]\nenabled = false\n", s).expect_err("link");
    assert!(matches!(e, SaveError::NotRegular(_)), "{e}");
    assert_eq!(fs::read(&elsewhere).expect("read"), GOOD.as_bytes());
    let link = fs::symlink_metadata(d.path().join("config.toml")).expect("meta");
    assert!(link.file_type().is_symlink());
}

#[test]
fn save_leaves_one_backup() {
    let d = dir_with(GOOD);
    let second = "[watch]\nrescan_secs = 60\n";
    save_config(d.path(), second, stamp(d.path())).expect("save");
    let bak = d.path().join("config.toml.bak");
    assert_eq!(fs::read(&bak).expect("bak"), GOOD.as_bytes());
    save_config(d.path(), "[watch]\nrescan_secs = 70\n", stamp(d.path())).expect("save");
    assert_eq!(fs::read(&bak).expect("bak"), second.as_bytes());
    assert_eq!(entries(d.path()), ["config.toml", "config.toml.bak"]);
}

#[test]
fn stamp_tags_read_back() {
    let d = dir_with(GOOD);
    for s in [stamp(d.path()), None] {
        assert_eq!(Stamp::parse_tag(&Stamp::tag(s)), Some(s));
    }
    assert_eq!(Stamp::parse_tag("12"), None);
}
