//! The shipped binary returns freed pages at once, however it was built.
//!
//! The decay is set at start, not by a build flag that `cargo install` from elsewhere would
//! miss. This spawns the real `git-terrace` and reads back what its allocator runs with.
#![cfg(not(windows))]

#[test]
fn the_binary_runs_with_zero_page_decay() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_git-terrace"))
        .env("TEST_ALLOC_DECAY", "1")
        .env_remove("JEMALLOC_SYS_WITH_MALLOC_CONF")
        .env_remove("MALLOC_CONF")
        .env_remove("_RJEM_MALLOC_CONF")
        .output();
    let Ok(out) = out else {
        panic!("run git-terrace: {out:?}");
    };
    assert!(out.status.success(), "{out:?}");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "0 0 0 0",
        "dirty and muzzy decay of new arenas and arena 0, ms"
    );
}
