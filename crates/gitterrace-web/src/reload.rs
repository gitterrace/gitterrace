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
const POLL: Duration = Duration::from_secs(1);

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
            lines.push(format!(
                "config: {key} changed but needs a restart. The running value stays in force"
            ));
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
mod tests {
    use gitterrace_testkit::git;
    use std::sync::Arc;

    use gitterrace_config::Config;
    use gitterrace_types::Period;

    use super::{Reloader, Watched};
    use crate::app::{App, Env};

    struct Fixture {
        repo: tempfile::TempDir,
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
            repo,
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
        assert_eq!(f.env.cfg().insights.period, Period::Year);
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
        assert_eq!(f.env.cfg().insights.period, Period::Year);
        assert!(
            f.reloader.poll(&f.env).is_empty(),
            "a bad file is not retried unchanged"
        );
        edit(&f, "[insights]\nperiod = \"3m\"\n");
        f.reloader.poll(&f.env);
        assert_eq!(
            f.env.cfg().insights.period,
            Period::Quarter,
            "the next good edit applies"
        );
    }

    /// `/api/v1` counts under the settings in force, the same ones the pages read.
    #[tokio::test]
    async fn an_insights_edit_reaches_the_api() {
        // No background build, so the index generation in the tag stays put.
        let quiet = "[index]\nbuild_on_start = false\n";
        let mut f = fixture(quiet);
        let router = crate::repo_router(f.repo.path(), String::new(), "r".into(), None, &f.env)
            .expect("router");
        // The entity tag folds in the counting defaults, so it moves exactly when the API's
        // options do, whatever the background index has built so far.
        let tag = |r: axum::Router| async move {
            use tower::ServiceExt as _;
            let req = axum::http::Request::get("/api/v1/repo")
                .header("host", "localhost")
                .body(axum::body::Body::empty())
                .expect("req");
            let res = r.oneshot(req).await.expect("res");
            assert_eq!(res.status(), 200);
            res.headers().get("etag").expect("etag").clone()
        };
        let before = tag(router.clone()).await;
        let same = tag(router.clone()).await;
        assert_eq!(before, same, "nothing changed, same tag");
        edit(&f, &format!("{quiet}[insights]\nexclude = [\"a.txt\"]\n"));
        f.reloader.poll(&f.env);
        assert_ne!(
            before,
            tag(router).await,
            "the API counts under the reloaded options"
        );
        assert_eq!(
            f.app.defaults().exclude,
            gitterrace_api::options_from(&f.env.cfg(), &f.app.name).exclude,
            "pages and API read one options function"
        );
    }

    #[test]
    fn a_restart_only_key_warns_and_does_not_change() {
        let mut f = fixture("");
        edit(
            &f,
            "[server]\naddr = \"127.0.0.1:9\"\nis_public = true\nhosts = [\"git.example.com\"]\n\
             [index]\nthreads = 3\n[multi]\ndepth = 4\n\
             skip_dirs = [\"x\"]\n[display]\ndates = \"absolute\"\n",
        );
        let lines = f.reloader.poll(&f.env).join("\n");
        for key in [
            "server.addr",
            "server.is_public",
            "server.hosts",
            "index.threads",
            "multi.depth",
            "multi.skip_dirs",
        ] {
            assert!(lines.contains(key), "{key} not named in {lines}");
        }
        let cfg = f.env.cfg();
        let d = Config::default();
        assert_eq!(
            cfg.server, d.server,
            "addr, public and hosts keep their running value"
        );
        assert!(!f.app.public(), "public mode is fixed at startup");
        assert_eq!(cfg.index.threads, d.index.threads);
        assert_eq!(cfg.multi, d.multi);
        assert_eq!(
            cfg.display.dates,
            gitterrace_types::DateStyle::Absolute,
            "safe keys in the same edit still apply"
        );
    }
}
