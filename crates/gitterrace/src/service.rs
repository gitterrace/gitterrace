//! `service`: run the server at login under systemd (Linux) or launchd (macOS).
//!
//! `print` renders the unit, `install` writes it and prints (or with `--now` runs) the command
//! that starts it, `uninstall` removes it. Only a file carrying our marker is ever removed or
//! replaced, the rule `handler` applies to its app. The unit never sets `--public`.

use std::{
    fmt::Write as _,
    net::SocketAddr,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, bail};

use crate::cli::Action;

const UNIT_NAME: &str = "gitterrace.service";
const LABEL: &str = "dev.gitterrace.server";
/// First line of a unit we wrote, and the plist key that marks a `LaunchAgent` as ours.
const UNIT_MARKER: &str =
    "# Written by git-terrace service install. Remove it with: git-terrace service uninstall";
const PLIST_KEY: &str = "GitterraceService";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Systemd,
    Launchd,
}

impl Kind {
    fn here() -> anyhow::Result<Self> {
        if cfg!(windows) {
            bail!("service is not supported on Windows, see docs/service.md");
        }
        Ok(if cfg!(target_os = "macos") { Self::Launchd } else { Self::Systemd })
    }
}

/// What the unit runs.
pub(crate) struct Spec {
    pub(crate) exe: PathBuf,
    pub(crate) paths: Vec<PathBuf>,
    pub(crate) addr: Option<SocketAddr>,
    /// macOS log file, unused by systemd.
    pub(crate) log: PathBuf,
}

impl Spec {
    fn args(&self) -> Vec<String> {
        let mut a = vec![self.exe.display().to_string(), "serve".to_owned()];
        if let Some(addr) = self.addr {
            a.push("--addr".to_owned());
            a.push(addr.to_string());
        }
        a.extend(self.paths.iter().map(|p| p.display().to_string()));
        a
    }
}

fn systemd_quote(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '"' => o.push_str("\\\""),
            '%' => o.push_str("%%"),
            '$' => o.push_str("$$"),
            _ => o.push(c),
        }
    }
    o.push('"');
    o
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

pub(crate) fn render_systemd(spec: &Spec) -> String {
    let exec: Vec<String> = spec.args().iter().map(|a| systemd_quote(a)).collect();
    format!(
        "{UNIT_MARKER}\n[Unit]\nDescription=gitterrace, a local read-only git browser\n\n\
         [Service]\nExecStart={}\nRestart=on-failure\nRestartSec=5\n\n\
         [Install]\nWantedBy=default.target\n",
        exec.join(" ")
    )
}

pub(crate) fn render_plist(spec: &Spec) -> String {
    let mut args = String::new();
    for a in spec.args() {
        let _ = writeln!(args, "\t\t<string>{}</string>", xml_escape(&a));
    }
    let log = xml_escape(&spec.log.display().to_string());
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
         \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n<dict>\n\
         \t<key>Label</key>\n\t<string>{LABEL}</string>\n\
         \t<key>{PLIST_KEY}</key>\n\t<true/>\n\
         \t<key>ProgramArguments</key>\n\t<array>\n{args}\t</array>\n\
         \t<key>RunAtLoad</key>\n\t<true/>\n\
         \t<key>KeepAlive</key>\n\t<dict>\n\t\t<key>SuccessfulExit</key>\n\t\t<false/>\n\t</dict>\n\
         \t<key>StandardOutPath</key>\n\t<string>{log}</string>\n\
         \t<key>StandardErrorPath</key>\n\t<string>{log}</string>\n\
         </dict>\n</plist>\n"
    )
}

fn unit_path(home: &Path, kind: Kind) -> PathBuf {
    match kind {
        Kind::Systemd => home.join(".config/systemd/user").join(UNIT_NAME),
        Kind::Launchd => home.join("Library/LaunchAgents").join(format!("{LABEL}.plist")),
    }
}

fn render(spec: &Spec, kind: Kind) -> String {
    match kind {
        Kind::Systemd => render_systemd(spec),
        Kind::Launchd => render_plist(spec),
    }
}

fn is_ours(file: &Path, kind: Kind) -> bool {
    std::fs::read_to_string(file).is_ok_and(|t| match kind {
        Kind::Systemd => t.starts_with("# Written by git-terrace service install"),
        Kind::Launchd => t.contains(PLIST_KEY),
    })
}

