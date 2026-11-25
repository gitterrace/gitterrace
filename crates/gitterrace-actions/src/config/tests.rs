use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use crate::config::{Config, ConfigError, Cwd, Kind, Reloader};

fn private_dir() -> tempfile::TempDir {
    let d = tempfile::tempdir().expect("tempdir");
    std::fs::set_permissions(d.path(), std::fs::Permissions::from_mode(0o700)).expect("chmod");
    d
}

fn write(dir: &Path, name: &str, body: &str, mode: u32) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, body).expect("write");
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).expect("chmod");
    p
}

const GOOD: &str = r#"
version = 1
[[action]]
id = "echo"
label = "Echo"
kinds = ["line", "commit"]
command = ["/bin/echo", "hi"]
cwd = "repo"
timeout = "3s"
key = "e"

[[action]]
id = "sh"
label = "Shell"
kinds = ["page"]
shell = true
command = 'printf %s "$GITTERRACE_TITLE"'
"#;

#[test]
fn parses_a_good_file() {
    let c = Config::parse(GOOD, Path::new("/home/x")).expect("parse");
    let echo = c.get("echo").expect("echo");
    assert_eq!(echo.argv, ["/bin/echo", "hi"]);
    assert_eq!(echo.cwd, Cwd::Repo);
    assert_eq!(echo.timeout.as_secs(), 3);
    assert_eq!(echo.kinds, [Kind::Line, Kind::Commit]);
    assert_eq!(c.get("sh").expect("sh").argv[..2], ["/bin/sh", "-c"]);
}

#[test]
fn a_duplicate_or_built_in_key_is_refused_naming_both() {
    let two = |k1: &str, k2: &str| {
        format!(
            "version = 1\n[[action]]\nid = \"one\"\nlabel = \"One\"\nkinds = [\"page\"]\ncommand = \
            [\"/bin/echo\"]\nkey = \"{k1}\"\n[[action]]\nid = \"two\"\nlabel = \"Two\"\nkinds = \
            [\"page\"]\ncommand = [\"/bin/echo\"]\nkey = \"{k2}\"\n"
        )
    };
    let dup = Config::parse(&two("x", "x"), Path::new("/")).expect_err("duplicate");
    let msg = dup.to_string();
    assert!(msg.contains("two") && msg.contains("action one"), "{msg}");
    let clash = Config::parse(&two("x", "s"), Path::new("/")).expect_err("clash");
    let msg = clash.to_string();
    assert!(msg.contains("two") && msg.contains("Send to"), "{msg}");
    assert!(Config::parse(&two("x", "z"), Path::new("/")).is_ok());
}

#[test]
fn refuses_bad_actions() {
    let base = |extra: &str| {
        format!("version = 1\n[[action]]\nid = \"a\"\nlabel = \"A\"\nkinds = [\"line\"]\n{extra}\n")
    };
    for bad in [
        "command = [\"/bin/echo\"]\nkinds2 = 1",
        "command = []",
        "command = \"/bin/echo hi\"",
        "command = [\"/bin/echo\"]\nshell = true",
        "command = [\"./relative\"]",
        "command = [\"/nonexistent/prog\"]",
        "command = [\"/bin/echo\"]\ntimeout = \"121s\"",
        "command = [\"/bin/echo\"]\ntimeout = \"soon\"",
        "command = [\"/bin/echo\"]\ncwd = \"relative\"",
        "command = [\"/bin/echo\"]\nkey = \"ab\"",
        "command = [\"/bin/echo\"]\nenv = [\"GITTERRACE_X\"]",
    ] {
        assert!(Config::parse(&base(bad), Path::new("/")).is_err(), "{bad}");
    }
    let typo = GOOD.replace("\"commit\"", "\"comit\"");
    assert!(Config::parse(&typo, Path::new("/")).is_err(), "unknown kind");
    let dup = format!(
        "{GOOD}\n[[action]]\nid = \"echo\"\nlabel = \"x\"\nkinds = [\"page\"]\ncommand = \
        [\"/bin/echo\"]\n"
    );
    assert!(matches!(Config::parse(&dup, Path::new("/")), Err(ConfigError::Invalid { .. })));
    assert!(Config::parse(&GOOD.replace("version = 1", "version = 2"), Path::new("/")).is_err());
    let upper = GOOD.replace("id = \"echo\"", "id = \"Echo\"");
    assert!(Config::parse(&upper, Path::new("/")).is_err());
}

#[test]
fn page_and_menu_keys_are_refused_as_action_keys() {
    for (k, who) in crate::config::RESERVED_KEYS {
        let e = err(&format!("{HEAD}key = \"{k}\"\n"));
        assert!(e.contains(&format!("key '{k}' is already used by {who}")), "{e}");
    }
    assert!(Config::parse(&format!("{HEAD}key = \"n\"\n"), Path::new("/")).is_ok());
}

