//! HTTP server: server-rendered pages over `gitterrace-git` and `gitterrace-index`.

mod about;
mod actions;
mod activity;
mod add;
mod app;
mod bounded;
mod caches;
mod charts;
mod charttip;
mod commit_line;
mod commit_types;
mod compare;
mod dochead;
mod empty;
mod error;
mod far;
mod features;
mod filter;
mod fmt;
mod frequency;
mod gitstate;
mod guard;
mod highlight;
mod history;
mod hotspots;
mod insights;
mod languages;
mod lead;
#[cfg(test)]
mod limits_wiring_tests;
mod live;
mod lost;
mod markdown;
mod media;
mod multi;
#[cfg(test)]
mod multi_tests;
mod overview_side;
mod ownership;
mod patterns;
mod public;
mod registry;
mod reload;
mod routes;
mod search;
mod sections;
mod seg;
mod settings;
mod settings_actions;
mod settings_page;
pub mod sitemap;
mod spark;
#[cfg(test)]
mod state_tests;
mod views;
mod when;
#[cfg(test)]
mod wiring_fs_tests;
#[cfg(test)]
mod wiring_live_tests;
#[cfg(test)]
mod wiring_tests;

use std::{
    io,
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    sync::Arc,
};

pub use add::TOKEN_HEADER as ADD_TOKEN_HEADER;
use gitterrace_config::Config;
use gitterrace_git::Repo;
use gitterrace_index::Index;
pub use guard::CSP as PAGE_CSP;
pub use highlight::languages as highlighted_languages;
pub use media::plain_media;
pub use multi::discover_repos;
pub use registry::{
    Entry as RegistryEntry, add_token, candidates as registry_candidates, state_dir,
};
pub use reload::{Load, Watched};
pub use views::stats_tab_labels;

use crate::app::{App, Env};

/// The full router `serve` uses: pages and `/api/v1`. Starts the index
/// refresher, so call it inside a tokio runtime. Drive it in-process with tower's `oneshot`.
///
/// Requests must carry a loopback `Host` header (`localhost`, `127.0.0.1`, `[::1]`), which
/// stops DNS-rebinding pages from reading the repository. In-process callers send
/// `Host: localhost`.
pub fn app_router(repo_path: impl AsRef<Path>) -> io::Result<axum::Router> {
    app_router_with(repo_path, Config::default())
}

/// [`app_router`] with the settings in force.
pub fn app_router_with(repo_path: impl AsRef<Path>, cfg: Config) -> io::Result<axum::Router> {
    Ok(build(repo_path.as_ref(), None, None, &Env::new(cfg))?.0)
}

/// Router for one or more repositories, each under `/r/NAME`; `/` forwards to a lone one and
/// indexes several. Same Host rules. Names come from folder names, not the registry.
pub fn multi_router(paths: Vec<PathBuf>) -> io::Result<axum::Router> {
    Ok(build_multi(paths, None, None, &Env::new(Config::default()))?.0)
}

/// [`multi_router`] for one export run: every repository shares commit tree diffs through
/// `changes`, which the caller owns and drops when the run ends, freeing them all.
pub fn export_router(
    paths: Vec<PathBuf>,
    changes: &Arc<gitterrace_git::ChangeMemo>,
    cfg: Config,
) -> io::Result<axum::Router> {
    let mut env = Env::new(cfg);
    env.changes = Some(Arc::clone(changes));
    Ok(build_multi(paths, None, None, &env)?.0)
}

/// One repository's pages and `/api/v1`, with links under `base`. Starts its
/// index refresher, so call it inside a tokio runtime.
#[cfg(test)]
fn repo_router(
    repo_path: &Path,
    base: String,
    key: String,
    live: Option<Arc<actions::Live>>,
    env: &Env,
) -> io::Result<axum::Router> {
    Ok(repo_router_tasks(repo_path, base, key, live, env)?.0)
}

