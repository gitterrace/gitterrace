use std::net::{SocketAddr, TcpListener};
use std::path::Path;
use std::time::{Duration, Instant};

use gitterrace_git::Repo;
use gitterrace_testkit::{Fixture, git};

use crate::open::{Listener, OpenError, ensure_server, identify, join_running, resolve, serves};

/// A repository with `src/a.rs`; returns (root, id, HEAD).
fn repo(dir: &Path) -> (std::path::PathBuf, String, String) {
    std::fs::create_dir_all(dir.join("src")).expect("dir");
    git(dir, &["init", "-q"]);
    std::fs::write(dir.join("src/a.rs"), "a\nb\nc\n").expect("w");
    git(dir, &["add", "."]);
    git(dir, &["commit", "-qm", "root"]);
    let head = git(dir, &["rev-parse", "HEAD"]);
    let root = git(dir, &["rev-list", "--max-parents=0", "HEAD"]);
    (
        dir.canonicalize().expect("canon"),
        root[..12].to_owned(),
        head,
    )
}

#[test]
fn resolves_registered_and_root_clones() {
    let t = tempfile::tempdir().expect("tmp");
    let (a, id, head) = repo(&t.path().join("a"));
    let link = format!("gitterrace://{id}/{}/src/a.rs#L2-L3", &head[..10]);
    let want = format!("/go/{id}/{head}/src/a.rs#L2-L3");

    let got = resolve(&link, &[(id.clone(), a.clone())], &[], 2, &[]).expect("registered");
    assert_eq!(
        (got.clone, got.path, got.note),
        (a.clone(), want.clone(), None)
    );

    let got = resolve(&link, &[], &[t.path().to_owned()], 2, &[]).expect("root");
    assert_eq!((got.clone, got.path), (a.clone(), want.clone()));

    let go = format!("http://127.0.0.1:9/go/{id}/{head}/src/a.rs#L2-L3");
    assert_eq!(
        resolve(&go, &[(id, a)], &[], 2, &[]).expect("go").path,
        want
    );
}

#[test]
fn unknown_id_missing_commit_and_missing_path() {
    let t = tempfile::tempdir().expect("tmp");
    let (a, id, head) = repo(&t.path().join("a"));
    let other = format!("gitterrace://000000000000/{head}");
    assert!(matches!(
        resolve(
            &other,
            &[("000000000000".to_owned(), a.clone())],
            &[t.path().to_owned()],
            2,
            &[]
        ),
        Err(OpenError::Unknown(_))
    ));
    let behind = format!("gitterrace://{id}/{}", "f".repeat(40));
    let got = resolve(&behind, &[(id.clone(), a.clone())], &[], 2, &[]).expect("behind");
    assert_eq!(got.path, "/");
    assert!(got.note.is_some_and(|n| n.contains("behind")));
    let gone = format!("gitterrace://{id}/{head}/no/such.rs");
    let got = resolve(&gone, &[(id.clone(), a)], &[], 2, &[]).expect("gone");
    assert_eq!(got.path, format!("/go/{id}/{head}"));
    assert!(matches!(
        resolve(&format!("gitterrace://{id}/{head}/../x"), &[], &[], 2, &[]),
        Err(OpenError::BadLink(_))
    ));
}