fn err(text: &str) -> String {
    Config::parse(text, Path::new("/")).expect_err("must refuse").to_string()
}

const HEAD: &str = "version = 1\n[[action]]\nid = \"a\"\nlabel = \"A\"\nkinds = \
    [\"line\"]\ncommand = [\"/bin/echo\"]\n";

#[test]
fn vars_are_validated_naming_the_key() {
    let ok = Config::parse(
        &format!("{HEAD}[action.vars]\nvault = \"Notes\"\nmode_2 = \"daily\"\n"),
        Path::new("/"),
    )
    .expect("parse");
    let vars = &ok.get("a").expect("a").vars;
    assert_eq!(vars.get("vault").map(String::as_str), Some("Notes"));
    assert_eq!(vars.len(), 2);

    for (bad, needle) in [
        ("MODE = \"x\"", "use lowercase: mode"),
        ("\"a-b\" = \"x\"", "vars.a-b"),
        ("\"\" = \"x\"", "vars."),
        (&format!("{} = \"x\"", "n".repeat(33)), "name must match"),
        (&format!("big = \"{}\"", "x".repeat(4097)), "vars.big"),
        ("num = 3", "parse"),
    ] {
        let e = err(&format!("{HEAD}[action.vars]\n{bad}\n"));
        assert!(e.contains(needle) && e.contains("action a") || needle == "parse", "{bad}: {e}");
    }
    let at_limit = format!("{HEAD}[action.vars]\nbig = \"{}\"\n", "x".repeat(4096));
    assert!(Config::parse(&at_limit, Path::new("/")).is_ok());
    let many: String = (0..33).map(|i| ["v", &i.to_string(), " = \"x\"\n"].concat()).collect();
    assert!(err(&format!("{HEAD}[action.vars]\n{many}")).contains("at most 32 vars"));
}

#[test]
fn scripts_are_shared_and_overridden() {
    let text = r#"
version = 1
[[script]]
id = "obsidian"
command = ["/bin/echo", "obs"]
cwd = "repo"
timeout = "5s"

[[action]]
id = "daily"
label = "Daily"
kinds = ["page"]
script = "obsidian"
timeout = "7s"
[action.vars]
mode = "daily"

[[action]]
id = "inbox"
label = "Inbox"
kinds = ["page"]
script = "obsidian"
cwd = "home"
"#;
    let c = Config::parse(text, Path::new("/")).expect("parse");
    let daily = c.get("daily").expect("daily");
    let inbox = c.get("inbox").expect("inbox");
    assert_eq!(daily.argv, ["/bin/echo", "obs"]);
    assert_eq!(inbox.argv, daily.argv);
    assert_eq!((daily.cwd.clone(), daily.timeout.as_secs()), (Cwd::Repo, 7));
    assert_eq!((inbox.cwd.clone(), inbox.timeout.as_secs()), (Cwd::Home, 5));

    let e = err(&text.replace("script = \"obsidian\"\ncwd", "script = \"nope\"\ncwd"));
    assert!(e.contains("action inbox") && e.contains("\"nope\""), "{e}");
    let both = err(&format!("{HEAD}script = \"x\"\n"));
    assert!(both.contains("not both"), "{both}");
    let dup = err(&format!("{text}\n[[script]]\nid = \"obsidian\"\ncommand = [\"/bin/echo\"]\n"));
    assert!(dup.contains("script obsidian") && dup.contains("duplicate"), "{dup}");
    // A script cannot reference another script, so a cycle cannot be written.
    let nested =
        err("version = 1\n[[script]]\nid = \"s\"\nscript = \"s\"\ncommand = [\"/bin/echo\"]\n");
    assert!(nested.contains("script"), "{nested}");
    let bad_cmd =
        err("version = 1\n[[script]]\nid = \"s\"\ncommand = [\"/nonexistent\"]\n[[action]]\nid = \
        \"a\"\nlabel = \"A\"\nkinds = [\"line\"]\nscript = \"s\"\n");
    assert!(bad_cmd.contains("action a"), "{bad_cmd}");
}

