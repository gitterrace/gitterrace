//! The allocator hands freed pages back to the OS at once, from one arena.
//!
//! An idle server should shrink to what it holds, not to the most it ever held. jemalloc keeps
//! freed pages for ten seconds by default and the system allocator on macOS keeps them for
//! good. The decay is applied at start, so it holds however the binary was built.
//!
//! The arena count can only be set before the first allocation, so it is a build setting
//! (`.cargo/config.toml`). A build from outside the repository runs with one arena per core
//! and says so at start: it works the same, idle memory is a few MB higher.

use tikv_jemalloc_ctl::{Access, AsName};

/// Applies the decay and notes a build without the arena setting. Returns true when the
/// harness asked only for the allocator settings in effect (`TEST_ALLOC_DECAY` for the decay,
/// `TEST_ALLOC_ARENAS` for the arena count, `TEST_ALLOC_TCACHE` for the thread cache of the
/// main thread and of a worker while and after it runs, read by the guards in
/// `tests/alloc.rs`), which are then printed. Installs the index workers' thread cache hook.
#[expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "the harness reads the answer from stdout; the build note goes to stderr before \
        logging starts"
)]
pub(crate) fn start() -> std::io::Result<bool> {
    release_freed_pages()?;
    gitterrace_git::set_worker_hook(thread_cache);
    if std::env::var_os("TEST_ALLOC_TCACHE").is_some() {
        let read =
            || -> std::io::Result<bool> { "thread.tcache.enabled\0".name().read().map_err(ctl) };
        let main = read()?;
        let worker = std::thread::scope(|s| {
            s.spawn(|| {
                thread_cache(true);
                let on = read();
                thread_cache(false);
                Ok::<_, std::io::Error>((on?, read()?))
            })
            .join()
        })
        .map_err(|_| std::io::Error::other("tcache probe panicked"))??;
        println!("{main} {} {}", worker.0, worker.1);
        return Ok(true);
    }
    let arenas: u32 = "opt.narenas\0".name().read().map_err(ctl)?;
    if std::env::var_os("TEST_ALLOC_ARENAS").is_some() {
        println!("{arenas}");
        return Ok(true);
    }
    if std::env::var_os("TEST_ALLOC_DECAY").is_none() {
        if arenas != 1 {
            eprintln!(
                "note: built without the repository's allocator setting, idle memory is a few MB \
                higher"
            );
        }
        return Ok(false);
    }
    let [dirty, muzzy, dirty0, muzzy0] = decay()?;
    println!("{dirty} {muzzy} {dirty0} {muzzy0}");
    Ok(true)
}

/// The index workers' thread cache: on while a worker runs, off (which flushes it into the
/// arena) when it ends. Off everywhere else, so an idle server's freed objects pin no pages.
/// A failed switch only costs speed, so it is not reported.
fn thread_cache(on: bool) {
    if "thread.tcache.enabled\0".name().write(on).is_err() {
        tracing::debug!("thread.tcache.enabled not switched");
    }
}

/// Sets dirty and muzzy page decay to zero for every arena, now and later.
fn release_freed_pages() -> std::io::Result<()> {
    for key in ["dirty_decay_ms", "muzzy_decay_ms"] {
        // Arenas made from now on.
        format!("arenas.{key}\0")
            .as_str()
            .name()
            .write(0_isize)
            .map_err(ctl)?;
        // Before main only arena 0 exists (the others are made on a thread's first
        // allocation and take the default above). jemalloc's "all arenas" index crashes on
        // arenas not made yet, so name it.
        format!("arena.0.{key}\0")
            .as_str()
            .name()
            .write(0_isize)
            .map_err(ctl)?;
    }
    Ok(())
}

/// The decay, in ms, of new arenas and of arena 0: `[dirty, muzzy]` each.
fn decay() -> std::io::Result<[isize; 4]> {
    let read = |key: &str| -> std::io::Result<isize> {
        format!("{key}\0").as_str().name().read().map_err(ctl)
    };
    Ok([
        read("arenas.dirty_decay_ms")?,
        read("arenas.muzzy_decay_ms")?,
        read("arena.0.dirty_decay_ms")?,
        read("arena.0.muzzy_decay_ms")?,
    ])
}

fn ctl(e: tikv_jemalloc_ctl::Error) -> std::io::Error {
    std::io::Error::other(e.to_string())
}
