//! `install-handler`: opt-in registration of the `gitterrace://` scheme with the OS.
//!
//! macOS gets a small `AppleScript` app (URLs reach apps as Apple events, not argv) in
//! `~/Applications`, registered with `LaunchServices`. Linux gets a `.desktop` file and
//! `xdg-mime`. WSL only prints steps. The handler just runs `<this program> open <url>`; the
//! strict parsing and lookup live in `open`.

use std::{
    fmt::Write as _,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, bail};

const APP_NAME: &str = "gitterrace-open.app";
const BUNDLE_ID: &str = "dev.gitterrace.open";
/// An `Info.plist` key only install-handler writes. With the bundle id it marks the app as
/// ours: an app that merely shares the id is a user's, and is never removed.
const MARKER_KEY: &str = "GitterraceInstallHandler";
/// The program the app runs, kept beside the marker so an upgrade and `open` can tell when
/// it moved without reading the compiled script.
const PROGRAM_KEY: &str = "GitterraceProgram";
/// The app icon, the brand mark (`make app-icon` renders it from docs/brand/mark-dark.svg).
/// It replaces osacompile's script icon, and the asset catalogue that would override it goes.
const ICON: &[u8] = include_bytes!("../assets/gitterrace.icns");
const DESKTOP_NAME: &str = "gitterrace-open.desktop";
const LSREGISTER: &str = concat!(
    "/System/Library/Frameworks/CoreServices.framework/Frameworks/",
    "LaunchServices.framework/Support/lsregister"
);

#[expect(
    clippy::print_stdout,
    reason = "user-facing CLI output: this is the command's result or diagnostic, not a log line"
)]
pub(crate) fn run(uninstall: bool) -> anyhow::Result<()> {
    let home = gitterrace_config::home_var()
        .map(PathBuf::from)
        .context("the home folder variable (HOME, or USERPROFILE on Windows) is not set")?;
    let exe = std::env::current_exe().context("finding this program")?;
    if cfg!(windows) {
        print!("{}", windows_steps(&exe));
        Ok(())
    } else if cfg!(target_os = "macos") {
        macos(&home, &exe, uninstall)
    } else if is_wsl() {
        print!("{}", wsl_steps(&exe));
        Ok(())
    } else {
        linux(&home, &exe, uninstall)
    }
}

pub(crate) fn is_wsl() -> bool {
    std::fs::read_to_string("/proc/version").is_ok_and(|v| v.to_lowercase().contains("microsoft"))
}

/// `AppleScript` source of the macOS handler. The URL is shell-quoted, and the program runs in
/// the background so the app returns at once.
pub(crate) fn applescript(exe: &Path) -> String {
    format!(
        "on open location theURL\n\tdo shell script quoted form of \"{}\" & \" open \" & quoted \
        form of theURL & \" >/dev/null 2>&1 &\"\nend open location\n",
        exe.display().to_string().replace('\\', "\\\\").replace('"', "\\\"")
    )
}

pub(crate) const fn url_types_json() -> &'static str {
    r#"[{"CFBundleURLName":"gitterrace link","CFBundleURLSchemes":["gitterrace"]}]"#
}

fn app_path(home: &Path) -> PathBuf {
    home.join("Applications").join(APP_NAME)
}