#[test]
fn permission_checks() {
    let d = private_dir();
    let ok = write(d.path(), "actions.toml", GOOD, 0o600);
    assert_eq!(Config::load(&ok, &[]).expect("load").actions.len(), 2);

    let gw = write(d.path(), "gw.toml", GOOD, 0o620);
    assert!(matches!(Config::load(&gw, &[]), Err(ConfigError::Unsafe { .. })), "group writable");
    let ww = write(d.path(), "ww.toml", GOOD, 0o602);
    assert!(matches!(Config::load(&ww, &[]), Err(ConfigError::Unsafe { .. })), "world writable");

    let link = d.path().join("link.toml");
    std::os::unix::fs::symlink(&ok, &link).expect("symlink");
    assert!(matches!(Config::load(&link, &[]), Err(ConfigError::Unsafe { .. })), "symlink");

    let open = tempfile::tempdir().expect("tempdir");
    std::fs::set_permissions(open.path(), std::fs::Permissions::from_mode(0o777)).expect("chmod");
    let in_open = write(open.path(), "actions.toml", GOOD, 0o600);
    assert!(
        matches!(Config::load(&in_open, &[]), Err(ConfigError::Unsafe { .. })),
        "writable parent"
    );
}

#[test]
fn repo_local_config_is_never_loaded() {
    let repo = private_dir();
    write(repo.path(), "actions.toml", GOOD, 0o600);
    std::fs::create_dir_all(repo.path().join(".gitterrace")).expect("mkdir");
    write(&repo.path().join(".gitterrace"), "actions.toml", GOOD, 0o600);
    let roots = vec![repo.path().to_path_buf()];
    // The default path is the user's config dir, never relative to a repo or the cwd.
    let default = gitterrace_config::config_dir(None).expect("home").join("actions.toml");
    assert!(default.is_absolute());
    assert!(!default.starts_with(repo.path()));
    // Even named explicitly, a file inside a served repo is refused.
    for p in [repo.path().join("actions.toml"), repo.path().join(".gitterrace/actions.toml")] {
        assert!(
            matches!(Config::load(&p, &roots), Err(ConfigError::Unsafe { .. })),
            "{}",
            p.display()
        );
    }
}

#[test]
fn reload_swaps_valid_and_keeps_old_on_invalid() {
    let d = private_dir();
    let p = write(d.path(), "actions.toml", GOOD, 0o600);
    let mut r = Reloader::new(p.clone(), vec![], Config::load(&p, &[]).expect("load"));
    let h = r.handle();
    assert!(r.poll().is_empty(), "unchanged file is not reloaded");

    let fewer = GOOD.split("[[action]]").take(2).collect::<Vec<_>>().join("[[action]]") + "\n# x";
    write(d.path(), "actions.toml", &fewer, 0o600);
    assert_eq!(r.poll(), ["action removed: sh"]);
    assert_eq!(h.read().expect("lock").actions.len(), 1);

    write(d.path(), "actions.toml", "version = 1\n[[action]]\nbroken", 0o600);
    assert!(r.poll()[0].contains("keeping the old set"));
    assert_eq!(h.read().expect("lock").actions.len(), 1);

    let with_vars = format!("{fewer}\n[action.vars]\nmode = \"daily\"\n");
    write(d.path(), "actions.toml", &with_vars, 0o600);
    assert_eq!(r.poll(), ["action changed: echo -> /bin/echo hi"]);
    let bad_var = with_vars.replace("mode =", "Mode =") + "\n";
    write(d.path(), "actions.toml", &bad_var, 0o600);
    assert!(r.poll()[0].contains("keeping the old set"));
    let cur = h.read().expect("lock").clone();
    assert_eq!(cur.get("echo").expect("echo").vars.get("mode").map(String::as_str), Some("daily"));
}