/// [`repo_router`] and the handles of the background tasks it started (index refresher, idle
/// release, warm-up), so a repository taken off a running server stops them and its memory
/// returns.
fn repo_router_tasks(
    repo_path: &Path,
    base: String,
    key: String,
    live: Option<Arc<actions::Live>>,
    env: &Env,
) -> io::Result<(axum::Router, Vec<tokio::task::AbortHandle>)> {
    let cfg = env.cfg();
    let (text_bytes, blame_bytes) = app::repo_limits(&cfg);
    let repo = Repo::discover(repo_path)
        .map_err(io::Error::other)?
        .with_limits(text_bytes, blame_bytes)
        .with_workers(cfg.index.threads);
    if env.public() {
        repo.restrict_to_committed();
    }
    if let Some(memo) = &env.changes {
        repo.use_change_memo(memo);
    }
    repo.set_worktree_limits(
        cfg.limits.worktree_files,
        (cfg.limits.worktree_diff_kb as u64) << 10,
    );
    let index = Index::open(&repo)?.with_fresh_cap(cfg.limits.unindexed_commits);
    index.set_block_cache(cfg.cache_bytes(gitterrace_config::Cache::Blocks));
    let mut app = App::configured(repo, index, env);
    app.base = base;
    app.key = key;
    app.actions = live;
    let app = Arc::new(app);
    env.register(&app);
    let mut tasks = vec![
        tokio::spawn(app::keep_index_fresh(Arc::clone(&app))).abort_handle(),
        tokio::spawn(app::release_when_idle(
            Arc::downgrade(&app),
            app::RECORDS_IDLE,
        ))
        .abort_handle(),
    ];
    let api = gitterrace_api::ApiState::open(repo_path)
        .map_err(io::Error::other)?
        .configured(&env.cfg());
    let api = if env.public() {
        api.committed_only()
    } else {
        api
    };
    // A reload reaches the API through the app: both count with the same `insights.*`.
    app.set_api(api.clone());
    let pages = routes::router(Arc::clone(&app))
        .nest("/api/v1", gitterrace_api::router(api))
        .route_layer(axum::middleware::from_fn_with_state(
            Arc::clone(&app),
            features::repo_gate,
        ));
    let pages = if env.public() {
        pages.route_layer(axum::middleware::from_fn(public::only_public))
    } else {
        pages
    };
    tasks.push(tokio::spawn(prewarm(Arc::clone(&app), pages.clone())).abort_handle());
    Ok((pages, tasks))
}

/// A failed bind with the address and, when the port is taken, what to do about it. The kind
/// is kept, so a caller can still tell the cases apart.
fn bind_error(addr: SocketAddr, e: &io::Error) -> io::Error {
    let hint = if e.kind() == io::ErrorKind::AddrInUse {
        "; another git terrace or other server is using that port: stop it, or pass --addr with another port (127.0.0.1:0 picks a free one)"
    } else {
        ""
    };
    io::Error::new(e.kind(), format!("cannot listen on {addr}: {e}{hint}"))
}

/// Unfiltered pages rendered in the background whenever HEAD moves.
///
/// Each costs 40-90 ms to
/// build on a 14k-commit repo, then a few ms from the memo, so no visitor pays the build.
/// `export` seeds its crawl from the same list, so a static copy has every insights page.
pub const WARM_PAGES: [&str; 6] = [
    "/",
    "/stats",
    "/stats/languages",
    "/stats/contributors",
    "/stats/hotspots",
    "/stats/ownership",
];

/// The diagram library: a page's script loads it by name, so no link names it.
pub const MERMAID_JS: &str = "/static/vendor/mermaid-11.17.2.min.js";

/// [`WARM_PAGES`] without the pages `cfg` switches off, for `export` to seed its crawl from.
pub fn warm_pages(cfg: &Config) -> Vec<&'static str> {
    features::warm(cfg, None)
}

/// Opens the insights source for HEAD each time the index moves, then renders the unfiltered
/// pages once, so the first visit to any of them is served from the memo. Sleeps between moves.
async fn prewarm(app: Arc<App>, pages: axum::Router) {
    use tower::ServiceExt;
    let by = Arc::clone(&app);
    app::on_refreshed(app, move || {
        let (app, pages) = (Arc::clone(&by), pages.clone());
        async move {
            // The warm-up is no visitor: what it opens goes one idle period after it started.
            let _warm = app.warming();
            let ready = app::blocking(&app, |a| {
                let head = a.repo.resolve("HEAD")?;
                Ok(a.source(&head)?.is_some())
            })
            .await
            .unwrap_or(false);
            if !ready {
                return;
            }
            let cfg = app.cfg();
            for path in features::warm(&cfg, Some(&app.name)) {
                // Internal requests skip the Host and cross-site guard, which wraps the outer
                // router; a failed warm-up only means the first visitor builds the page.
                if let Ok(req) = axum::http::Request::get(path).body(axum::body::Body::empty()) {
                    let _warm = pages.clone().oneshot(req).await;
                }
            }
            if cfg.feature_on(gitterrace_config::Feature::Blame, Some(&app.name)) {
                routes::prewarm_blame(app).await;
            }
        }
    })
    .await;
}

fn guarded(router: axum::Router, gate: guard::Gate) -> axum::Router {
    use tower_http::compression::{CompressionLayer, Predicate, predicate::NotForContentType};
    // Fonts and images are already compressed; the default predicate also skips tiny bodies.
    let worth_it = tower_http::compression::DefaultPredicate::new()
        .and(NotForContentType::new("font/"))
        .and(NotForContentType::IMAGES);
    router
        .layer(
            CompressionLayer::new()
                .gzip(true)
                .br(true)
                .compress_when(worth_it),
        )
        .layer(axum::middleware::from_fn_with_state(
            Arc::new(gate),
            guard::guard,
        ))
}

