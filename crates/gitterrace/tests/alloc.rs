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

/// The binary built from this repository runs jemalloc with one arena
/// (`.cargo/config.toml`). With one arena per core, each keeps partly used pages of its own
/// after the idle release, and a released server stays 5 to 8 MB above a just-started one.
#[test]
fn the_binary_runs_with_one_arena() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_git-terrace"))
        .env("TEST_ALLOC_ARENAS", "1")
        .env_remove("MALLOC_CONF")
        .env_remove("_RJEM_MALLOC_CONF")
        .output();
    let Ok(out) = out else {
        panic!("run git-terrace: {out:?}");
    };
    assert!(out.status.success(), "{out:?}");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "1",
        "jemalloc arenas: build without JEMALLOC_SYS_WITH_MALLOC_CONF from .cargo/config.toml"
    );
}

/// Thread caches stay off on the serving threads and come on only inside an index worker,
/// flushed when it ends: with them the git.git index build takes 10.8 s, without 21.6 s.
#[test]
fn the_binary_enables_thread_caches_only_inside_index_workers() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_git-terrace"))
        .env("TEST_ALLOC_TCACHE", "1")
        .env_remove("MALLOC_CONF")
        .env_remove("_RJEM_MALLOC_CONF")
        .output();
    let Ok(out) = out else {
        panic!("run git-terrace: {out:?}");
    };
    assert!(out.status.success(), "{out:?}");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "false true false",
        "thread cache: main thread, worker running, worker done"
    );
}