#[test]
fn placeholders_load_and_every_refusal_names_its_key() {
    let cmd = |c: &str| HEAD.replace("command = [\"/bin/echo\"]", c);
    let ok = Config::parse(
        &cmd("command = [\"/bin/echo\", \"--notes\", \"{markdown}\", \"--\", \"{title}\", \
                \"{{x}}\"]\nstdin = \"markdown\""),
        Path::new("/"),
    )
    .expect("parse");
    let a = ok.get("a").expect("a");
    assert_eq!(a.argv[1..], ["--notes", "{markdown}", "--", "{title}", "{{x}}"]);
    assert_eq!(a.stdin, crate::config::Stdin::Markdown);
    // A literal prefix or a trusted first value needs no `--`.
    let prefixed = cmd("command = [\"/bin/echo\", \"x={title}\", \"{root}/{path}\"]");
    let url = cmd(
        "command = [\"/bin/echo\", \"tendir://new-task?title={title:url}&notes={markdown:url}\", \
        \"{var.v:url}\", \"{sha:url}\"]\nvars = { v = \"x\" }",
    );
    if let Err(e) = Config::parse(&url, Path::new("/")) {
        panic!("{e}");
    }
    assert!(Config::parse(&prefixed, Path::new("/")).is_ok());
    for (bad, needle) in [
        ("command = [\"/bin/echo\", \"{titel}\"]", "command[1]: unknown placeholder {titel}"),
        ("command = [\"{sha}\"]", "command[0]: the program is literal"),
        ("command = [\"/bin/echo\", \"{title}\"]", "command[1]: {title} comes from the repository"),
        ("command = [\"/bin/echo\", \"{path}\", \"--\"]", "Put a literal \"--\""),
        (
            "command = [\"/bin/echo\", \"--\", \"{title:bogus}\"]",
            "command[2]: {title:bogus}: unknown modifier :bogus, known: :url",
        ),
        (
            "command = [\"/bin/echo\", \"{title:url}\"]",
            "command[1]: {title:url} comes from the repository",
        ),
        (
            "command = [\"/bin/echo\", \"--\", \"{titel:url}\"]",
            "command[2]: unknown placeholder {titel}",
        ),
        ("command = [\"/bin/echo\", \"{var.nope}\"]", "command[1]: {var.nope}: no var"),
        ("command = [\"/bin/echo\", \"a}\"]", "command[1]"),
        ("command = [\"/bin/echo\", \"{x\"]", "command[1]"),
        (
            "shell = true\ncommand = 'echo {title}'",
            "command: shell = true cannot take placeholders",
        ),
        ("command = [\"/bin/echo\"]\nstdin = \"xml\"", "stdin = \"xml\""),
    ] {
        let e = err(&cmd(bad));
        assert!(e.contains("action a") && e.contains(needle), "{bad}: {e}");
    }
    let script = "version = 1\n[[script]]\nid = \"s\"\ncommand = [\"/bin/echo\", \"{title}\"]\n";
    let e = err(script);
    assert!(e.contains("script s") && e.contains("command[1]"), "{e}");
    let shell = "version = 1\n[[script]]\nid = \"s\"\nshell = true\ncommand = 'echo {url}'\n";
    assert!(err(shell).contains("script s: command: shell"), "{}", err(shell));
    // A script's {var.NAME} is checked against each action that uses it.
    let uses = "version = 1\n[[script]]\nid = \"s\"\ncommand = [\"/bin/echo\", \"{var.url}\"]\n\
                    [[action]]\nid = \"a\"\nlabel = \"A\"\nkinds = [\"line\"]\nscript = \"s\"\n";
    assert!(err(uses).contains("action a: command[1]: {var.url}"), "{}", err(uses));
    let with = format!("{uses}[action.vars]\nurl = \"https://x\"\n");
    assert!(Config::parse(&with, Path::new("/")).is_ok());
    // Shell text that only looks like braces keeps working.
    let awk = cmd("shell = true\ncommand = 'echo ${HOME} | awk \"{print}\"'");
    assert!(Config::parse(&awk, Path::new("/")).is_ok());
}

/// Every `toml` block in docs/actions.md and docs/landing.md loads, so the docs never show a
/// config the server refuses. `deny_unknown_fields` makes a key the parser renamed a failure
/// here, and `examples::tests` holds each block to its starter or examples entry. Programs
/// are stubbed: `~/...` under a temp home, bare names as absolute
/// stubs, so only the PATH lookup itself is not exercised here.
#[test]
fn every_documented_example_loads() {
    use std::os::unix::fs::PermissionsExt;
    let docs =
        [include_str!("../../../../docs/actions.md"), include_str!("../../../../docs/landing.md")];
    let home = tempfile::tempdir().expect("tmp");
    let stub = |p: &Path| {
        std::fs::create_dir_all(p.parent().expect("parent")).expect("dir");
        std::fs::write(p, "#!/bin/sh\n").expect("write");
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o700)).expect("chmod");
    };
    let blocks: Vec<&str> = docs
        .iter()
        .flat_map(|d| d.split("```toml\n").skip(1).filter_map(|b| b.split("```").next()))
        .collect();
    assert!(blocks.len() >= 3, "found {} toml blocks", blocks.len());
    for (i, block) in blocks.iter().enumerate() {
        let mut text = if block.contains("version") {
            (*block).to_owned()
        } else {
            format!("version = 1\n{block}")
        };
        #[expect(
            clippy::needless_collect,
            reason = "the loop rewrites `text`, which the line iterator would still borrow"
        )]
        for prog in text
            .lines()
            .filter_map(|l| l.trim().strip_prefix("command"))
            .filter_map(|l| l.split('"').nth(1))
            .map(str::to_owned)
            .collect::<Vec<_>>()
        {
            if let Some(rest) = prog.strip_prefix("~/") {
                stub(&home.path().join(rest));
            } else if !prog.contains('/') {
                let abs = home.path().join("bin").join(&prog);
                stub(&abs);
                text = text.replace(&format!("[\"{prog}\""), &format!("[\"{}\"", abs.display()));
            }
        }
        if let Err(e) = Config::parse(&text, home.path()) {
            panic!("docs toml block {i} does not load: {e}\n{text}");
        }
    }
}
