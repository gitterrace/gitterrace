//! `install-handler`: opt-in registration of the `gitterrace://` scheme with the OS.
//!
//! macOS gets a small AppleScript app (URLs reach apps as Apple events, not argv) in
//! `~/Applications`, registered with LaunchServices. Linux gets a `.desktop` file and
//! `xdg-mime`. WSL only prints steps. The handler just runs `<this program> open <url>`; the
//! strict parsing and lookup live in `open`.

use std::{
    fmt::Write as _,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{bail, Context};

const APP_NAME: &str = "gitterrace-open.app";
const BUNDLE_ID: &str = "dev.gitterrace.open";
const DESKTOP_NAME: &str = "gitterrace-open.desktop";
const LSREGISTER: &str = "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";

pub(crate) fn run(uninstall: bool) -> anyhow::Result<()> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME is not set")?;
    let exe = std::env::current_exe().context("finding this program")?;
    if cfg!(target_os = "macos") {
        macos(&home, &exe, uninstall)
    } else if is_wsl() {
        print!("{}", wsl_steps(&exe));
        Ok(())
    } else {
        linux(&home, &exe, uninstall)
    }
}

fn is_wsl() -> bool {
    std::fs::read_to_string("/proc/version").is_ok_and(|v| v.to_lowercase().contains("microsoft"))
}

/// AppleScript source of the macOS handler. The URL is shell-quoted, and the program runs in
/// the background so the app returns at once.
pub(crate) fn applescript(exe: &Path) -> String {
    format!(
        "on open location theURL\n\tdo shell script quoted form of \"{}\" & \" open \" & quoted form of theURL & \" >/dev/null 2>&1 &\"\nend open location\n",
        exe.display().to_string().replace('\\', "\\\\").replace('"', "\\\"")
    )
}

/// JSON for `CFBundleURLTypes`, inserted into the generated `Info.plist`.
pub(crate) fn url_types_json() -> &'static str {
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

fn macos(home: &Path, exe: &Path, uninstall: bool) -> anyhow::Result<()> {
    let app = app_path(home);
    if uninstall {
        if !app.exists() {
            println!("nothing to remove: {} does not exist", app.display());
            return Ok(());
        }
        let plist = std::fs::read_to_string(app.join("Contents/Info.plist")).unwrap_or_default();
        if !plist.contains(BUNDLE_ID) {
            bail!("{} was not written by install-handler, leaving it", app.display());
        }
        let _ = Command::new(LSREGISTER).arg("-u").arg(&app).output();
        std::fs::remove_dir_all(&app).with_context(|| format!("removing {}", app.display()))?;
        println!("removed {}\nunregistered gitterrace:// from LaunchServices", app.display());
        return Ok(());
    }
    if app.exists() {
        let plist = std::fs::read_to_string(app.join("Contents/Info.plist")).unwrap_or_default();
        if !plist.contains(BUNDLE_ID) {
            bail!("{} exists and is not ours, leaving it", app.display());
        }
        std::fs::remove_dir_all(&app)?;
    }
    std::fs::create_dir_all(app.parent().context("no parent")?)?;
    let script = std::env::temp_dir().join(format!("gitterrace-open-{}.applescript", std::process::id()));
    std::fs::write(&script, applescript(exe))?;
    let made = tool(Command::new("osacompile").arg("-o").arg(&app).arg(&script));
    let _ = std::fs::remove_file(&script);
    made?;
    let plist = app.join("Contents/Info.plist");
    plist_set(&plist, &["-replace", "CFBundleIdentifier", "-string", BUNDLE_ID])?;
    plist_set(&plist, &["-replace", "LSUIElement", "-bool", "true"])?;
    plist_set(&plist, &["-replace", "CFBundleURLTypes", "-json", url_types_json()])?;
    // Editing the bundle invalidates osacompile's ad hoc signature, so sign it again.
    tool(Command::new("codesign").args(["--force", "--sign", "-"]).arg(&app))?;
    tool(Command::new(LSREGISTER).arg("-f").arg(&app))?;
    println!(
        "wrote {} (runs: {} open <url>)\nregistered gitterrace:// with LaunchServices\nremove with: git terrace install-handler --uninstall",
        app.display(),
        exe.display()
    );
    Ok(())
}

/// Content of the Linux `.desktop` file.
pub(crate) fn desktop_entry(exe: &Path) -> String {
    let mut s = String::from("[Desktop Entry]\nType=Application\nName=gitterrace link opener\n");
    let _ = writeln!(s, "Exec=\"{}\" open %u", exe.display());
    s.push_str("NoDisplay=true\nTerminal=false\nMimeType=x-scheme-handler/gitterrace;\n");
    s
}

fn desktop_path(home: &Path) -> PathBuf {
    home.join(".local/share/applications").join(DESKTOP_NAME)
}

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
        "wrote {}\nran: xdg-mime default {DESKTOP_NAME} x-scheme-handler/gitterrace\nremove with: git terrace install-handler --uninstall",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn golden_macos_content() {
        assert_eq!(
            applescript(Path::new("/opt/bin/git-terrace")),
            "on open location theURL\n\tdo shell script quoted form of \"/opt/bin/git-terrace\" & \" open \" & quoted form of theURL & \" >/dev/null 2>&1 &\"\nend open location\n"
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
            "[Desktop Entry]\nType=Application\nName=gitterrace link opener\nExec=\"/opt/bin/git-terrace\" open %u\nNoDisplay=true\nTerminal=false\nMimeType=x-scheme-handler/gitterrace;\n"
        );
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

    #[test]
    fn wsl_writes_nothing() {
        assert!(wsl_steps(Path::new("/x")).contains("nothing was written"));
    }
}
