//! Unit tests for the link handler install.

use super::*;

/// Guard class: the embedded app icon is a real icns that holds the sizes Finder and the
/// Dock ask for, so a truncated or wrong-format asset never reaches a user's bundle.
#[test]
fn the_app_icon_is_an_icns_with_every_size() {
    assert_eq!(&ICON[..4], b"icns");
    let len = u32::from_be_bytes([ICON[4], ICON[5], ICON[6], ICON[7]]) as usize;
    assert_eq!(len, ICON.len(), "icns length header");
    let mut types = Vec::new();
    let mut at = 8;
    while at + 8 <= ICON.len() {
        types.push(String::from_utf8_lossy(&ICON[at..at + 4]).into_owned());
        let n = u32::from_be_bytes([ICON[at + 4], ICON[at + 5], ICON[at + 6], ICON[at + 7]]);
        at += n as usize;
    }
    // 16, 32, 128, 256, 512 at 1x and @2x, as iconutil writes them (ic04 is 16, ic05 32,
    // ic10 is 512@2x, 1024 px).
    for t in ["ic04", "ic11", "ic05", "ic12", "ic07", "ic13", "ic08", "ic14", "ic09", "ic10"] {
        assert!(types.iter().any(|x| x == t), "missing {t} in {types:?}");
    }
}

#[test]
fn golden_macos_content() {
    assert_eq!(
        applescript(Path::new("/opt/bin/git-terrace")),
        "on open location theURL\n\tdo shell script quoted form of \"/opt/bin/git-terrace\" & \
        \" open \" & quoted form of theURL & \" >/dev/null 2>&1 &\"\nend open location\n"
    );
    assert_eq!(
        url_types_json(),
        r#"[{"CFBundleURLName":"gitterrace link","CFBundleURLSchemes":["gitterrace"]}]"#
    );
}

#[test]
fn golden_desktop_entry() {
    assert_eq!(
        desktop_entry(Path::new("/opt/bin/git-terrace")),
        concat!(
            "[Desktop Entry]\nType=Application\nName=gitterrace link opener\n",
            r#"Exec=sh -c "\\"\\$0\\" open \\"\\$1\\"" "/opt/bin/git-terrace" %u"#,
            "\nNoDisplay=true\nTerminal=false\nMimeType=x-scheme-handler/gitterrace;\n"
        )
    );
}

