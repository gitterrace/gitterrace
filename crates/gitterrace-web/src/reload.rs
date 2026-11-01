//! Live reload of `config.toml`: one `stat` per tick, a re-parse only when the file's mtime or
//! size moved. An invalid edit keeps the settings in force and names the key in the log.

use std::{
    fs,
    path::PathBuf,
    time::{Duration, SystemTime},
};

use gitterrace_config::{Config, ConfigError};

use crate::app::Env;

/// How often `config.toml` is checked, the same beat as `actions.toml`.
pub(crate) const POLL: Duration = Duration::from_secs(1);

/// Re-reads the settings the way startup did, flags included, so a flag still beats the file.
pub type Load = Box<dyn Fn() -> Result<Config, ConfigError> + Send + Sync>;

/// A `config.toml` to watch and how to load it.
pub struct Watched {
    pub file: PathBuf,
    pub load: Load,
}

type Stamp = Option<(SystemTime, u64)>;

pub(crate) struct Reloader {
    watched: Watched,
    seen: Stamp,
}

impl Reloader {
    /// Takes the file as it is now as already applied: startup just read it.
    pub(crate) fn new(watched: Watched) -> Self {
        let seen = stamp(&watched.file);
        Self { watched, seen }
    }

    /// Applies an edit if there is one. Returns the log lines to print.
    pub(crate) fn poll(&mut self, env: &Env) -> Vec<String> {
        let now = stamp(&self.watched.file);
        if now == self.seen {
            return Vec::new();
        }
        self.seen = now;
        let cfg = match (self.watched.load)() {
            Ok(c) => c,
            Err(e) => {
                return vec![format!(
                    "config not reloaded, keeping the old settings: {e}"
                )];
            }
        };
        let mut lines = vec!["config reloaded".to_owned()];
        for key in env.apply(cfg) {
            lines.push(format!("config: {key} changed, restart to apply it"));
        }
        lines
    }
}

fn stamp(file: &std::path::Path) -> Stamp {
    let m = fs::metadata(file).ok()?;
    Some((m.modified().ok()?, m.len()))
}

pub(crate) async fn run(mut reloader: Reloader, env: Env) {
    loop {
        tokio::time::sleep(POLL).await;
        let env = env.clone();
        let Ok((back, lines)) = tokio::task::spawn_blocking(move || {
            let lines = reloader.poll(&env);
            (reloader, lines)
        })
        .await
        else {
            return;
        };
        reloader = back;
        for l in lines {
            if l.contains("not reloaded") || l.contains("restart") {
                tracing::warn!("{l}");
            } else {
                tracing::info!("{l}");
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use std::{path::Path, sync::Arc};

    use gitterrace_config::Config;

    use super::{Reloader, Watched};
    use crate::app::{App, Env};

    fn git(dir: &Path, args: &[&str]) {
        let ok = std::process::Command::new("git")
            .args(["-c", "user.name=T", "-c", "user.email=t@e.x"])
            .args(args)
            .current_dir(dir)
            .status()
            .is_ok_and(|s| s.success());
        assert!(ok, "git {args:?}");
    }

    struct Fixture {
        _repo: tempfile::TempDir,
        dir: tempfile::TempDir,
        env: Env,
        app: Arc<App>,
        reloader: Reloader,
    }

    fn fixture(text: &str) -> Fixture {
        let repo = tempfile::tempdir().expect("tmp");
        git(repo.path(), &["init", "-q", "-b", "main"]);
        std::fs::write(repo.path().join("a.txt"), "1\n").expect("write");
        git(repo.path(), &["add", "."]);
        git(repo.path(), &["commit", "-q", "-m", "feat: one"]);
        let dir = tempfile::tempdir().expect("tmp");
        std::fs::write(dir.path().join("config.toml"), text).expect("write");
        let load_dir = dir.path().to_path_buf();
        let load = move || gitterrace_config::load(Some(&load_dir)).map(|l| l.config);
        let env = Env::new(load().expect("config"));
        let r = gitterrace_git::Repo::discover(repo.path()).expect("repo");
        let index = gitterrace_index::Index::open(&r).expect("index");
        let app = Arc::new(App::configured(r, index, &env));
        env.register(&app);
        let reloader = Reloader::new(Watched {
            file: dir.path().join("config.toml"),
            load: Box::new(load),
        });
        Fixture {
            _repo: repo,
            dir,
            env,
            app,
            reloader,
        }
    }

    fn edit(f: &Fixture, text: &str) {
        std::fs::write(f.dir.path().join("config.toml"), text).expect("write");
    }

    #[test]
    fn a_valid_edit_applies_on_the_next_poll_and_tells_open_tabs() {
        let mut f = fixture("");
        assert!(f.reloader.poll(&f.env).is_empty(), "no edit, no work");
        let mut tab = f.app.changes.subscribe();
        f.app.memo_put("k".into(), Arc::new("page".into()));
        edit(&f, "[insights]\nperiod = \"1y\"\n[limits]\ntext_kb = 7\n");
        f.reloader.poll(&f.env);
        assert_eq!(f.env.cfg().insights.period, "1y");
        assert_eq!(f.app.cfg().limits.text_kb, 7);
        assert_eq!(
            f.app.repo.text_limit(),
            7 << 10,
            "the repository handle follows"
        );
        assert!(f.app.memo_get("k").is_none(), "rendered pages are dropped");
        let c = tab.try_recv().expect("config event");
        assert_eq!(c.kinds, ["config"]);
    }

    #[test]
    fn an_invalid_edit_keeps_the_old_values_and_names_the_key() {
        let mut f = fixture("[insights]\nperiod = \"1y\"\n");
        edit(&f, "[insights]\nperiod = \"forever\"\n");
        let lines = f.reloader.poll(&f.env);
        assert!(
            lines.iter().any(|l| l.contains("insights.period")),
            "{lines:?}"
        );
        assert_eq!(f.env.cfg().insights.period, "1y");
        assert!(
            f.reloader.poll(&f.env).is_empty(),
            "a bad file is not retried unchanged"
        );
        edit(&f, "[insights]\nperiod = \"3m\"\n");
        f.reloader.poll(&f.env);
        assert_eq!(
            f.env.cfg().insights.period,
            "3m",
            "the next good edit applies"
        );
    }

    #[test]
    fn a_restart_only_key_warns_and_does_not_change() {
        let mut f = fixture("");
        edit(
            &f,
            "[server]\naddr = \"127.0.0.1:9\"\n[index]\nthreads = 3\n[multi]\ndepth = 4\n\
             skip_dirs = [\"x\"]\n[display]\ndates = \"absolute\"\n",
        );
        let lines = f.reloader.poll(&f.env).join("\n");
        for key in [
            "server.addr",
            "index.threads",
            "multi.depth",
            "multi.skip_dirs",
        ] {
            assert!(lines.contains(key), "{key} not named in {lines}");
        }
        let cfg = f.env.cfg();
        let d = Config::default();
        assert_eq!(cfg.server.addr, d.server.addr);
        assert_eq!(cfg.index.threads, d.index.threads);
        assert_eq!(cfg.multi, d.multi);
        assert_eq!(
            cfg.display.dates, "absolute",
            "safe keys in the same edit still apply"
        );
    }
}