fn build(
    repo_path: &Path,
    extra: Option<IpAddr>,
    actions_file: Option<&Path>,
    env: &Env,
) -> io::Result<(axum::Router, String)> {
    build_server(
        vec![repo_path.to_path_buf()],
        extra,
        actions_file,
        env,
        None,
        None,
    )
}

fn build_multi(
    paths: Vec<PathBuf>,
    extra: Option<IpAddr>,
    actions_file: Option<&Path>,
    env: &Env,
) -> io::Result<(axum::Router, String)> {
    build_server(paths, extra, actions_file, env, None, None)
}

/// The whole server: a repository table, every repository under `/r/NAME`, so `add` (the secret
/// `POST /_repos` checks, `None` leaves the route unmounted) grows it without moving a link.
/// `state` is where names and additions are recorded.
fn build_server(
    paths: Vec<PathBuf>,
    extra: Option<IpAddr>,
    actions_file: Option<&Path>,
    env: &Env,
    add: Option<gitterrace_actions::Token>,
    state: Option<PathBuf>,
) -> io::Result<(axum::Router, String)> {
    let n = paths.len();
    let mut table = multi::Table::new(paths, env.clone(), state.clone())?;
    let targets = table.targets()?;
    let roots: Vec<PathBuf> = targets.iter().map(|t| t.repo.root()).collect();
    // A public server never reads actions.toml: no run route, no Send to targets.
    let actions_file = actions_file.filter(|_| !env.public());
    if env.public() {
        for t in &targets {
            t.repo.restrict_to_committed();
        }
    }
    // Behind a proxy a public server sits on loopback and every request is remote.
    let local = extra.is_none() && !env.public();
    let output_kb = env.cfg().limits.action_output_kb;
    let (live, status) = actions::start(actions_file, &roots, local, output_kb);
    env.set_actions_status(&status);
    table.live.clone_from(&live);
    let sends = actions::sends(live.as_ref(), actions_file, &roots);
    env.set_sends(&sends);
    let site = Arc::new(actions::Site {
        targets: std::sync::RwLock::new(targets.into_iter().map(Arc::new).collect()),
        live: live.clone(),
        status,
        sends,
        env: env.clone(),
    });
    let table = Arc::new(table);
    table.start_all();
    let router = multi::router(Arc::clone(&table));
    let shown = fmt::count(n, "repository", "repositories");
    let mut router = router.merge(actions::router(Arc::clone(&site)));
    // A non-loopback bind or a public server never takes additions: the token is the only
    // gate a remote client would face.
    if let Some(token) = add.filter(|_| local) {
        router = router.merge(add::router(Arc::new(add::Adder {
            table,
            site,
            token,
            actions_file: actions_file.map(Path::to_path_buf),
            state,
        })));
    }
    if settings::mountable(env.public(), local, env.settings_dir.is_some())
        && let Some(dir) = env.settings_dir.clone()
    {
        match gitterrace_actions::Token::generate() {
            Ok(token) => {
                env.set_settings_on();
                router = router.merge(settings::router(Arc::new(settings::Settings {
                    dir,
                    actions: actions_file.map(Path::to_path_buf),
                    token,
                    env: env.clone(),
                })));
            }
            Err(e) => tracing::warn!("settings page off: no randomness for the token: {e}"),
        }
    }
    let router = router.route_layer(axum::middleware::from_fn_with_state(
        env.clone(),
        features::server_gate,
    ));
    let router = if env.public() {
        router.route_layer(axum::middleware::from_fn(public::only_public))
    } else {
        router
    };
    let gate = guard::Gate::new(extra, &env.cfg());
    Ok((guarded(router, gate), shown))
}

/// One line per per-request cost setting raised above its default on a public server: config is
/// the operator's, so this only says what a visitor can now make one request cost.
fn public_cost_warnings(cfg: &Config) -> Vec<String> {
    if !cfg.server.is_public {
        return Vec::new();
    }
    cfg.costs_above_default()
        .into_iter()
        .map(|k| format!("public server: {k} is above its default, so one request may cost more"))
        .collect()
}

/// Records the served repositories for `git terrace open`. Best effort: a read-only or odd
/// state folder must not stop the server, so a failure is only logged.
fn remember(dir: Option<&Path>, paths: &[PathBuf]) {
    let Some(dir) = dir else {
        return;
    };
    let repos: Vec<Repo> = paths
        .iter()
        .filter_map(|p| Repo::discover(p).ok())
        .collect();
    let refs: Vec<&Repo> = repos.iter().collect();
    if let Err(e) = registry::record(dir, &refs) {
        tracing::warn!(
            "not recording served repositories in {}: {e}",
            dir.display()
        );
    }
}