#[test]
fn desktop_entry_escapes_exec_specials() {
    let e = desktop_entry(Path::new("/o/100%/a\"b$c`d\\e/git-terrace"));
    assert!(e.contains(r#" "/o/100%%/a\\"b\\$c\\`d\\\\e/git-terrace" %u"#), "{e}");
}

/// Splits an `Exec` value as a launcher does: the key-file layer undoes `\\`, then
/// double quotes group and `\` escapes the next character, and `%u` becomes `url`.
#[cfg(unix)]
fn exec_argv(exec: &str, url: &str) -> Vec<String> {
    let unkeyed = exec.replace(r"\\", "\\");
    let (mut out, mut cur, mut quoted, mut esc) = (Vec::new(), String::new(), false, false);
    for c in unkeyed.chars() {
        match c {
            _ if esc => {
                cur.push(c);
                esc = false;
            }
            '\\' if quoted => esc = true,
            '"' => quoted = !quoted,
            ' ' if !quoted => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out.into_iter().map(|a| if a == "%u" { url.to_owned() } else { a.replace("%%", "%") }).collect()
}

#[test]
#[cfg(unix)]
fn desktop_entry_exec_safe_for_sh_c_invocation() {
    use std::os::unix::fs::PermissionsExt;

    let t = tempfile::tempdir().expect("tmp");
    let exe = t.path().join("a $b`c`/git-terrace");
    std::fs::create_dir_all(exe.parent().expect("parent")).expect("mkdir");
    let log = t.path().join("args");
    std::fs::write(&exe, format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\n", log.display()))
        .expect("write");
    std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let pwned = t.path().join("pwned");
    let url = format!("gitterrace://x;touch {}|$(touch {0})`touch {0}`", pwned.display());
    let entry = desktop_entry(&exe);
    let exec = entry.lines().find_map(|l| l.strip_prefix("Exec=")).expect("exec");
    let argv = exec_argv(exec, &url);
    assert_eq!(argv[0], "sh", "{exec}");
    let ran = Command::new(&argv[0]).args(&argv[1..]).status().expect("run");
    assert!(ran.success());
    assert!(!pwned.exists(), "the url ran as a command");
    let got = std::fs::read_to_string(&log).expect("log");
    assert_eq!(got, format!("open\n{url}\n"));
}

#[test]
fn linux_uninstall_removes_only_its_file() {
    let home = std::env::temp_dir().join(format!("gt-handler-{}", std::process::id()));
    let apps = home.join(".local/share/applications");
    std::fs::create_dir_all(&apps).unwrap();
    let other = apps.join("other.desktop");
    std::fs::write(&other, "x").unwrap();
    std::fs::write(desktop_path(&home), desktop_entry(Path::new("/x"))).unwrap();
    linux(&home, Path::new("/x"), true).unwrap();
    assert!(!desktop_path(&home).exists());
    assert!(other.exists());
    std::fs::remove_dir_all(&home).unwrap();
}

fn plist(body: &str) -> String {
    format!("<plist version=\"1.0\">\n<dict>\n\t{body}\n</dict>\n</plist>\n")
}

fn id() -> String {
    format!("<key>CFBundleIdentifier</key>\n\t<string>{BUNDLE_ID}</string>")
}

/// Guard class: ownership needs the bundle id and the marker, so uninstall and
/// reinstall never remove an unmarked app: a user's that shares the id, or one from a
/// build before the marker. The refusal names the path and the one-time removal.
#[test]
fn an_unmarked_app_is_refused() {
    let marker = format!("<key>{MARKER_KEY}</key>\n\t<true/>");
    assert_eq!(owner(&plist(&id())), Owner::Foreign, "id alone");
    assert_eq!(owner(&plist(&marker)), Owner::Foreign, "marker alone");
    let off = format!("{}\n\t<key>{MARKER_KEY}</key>\n\t<false/>", id());
    assert_eq!(owner(&plist(&off)), Owner::Foreign, "marker false");
    let program = format!("<key>{PROGRAM_KEY}</key>\n\t<string>/a &amp; b/git-terrace</string>");
    assert_eq!(
        owner(&plist(&format!("{}\n\t{marker}\n\t{program}", id()))),
        Owner::Ours(Some(PathBuf::from("/a & b/git-terrace")))
    );

    let tmp = tempfile::tempdir().expect("tmp");
    let home = tmp.path();
    let app = app_path(home);
    std::fs::create_dir_all(app.join("Contents")).expect("dir");
    std::fs::write(app.join("Contents/Info.plist"), plist(&id())).expect("w");
    let why = macos(home, Path::new("/x"), false).expect_err("reinstall refused");
    assert!(why.to_string().contains(&app.display().to_string()), "{why}");
    assert!(why.to_string().contains("remove it once"), "{why}");
    assert!(macos(home, Path::new("/x"), true).is_err(), "uninstall refused");
    assert!(app.join("Contents/Info.plist").exists(), "left in place");
}

/// Guard class: a marked app is rewritten in place for the current program, with the
/// current icon and a valid signature. Built in a fixture home with the real tools,
/// without `LaunchServices`.
#[cfg(target_os = "macos")]
#[test]
fn a_marked_app_is_upgraded_in_place() {
    let tmp = tempfile::tempdir().expect("tmp");
    let app = app_path(tmp.path());
    let old = Path::new("/old/bin/git-terrace");
    let new = Path::new("/new place/bin/git-terrace");
    write_app(&app, old).expect("first");
    assert_eq!(owner_of(&app), Owner::Ours(Some(old.to_path_buf())));
    // An older icon, as a previous build would have left.
    let icon = app.join("Contents/Resources/gitterrace.icns");
    std::fs::write(&icon, b"old icon").expect("w");

    let Owner::Ours(previous) = owner_of(&app) else {
        panic!("marked app not ours");
    };
    write_app(&app, new).expect("upgrade");
    assert_eq!(previous.as_deref(), Some(old));
    assert_eq!(owner_of(&app), Owner::Ours(Some(new.to_path_buf())));
    assert_eq!(std::fs::read(&icon).expect("icon"), ICON);
    let main = app.join("Contents/Resources/Scripts/main.scpt");
    let src = Command::new("osadecompile").arg(&main).output().expect("osadecompile");
    let src = String::from_utf8_lossy(&src.stdout);
    assert!(src.contains("/new place/bin/git-terrace"), "{src}");
    tool(Command::new("codesign").args(["--verify", "--strict"]).arg(&app)).expect("signature");
}

/// Guard class: `open` hints when the handler runs a missing or another program, stays
/// quiet when it runs this one, and never rewrites the app.
#[test]
fn open_hints_at_a_stale_handler_without_touching_it() {
    let tmp = tempfile::tempdir().expect("tmp");
    let app = app_path(tmp.path());
    std::fs::create_dir_all(app.join("Contents")).expect("dir");
    let here = tmp.path().join("git-terrace");
    std::fs::write(&here, "").expect("exe");
    let with = |p: &Path| {
        let body = format!(
            "{}\n\t<key>{MARKER_KEY}</key>\n\t<true/>\n\t<key>{PROGRAM_KEY}</key>\n\
             \t<string>{}</string>",
            id(),
            p.display()
        );
        std::fs::write(app.join("Contents/Info.plist"), plist(&body)).expect("w");
    };
    with(&here);
    assert_eq!(hint_for(&app, &here), None);
    with(Path::new("/gone/git-terrace"));
    let hint = hint_for(&app, &here).expect("hint");
    assert!(hint.contains("a missing program") && hint.contains("install-handler"), "{hint}");
    assert!(
        std::fs::read_to_string(app.join("Contents/Info.plist"))
            .expect("r")
            .contains("/gone/git-terrace")
    );
    with(tmp.path());
    assert!(hint_for(&app, &here).expect("hint").contains("another program"));
}

#[test]
fn wsl_writes_nothing() {
    assert!(wsl_steps(Path::new("/x")).contains("nothing was written"));
}