/// Serves `repos` on `addr` from a thread that lives until the test process ends, with its
/// registry and add secret in `state` (never the user's own folder).
fn serve(repos: Vec<std::path::PathBuf>, addr: SocketAddr, state: Option<&Path>) {
    let held = state.map(Path::to_path_buf);
    let state = held.clone();
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("rt")
            .block_on(gitterrace_web::serve_in(
                repos,
                addr,
                None,
                gitterrace_config::Config::default(),
                None,
                state,
            ))
            .expect("serve");
    });
    let t = Instant::now();
    // Without a state folder the server has no secret to prove, so only its port answers.
    let up = |l: Listener| match held {
        Some(_) => l == Listener::Gitterrace,
        None => l != Listener::Free,
    };
    while !up(identify(addr, held.as_deref())) {
        assert!(
            t.elapsed() < Duration::from_secs(10),
            "server never came up"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn free_addr() -> SocketAddr {
    let l = TcpListener::bind("127.0.0.1:0").expect("bind");
    l.local_addr().expect("addr")
}

/// Guard class: open never starts a second server when one answers on the address.
#[test]
fn open_never_starts_a_second_server_when_one_answers_on_the_address() {
    let tmp = tempfile::tempdir().expect("tmp");
    let a = Fixture::standard(&tmp.path().join("a")).path().to_owned();
    let b = Fixture::init(&tmp.path().join("b"))
        .write("x", "other\n")
        .commit("other root")
        .path()
        .to_owned();
    let c = Fixture::init(&tmp.path().join("c"))
        .write("y", "third\n")
        .commit("third root")
        .path()
        .to_owned();
    let fourth = Fixture::init(&tmp.path().join("d"))
        .write("z", "fourth\n")
        .commit("fourth root")
        .path()
        .to_owned();
    let state = tmp.path().join("state");
    let addr = free_addr();
    serve(vec![a.clone(), b], addr, Some(&state));
    let probe_of = |p: &Path| {
        let r = Repo::discover(p).expect("repo");
        format!(
            "/go/{}/{}",
            r.id().expect("id"),
            r.resolve("HEAD").expect("head")
        )
    };

    // (1) a served repository is joined, in multi mode with its /r/<name>/ prefix.
    let mut started = Vec::new();
    let got = ensure_server(&a, addr, Some(&state), &mut |_, x| {
        started.push(x);
        Ok(())
    })
    .expect("join");
    assert_eq!((got, started.len()), (addr, 0));
    let repo = Repo::discover(&a).expect("repo");
    let probe = format!(
        "/go/{}/{}",
        repo.id().expect("id"),
        repo.resolve("HEAD").expect("head")
    );
    assert!(serves(addr, &probe));

    // (2) a repository the server lacks is added to it: same address, nothing started,
    // and its permalink now resolves there. Asking again is a join.
    assert!(!serves(addr, &probe_of(&c)));
    for _ in 0..2 {
        let mut started = Vec::new();
        let got = ensure_server(&c, addr, Some(&state), &mut |_, x| {
            started.push(x);
            Ok(())
        })
        .expect("added");
        assert_eq!((got, started.len()), (addr, 0));
        assert!(serves(addr, &probe_of(&c)));
    }

    // (2b) without the secret (another user, or no state folder) adding fails, and only
    // then does the repository get a server of its own; the listener stays untouched.
    let mut started = Vec::new();
    let got = ensure_server(&fourth, addr, None, &mut |_, x| {
        started.push(x);
        serve(vec![fourth.clone()], x, None);
        Ok(())
    })
    .expect("own server");
    assert_eq!(started, vec![got]);
    assert_ne!(got, addr);
    assert_eq!(identify(addr, Some(&state)), Listener::Gitterrace);
    assert!(!serves(addr, &probe_of(&fourth)));

    // A foreign listener is not replaced either.
    let foreign = TcpListener::bind("127.0.0.1:0").expect("bind");
    let faddr = foreign.local_addr().expect("addr");
    assert_eq!(identify(faddr, Some(&state)), Listener::Other);
    let mut started = Vec::new();
    let got = ensure_server(&c, faddr, Some(&state), &mut |_, x| {
        started.push(x);
        serve(vec![c.clone()], x, None);
        Ok(())
    })
    .expect("beside foreign");
    assert_ne!(got, faddr);
    assert_eq!(started, vec![got]);

    // (3) nothing listens: the server starts on the configured address.
    let free = free_addr();
    let mut started = Vec::new();
    let got = ensure_server(&c, free, Some(&state), &mut |_, x| {
        started.push(x);
        serve(vec![c.clone()], x, None);
        Ok(())
    })
    .expect("fresh");
    assert_eq!((got, started), (free, vec![free]));
}

/// Guard class: a second `git terrace` on a held address ends up served there, never a
/// bind error, and only a local gitterrace server with its secret is joined.
#[test]
fn a_second_serve_joins_the_running_server() {
    let tmp = tempfile::tempdir().expect("tmp");
    let a = Fixture::standard(&tmp.path().join("a")).path().to_owned();
    let b = Fixture::init(&tmp.path().join("b"))
        .write("x", "other\n")
        .commit("other root")
        .path()
        .to_owned();
    let state = tmp.path().join("state");
    let addr = free_addr();
    assert!(!join_running(addr, std::slice::from_ref(&b), Some(&state)).expect("free"));
    serve(vec![a], addr, Some(&state));
    assert!(!join_running(addr, std::slice::from_ref(&b), None).expect("no secret"));
    assert!(join_running(addr, std::slice::from_ref(&b), Some(&state)).expect("join"));
    let r = Repo::discover(&b).expect("repo");
    let probe = format!(
        "/go/{}/{}",
        r.id().expect("id"),
        r.resolve("HEAD").expect("head")
    );
    assert!(serves(addr, &probe));
    let foreign = TcpListener::bind("127.0.0.1:0").expect("bind");
    let faddr = foreign.local_addr().expect("addr");
    assert!(!join_running(faddr, &[b], Some(&state)).expect("foreign"));
}

fn probe_of(p: &Path) -> String {
    let r = Repo::discover(p).expect("repo");
    format!(
        "/go/{}/{}",
        r.id().expect("id"),
        r.resolve("HEAD").expect("head")
    )
}

/// Guard class: joining several roots is all or nothing up to the first add, and never
/// falls through to a bind on the held address. A root that is not a repository stops
/// before anything is added; an add the server refuses is an error, not `Ok(false)`.
#[test]
fn a_partial_join_adds_nothing_and_never_binds() {
    let tmp = tempfile::tempdir().expect("tmp");
    let a = Fixture::standard(&tmp.path().join("a")).path().to_owned();
    let b = Fixture::init(&tmp.path().join("b"))
        .write("x", "other\n")
        .commit("other root")
        .path()
        .to_owned();
    let plain = tmp.path().join("plain");
    std::fs::create_dir_all(&plain).expect("dir");
    let state = tmp.path().join("state");
    let addr = free_addr();
    serve(vec![a], addr, Some(&state));

    let err = join_running(addr, &[b.clone(), plain], Some(&state));
    assert!(err.is_err(), "a bad root is an error, not a bind");
    assert!(
        !serves(addr, &probe_of(&b)),
        "nothing added before the bad root"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // A repository the server finds but cannot open: it answers 422.
        let c = Fixture::init(&tmp.path().join("c"))
            .write("y", "third\n")
            .commit("third root")
            .path()
            .to_owned();
        let git_dir = c.join(".git");
        std::fs::set_permissions(&git_dir, std::fs::Permissions::from_mode(0o555)).expect("chmod");
        if std::fs::create_dir(git_dir.join("probe")).is_err() {
            let got = join_running(addr, &[b, c], Some(&state));
            std::fs::set_permissions(&git_dir, std::fs::Permissions::from_mode(0o755))
                .expect("chmod");
            let why = got
                .expect_err("a refused add is an error, not a bind")
                .to_string();
            assert!(why.contains("could not add"), "{why}");
        }
    }
}

/// Guard class: a listener is gitterrace only when it proves our secret. A local server
/// that answers every request like gitterrace's status route, while our token file for
/// its port exists, is foreign and is never sent a repository path.
#[test]
fn a_mimic_of_the_status_reply_is_not_gitterrace() {
    use std::io::{Read, Write};
    use std::sync::{Arc, Mutex};

    let tmp = tempfile::tempdir().expect("tmp");
    let b = Fixture::standard(&tmp.path().join("b")).path().to_owned();
    let fake = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = fake.local_addr().expect("addr");
    let seen = Arc::new(Mutex::new(String::new()));
    let log = Arc::clone(&seen);
    std::thread::spawn(move || {
        for mut s in fake.incoming().flatten() {
            let mut buf = [0u8; 4096];
            let n = s.read(&mut buf).unwrap_or(0);
            log.lock()
                .expect("lock")
                .push_str(&String::from_utf8_lossy(&buf[..n]));
            let body = r#"{"note":null,"proof":"x"}"#;
            let reply = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
                body.len()
            );
            drop(s.write_all(reply.as_bytes()));
        }
    });
    // Our token file for that port, as a stale one left by a server that is gone.
    let state = tmp.path().join("state");
    std::fs::create_dir_all(&state).expect("dir");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o700)).expect("chmod");
    }
    let file = state.join(format!("add-token-{}", addr.port()));
    std::fs::write(&file, "stale-secret").expect("token");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).expect("chmod");
    }

    assert_eq!(identify(addr, Some(&state)), Listener::Other);
    assert!(!join_running(addr, std::slice::from_ref(&b), Some(&state)).expect("foreign"));
    let seen = seen.lock().expect("lock").clone();
    assert!(!seen.contains("POST"), "{seen}");
    assert!(!seen.contains(&b.display().to_string()), "{seen}");
}