/// Starts serving until ctrl-c.
///
/// `actions` is the user's `actions.toml`; `None` (no config
/// folder, or `--no-actions`) leaves the run route unmounted, as does a non-loopback bind.
/// `watched` is the `config.toml` to reload while running, see [`Watched`].
pub async fn serve(
    repo_paths: Vec<PathBuf>,
    addr: SocketAddr,
    actions: Option<PathBuf>,
    cfg: Config,
    watched: Option<Watched>,
) -> io::Result<()> {
    serve_in(
        repo_paths,
        addr,
        actions,
        cfg,
        watched,
        registry::state_dir(),
    )
    .await
}

/// [`serve`] with the state folder (registry and add secret) named, `None` for none.
pub async fn serve_in(
    repo_paths: Vec<PathBuf>,
    addr: SocketAddr,
    actions: Option<PathBuf>,
    cfg: Config,
    watched: Option<Watched>,
    state: Option<PathBuf>,
) -> io::Result<()> {
    refuse_lan(addr, &cfg)?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| bind_error(addr, &e))?;
    serve_on(listener, repo_paths, actions, cfg, watched, state).await
}

/// [`serve_in`] on a listener already bound, so a caller that picked port 0 keeps the port
/// it was given instead of racing another process for it.
pub async fn serve_on(
    listener: tokio::net::TcpListener,
    repo_paths: Vec<PathBuf>,
    actions: Option<PathBuf>,
    cfg: Config,
    watched: Option<Watched>,
    state: Option<PathBuf>,
) -> io::Result<()> {
    let addr = listener.local_addr()?;
    refuse_lan(addr, &cfg)?;
    let extra = (!addr.ip().is_loopback()).then(|| addr.ip());
    let public = cfg.server.is_public;
    for k in public_cost_warnings(&cfg) {
        tracing::warn!("{k}");
    }
    registry::set_entries(cfg.limits.registry_entries);
    tokio::task::spawn_blocking({
        let (paths, state) = (repo_paths.clone(), state.clone());
        move || remember(state.as_deref(), &paths)
    });
    let mut env = Env::new(cfg);
    // The settings page edits the folder `config.toml` is reloaded from, and nothing else.
    env.settings_dir = watched
        .as_ref()
        .and_then(|w| w.file.parent().map(Path::to_path_buf));
    // The secret `open` reads to add a repository. Without a state folder, or when it cannot
    // be written, the route stays unmounted and `open` starts a server of its own.
    let add = match (&state, extra) {
        (Some(dir), None) if !public => publish_add_token(dir, addr.port()),
        _ => None,
    };
    let published = add.is_some();
    let (router, name) = build_server(
        repo_paths,
        extra,
        actions.as_deref(),
        &env,
        add,
        state.clone(),
    )?;
    if let Some(w) = watched {
        tokio::spawn(reload::run(reload::Reloader::new(w), env.clone()));
    }
    if public {
        tracing::warn!(
            "public: anyone who can reach this address reads the committed history. Working tree, stashes, live updates and actions are off"
        );
    }
    tracing::info!("serving {name} on http://{addr}");
    let served = axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            if tokio::signal::ctrl_c().await.is_err() {
                std::future::pending::<()>().await;
            }
            // Open tabs hold their streams; closing them lets the server stop.
            env.shut_down();
        })
        .await;
    if let (true, Some(dir)) = (published, &state) {
        registry::remove_add_token(dir, addr.port());
    }
    served
}

/// Refuses an address other machines reach unless the config says public. Also checked by
/// `Config::validate`; repeated here because `serve` takes any `Config`.
fn refuse_lan(addr: SocketAddr, cfg: &Config) -> io::Result<()> {
    if addr.ip().is_loopback() || cfg.server.is_public {
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!(
            "{addr} is reachable from other machines. Pass --public (server.is_public = true) to serve read-only there"
        ),
    ))
}

/// A fresh secret for `POST /_repos`, written where only this user can read it.
fn publish_add_token(dir: &Path, port: u16) -> Option<gitterrace_actions::Token> {
    let token = gitterrace_actions::Token::generate()
        .map_err(|e| tracing::warn!("no add token: {e}"))
        .ok()?;
    match registry::write_add_token(dir, port, token.as_str()) {
        Ok(()) => Some(token),
        Err(e) => {
            tracing::warn!("adding repositories is off: {}: {e}", dir.display());
            None
        }
    }
}

#[cfg(test)]
mod tests;