fn tool(cmd: &mut Command) -> anyhow::Result<()> {
    let out = cmd.output().with_context(|| format!("running {cmd:?}"))?;
    if !out.status.success() {
        bail!("{cmd:?} failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

fn plist_set(plist: &Path, args: &[&str]) -> anyhow::Result<()> {
    tool(Command::new("plutil").args(args).arg(plist))
}

#[expect(
    clippy::print_stdout,
    reason = "user-facing CLI output: this is the command's result or diagnostic, not a log line"
)]
fn macos(home: &Path, exe: &Path, uninstall: bool) -> anyhow::Result<()> {
    let app = app_path(home);
    if uninstall {
        if !app.exists() {
            println!("nothing to remove: {} does not exist", app.display());
            return Ok(());
        }
        if owner_of(&app) == Owner::Foreign {
            bail!(not_ours(&app));
        }
        let _ = Command::new(LSREGISTER).arg("-u").arg(&app).output();
        std::fs::remove_dir_all(&app).with_context(|| format!("removing {}", app.display()))?;
        println!("removed {}\nunregistered gitterrace:// from LaunchServices", app.display());
        return Ok(());
    }
    // A marked app is ours and is rewritten in place: current icon, program path and
    // signature. An unmarked one may be a user's, or from a build before the marker.
    let previous = match owner_of(&app) {
        Owner::Absent => None,
        Owner::Ours(previous) => previous,
        Owner::Foreign => bail!(not_ours(&app)),
    };
    write_app(&app, exe)?;
    tool(Command::new(LSREGISTER).arg("-f").arg(&app))?;
    if let Some(old) = previous.filter(|old| old != exe) {
        println!("the handler pointed at {}, it now points at {}", old.display(), exe.display());
    }
    println!(
        "wrote {} (runs: {} open <url>)\nregistered gitterrace:// with LaunchServices\nremove \
        with: git terrace install-handler --uninstall",
        app.display(),
        exe.display()
    );
    Ok(())
}

fn not_ours(app: &Path) -> String {
    format!(
        "{} has no install-handler marker, leaving it. If it is an older gitterrace handler, \
         remove it once by hand, then run install-handler again",
        app.display()
    )
}

/// Builds the handler app at `app` for `exe`, replacing one that is there. No
/// `LaunchServices` call, so a test can build one in a fixture home.
fn write_app(app: &Path, exe: &Path) -> anyhow::Result<()> {
    if app.exists() {
        std::fs::remove_dir_all(app).with_context(|| format!("replacing {}", app.display()))?;
    }
    std::fs::create_dir_all(app.parent().context("no parent")?)?;
    let script =
        std::env::temp_dir().join(format!("gitterrace-open-{}.applescript", std::process::id()));
    std::fs::write(&script, applescript(exe))?;
    let made = tool(Command::new("osacompile").arg("-o").arg(app).arg(&script));
    let _ = std::fs::remove_file(&script);
    made?;
    let plist = app.join("Contents/Info.plist");
    plist_set(&plist, &["-replace", "CFBundleIdentifier", "-string", BUNDLE_ID])?;
    plist_set(&plist, &["-replace", "LSUIElement", "-bool", "true"])?;
    plist_set(&plist, &["-replace", MARKER_KEY, "-bool", "true"])?;
    let program = exe.to_str().context("the program path is not UTF-8")?;
    plist_set(&plist, &["-replace", PROGRAM_KEY, "-string", program])?;
    plist_set(&plist, &["-replace", "CFBundleURLTypes", "-json", url_types_json()])?;
    let res = app.join("Contents/Resources");
    std::fs::write(res.join("gitterrace.icns"), ICON)?;
    let _ = std::fs::remove_file(res.join("applet.icns"));
    let _ = std::fs::remove_file(res.join("Assets.car"));
    plist_set(&plist, &["-replace", "CFBundleIconFile", "-string", "gitterrace"])?;
    // osacompile names its asset catalogue here; without the key the icon file above wins.
    let _ = plist_set(&plist, &["-remove", "CFBundleIconName"]);
    // Editing the bundle invalidates osacompile's ad hoc signature, so sign it again.
    tool(Command::new("codesign").args(["--force", "--sign", "-"]).arg(app))
}

#[derive(Debug, PartialEq, Eq)]
enum Owner {
    Absent,
    Ours(Option<PathBuf>),
    Foreign,
}

fn owner_of(app: &Path) -> Owner {
    if !app.exists() {
        return Owner::Absent;
    }
    let plist = std::fs::read_to_string(app.join("Contents/Info.plist")).unwrap_or_default();
    owner(&plist)
}

/// Ours needs the bundle id and the marker key in the `Info.plist` (XML, as plutil keeps
/// it). The id alone is not enough: a user's app may share it.
fn owner(plist: &str) -> Owner {
    let id = format!("<key>CFBundleIdentifier</key>\n\t<string>{BUNDLE_ID}</string>");
    let marker = format!("<key>{MARKER_KEY}</key>\n\t<true/>");
    if !(plist.contains(&id) && plist.contains(&marker)) {
        return Owner::Foreign;
    }
    let key = format!("<key>{PROGRAM_KEY}</key>\n\t<string>");
    let program = plist.find(&key).and_then(|i| {
        let rest = &plist[i + key.len()..];
        rest.find("</string>").map(|j| PathBuf::from(unxml(&rest[..j])))
    });
    Owner::Ours(program)
}

/// The five XML entities plutil writes, back to text. `&amp;` goes last.
fn unxml(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// A hint for `git terrace open` when the macOS handler runs another program than `exe`, or
/// one that is gone. It only tells: rewriting the handler is install-handler's job.
pub(crate) fn stale_hint(exe: &Path) -> Option<String> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    hint_for(&app_path(Path::new(&gitterrace_config::home_var()?)), exe)
}

fn hint_for(app: &Path, exe: &Path) -> Option<String> {
    let Owner::Ours(Some(runs)) = owner_of(app) else {
        return None;
    };
    let same =
        runs == exe || runs.canonicalize().is_ok_and(|r| exe.canonicalize().is_ok_and(|e| e == r));
    if same {
        return None;
    }
    let why = if runs.exists() { "another" } else { "a missing" };
    Some(format!(
        "gitterrace:// links run {why} program ({}). Run `git terrace install-handler` to point \
        them here",
        runs.display()
    ))
}

/// Content of the Linux `.desktop` file. The link reaches the program as `"$1"` of a fixed
/// `sh -c` script, never as script text, so a launcher that hands the line to a shell still
/// passes `;`, `$` or a backtick in the link as data.
pub(crate) fn desktop_entry(exe: &Path) -> String {
    let mut s = String::from("[Desktop Entry]\nType=Application\nName=gitterrace link opener\n");
    let _ = writeln!(
        s,
        r#"Exec=sh -c "\\"\\$0\\" open \\"\\$1\\"" "{}" %u"#,
        desktop_quote(&exe.display().to_string())
    );
    s.push_str("NoDisplay=true\nTerminal=false\nMimeType=x-scheme-handler/gitterrace;\n");
    s
}

/// A path for inside `Exec="..."`: the Desktop Entry spec reads `"`, `` ` ``, `$` and `\`
/// as special there and `%` as a field code anywhere, so each is escaped (the spec's double
/// escaping: `\\"` in the file is `"` in the argument).
fn desktop_quote(p: &str) -> String {
    let mut out = String::with_capacity(p.len());
    for c in p.chars() {
        match c {
            // Exec quoting wants `\c`; the key-file layer then doubles every backslash.
            '\\' => out.push_str(r"\\\\"),
            '"' | '`' | '$' => {
                out.push_str(r"\\");
                out.push(c);
            }
            '%' => out.push_str("%%"),
            _ => out.push(c),
        }
    }
    out
}

fn desktop_path(home: &Path) -> PathBuf {
    home.join(".local/share/applications").join(DESKTOP_NAME)
}

#[expect(
    clippy::print_stdout,
    reason = "user-facing CLI output: this is the command's result or diagnostic, not a log line"
)]
fn linux(home: &Path, exe: &Path, uninstall: bool) -> anyhow::Result<()> {
    let file = desktop_path(home);
    if uninstall {
        if file.exists() {
            std::fs::remove_file(&file)?;
            println!("removed {}", file.display());
        } else {
            println!("nothing to remove: {} does not exist", file.display());
        }
        let _ = Command::new("update-desktop-database").arg(file.parent().unwrap_or(home)).output();
        return Ok(());
    }
    std::fs::create_dir_all(file.parent().context("no parent")?)?;
    std::fs::write(&file, desktop_entry(exe))?;
    tool(Command::new("xdg-mime").args(["default", DESKTOP_NAME, "x-scheme-handler/gitterrace"]))?;
    println!(
        "wrote {}\nran: xdg-mime default {DESKTOP_NAME} x-scheme-handler/gitterrace\nremove with: \
        git terrace install-handler --uninstall",
        file.display()
    );
    Ok(())
}