/// The command that enables and starts the service.
fn start_command(file: &Path, kind: Kind) -> Vec<String> {
    match kind {
        Kind::Systemd => {
            ["systemctl", "--user", "enable", "--now", UNIT_NAME].map(String::from).to_vec()
        }
        Kind::Launchd => vec![
            "launchctl".to_owned(),
            "load".to_owned(),
            "-w".to_owned(),
            file.display().to_string(),
        ],
    }
}

fn stop_command(file: &Path, kind: Kind) -> Vec<String> {
    match kind {
        Kind::Systemd => {
            ["systemctl", "--user", "disable", "--now", UNIT_NAME].map(String::from).to_vec()
        }
        Kind::Launchd => vec![
            "launchctl".to_owned(),
            "unload".to_owned(),
            "-w".to_owned(),
            file.display().to_string(),
        ],
    }
}

fn run_command(cmd: &[String]) -> anyhow::Result<()> {
    let (prog, args) = cmd.split_first().context("empty command")?;
    let out = Command::new(prog).args(args).output().with_context(|| format!("running {prog}"))?;
    if !out.status.success() {
        bail!("{} failed: {}", cmd.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

#[expect(
    clippy::print_stdout,
    reason = "user-facing CLI output: this is the command's result or diagnostic, not a log line"
)]
pub(crate) fn run(
    action: Action,
    now: bool,
    paths: &[PathBuf],
    addr: Option<SocketAddr>,
) -> anyhow::Result<()> {
    let kind = Kind::here()?;
    let home = gitterrace_config::home_var()
        .map(PathBuf::from)
        .context("the home folder variable (HOME) is not set")?;
    let file = unit_path(&home, kind);
    if action == Action::Uninstall {
        return uninstall(&file, kind, now);
    }
    let exe = std::env::current_exe().context("finding this program")?;
    let paths = paths
        .iter()
        .map(|p| {
            std::fs::canonicalize(p).with_context(|| format!("no such folder: {}", p.display()))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let spec = Spec { exe, paths, addr, log: home.join("Library/Logs/gitterrace.log") };
    let text = render(&spec, kind);
    if action == Action::Print {
        print!("{text}");
        return Ok(());
    }
    if file.exists() && !is_ours(&file, kind) {
        bail!("{} exists and was not written by this command, left as it is", file.display());
    }
    write_unit(&file, &text, spec.log.parent().filter(|_| kind == Kind::Launchd))?;
    println!("wrote {}", file.display());
    let cmd = start_command(&file, kind);
    if now {
        run_command(&cmd)?;
        println!("ran: {}", cmd.join(" "));
    } else {
        println!("start it at login and now with: {}", cmd.join(" "));
    }
    println!("remove with: git-terrace service uninstall");
    Ok(())
}

fn write_unit(file: &Path, text: &str, log_dir: Option<&Path>) -> anyhow::Result<()> {
    std::fs::create_dir_all(file.parent().context("no parent folder")?)?;
    if let Some(d) = log_dir {
        std::fs::create_dir_all(d)?;
    }
    let dir = file.parent().context("no parent folder")?;
    let name = file.file_name().and_then(|n| n.to_str()).context("no file name")?;
    gitterrace_git::write_atomic(dir, name, text.as_bytes(), 0o644)
        .with_context(|| format!("writing {}", file.display()))
}

#[expect(
    clippy::print_stdout,
    reason = "user-facing CLI output: this is the command's result or diagnostic, not a log line"
)]
fn uninstall(file: &Path, kind: Kind, now: bool) -> anyhow::Result<()> {
    if !file.exists() {
        println!("nothing to remove: {} does not exist", file.display());
        return Ok(());
    }
    if !is_ours(file, kind) {
        bail!("{} was not written by this command, left as it is", file.display());
    }
    if now {
        // Best effort: the service may not be running.
        let _ = run_command(&stop_command(file, kind));
    }
    std::fs::remove_file(file).with_context(|| format!("removing {}", file.display()))?;
    println!("removed {}", file.display());
    if !now {
        println!("stop it first, if running, with: {}", stop_command(file, kind).join(" "));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    fn spec(paths: &[&str]) -> Spec {
        Spec {
            exe: PathBuf::from("/opt/my tools/git-terrace"),
            paths: paths.iter().map(PathBuf::from).collect(),
            addr: None,
            log: PathBuf::from("/Users/a b/Library/Logs/gitterrace.log"),
        }
    }

    const NASTY: &[&str] = &["/srv/a b/repos", "/srv/q\"uote/x", "/srv/50%/$HOME/a&b<c>"];

    #[test]
    fn systemd_unit_escapes_spaces_quotes_and_specifiers() {
        let u = render_systemd(&spec(NASTY));
        assert!(u.contains("\"/srv/a b/repos\""), "{u}");
        assert!(u.contains("\"/srv/q\\\"uote/x\""), "{u}");
        assert!(u.contains("\"/srv/50%%/$$HOME/a&b<c>\""), "{u}");
        assert!(u.contains("\"/opt/my tools/git-terrace\" \"serve\""), "{u}");
        assert!(!u.contains("--public"));
        assert!(u.contains("Restart=on-failure") && u.contains("WantedBy=default.target"));
    }

    #[test]
    fn plist_escapes_xml_and_keeps_paths_whole() {
        let p = render_plist(&spec(NASTY));
        assert!(p.contains("<string>/srv/a b/repos</string>"), "{p}");
        assert!(p.contains("<string>/srv/q&quot;uote/x</string>"), "{p}");
        assert!(p.contains("<string>/srv/50%/$HOME/a&amp;b&lt;c&gt;</string>"), "{p}");
        assert!(!p.contains("--public"));
        assert!(p.contains("RunAtLoad") && p.contains("StandardErrorPath"));
    }

    #[test]
    fn addr_is_passed_only_when_given() {
        let mut s = spec(&["/r"]);
        assert!(!s.args().contains(&"--addr".to_owned()));
        s.addr = Some("127.0.0.1:9000".parse().unwrap_or_else(|_| unreachable!()));
        assert!(s.args().windows(2).any(|w| w[0] == "--addr" && w[1] == "127.0.0.1:9000"));
    }

    /// Without a system tool, a strict parse: every line is a comment, a section header or
    /// `Key=value` with a known key.
    fn strict_unit_parse(u: &str) {
        let keys: BTreeSet<&str> =
            ["Description", "ExecStart", "Restart", "RestartSec", "WantedBy"].into();
        for l in u.lines().filter(|l| !l.is_empty() && !l.starts_with('#')) {
            if l.starts_with('[') {
                assert!(matches!(l, "[Unit]" | "[Service]" | "[Install]"), "{l}");
            } else {
                let (k, v) = l.split_once('=').unwrap_or_else(|| panic!("not key=value: {l}"));
                assert!(keys.contains(k) && !v.is_empty(), "{l}");
            }
        }
    }

    fn tool_ok(tool: &str, args: &[&str]) -> Option<bool> {
        Command::new(tool).args(args).output().ok().map(|o| o.status.success())
    }

    #[test]
    fn rendered_unit_and_plist_parse() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let s = spec(NASTY);
        strict_unit_parse(&render_systemd(&s));
        let plist = dir.path().join("a.plist");
        std::fs::write(&plist, render_plist(&s)).unwrap_or_else(|e| panic!("{e}"));
        if let Some(ok) = tool_ok("plutil", &["-lint", &plist.display().to_string()]) {
            assert!(ok, "plutil -lint refused the plist");
        }
        // The unit names /opt/my tools/git-terrace, which does not exist, so verify a unit
        // that runs a real binary.
        let unit = dir.path().join("t.service");
        let mut real = spec(&["/tmp"]);
        real.exe = PathBuf::from("/bin/sh");
        std::fs::write(&unit, render_systemd(&real)).unwrap_or_else(|e| panic!("{e}"));
        if let Some(ok) = tool_ok("systemd-analyze", &["verify", &unit.display().to_string()]) {
            assert!(ok, "systemd-analyze verify refused the unit");
        }
    }

    #[test]
    fn uninstall_removes_only_our_file_inside_the_home() {
        let home = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        for kind in [Kind::Systemd, Kind::Launchd] {
            let file = unit_path(home.path(), kind);
            write_unit(&file, "something else\n", None).unwrap_or_else(|e| panic!("{e}"));
            assert!(uninstall(&file, kind, false).is_err());
            assert!(file.exists());
            write_unit(&file, &render(&spec(&["/r"]), kind), None)
                .unwrap_or_else(|e| panic!("{e}"));
            uninstall(&file, kind, false).unwrap_or_else(|e| panic!("{e}"));
            assert!(!file.exists());
            assert!(file.starts_with(home.path()));
        }
    }
}