/// WSL cannot be registered from here, so only the steps are printed.
pub(crate) fn wsl_steps(exe: &Path) -> String {
    format!(
        "WSL: nothing was written. To open gitterrace:// links from Windows, register a\n\
         handler for the scheme yourself that runs:\n  wsl.exe {} open \"%1\"\n",
        exe.display()
    )
}

/// Windows registers a scheme under `HKCU\Software\Classes`. Writing the registry from here is
/// not done: the commands are printed for the user to run, like on WSL.
pub(crate) fn windows_steps(exe: &Path) -> String {
    let exe = exe.display();
    format!(
        "Windows: nothing was written. To open gitterrace:// links, run in a terminal:\n\
         \x20 reg add HKCU\\Software\\Classes\\gitterrace /ve /d \"URL:gitterrace link\" /f\n\
         \x20 reg add HKCU\\Software\\Classes\\gitterrace /v \"URL Protocol\" /d \"\" /f\n\
         \x20 reg add HKCU\\Software\\Classes\\gitterrace\\shell\\open\\command \
         /ve /d \"\\\"{exe}\\\" \
         open \\\"%1\\\"\" /f\n\
         remove with: reg delete HKCU\\Software\\Classes\\gitterrace /f\n"
    )
}

#[cfg(test)]
mod tests;
