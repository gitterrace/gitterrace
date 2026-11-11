//! Transport-neutral reads. Both the HTTP router and the MCP tools call these,
//! so the two surfaces cannot drift apart. Everything here is blocking.

mod rebuild;

use std::path::Path;
use std::sync::{Arc, Mutex, RwLock};

use gitterrace_actions::{Lines, styled};
use gitterrace_git::Repo;
use gitterrace_index::feed::{Checked, InsightsSource};
use gitterrace_index::{Index, Insights, Options, Selected};
use gitterrace_types::LinkStyle;
use gitterrace_types::api::{
    BlameResponse, BlobResponse, CommitResponse, ContributorsResponse, LogResponse, RefsResponse,
    RepoResponse, RepoTotals, TreeResponse,
};

use crate::error::ApiError;
use crate::{insights, wire};

const DEFAULT_REV: &str = "HEAD";
/// Rev syntax that searches history (`:/text`, `^{/text}`) or reads reflogs (`@{...}`): a
/// request could make one call walk every commit. Branch, tag, sha and `~`/`^` stay allowed.
const REJECTED_REV: [&str; 3] = [":/", "^{/", "@{"];

/// Counting options the configuration sets for the repository named `name`: `insights.*` plus
/// `[repo.NAME] exclude`.
pub fn options_from(cfg: &gitterrace_config::Config, name: &str) -> Options {
    let i = &cfg.insights;
    let mut exclude = i.exclude.clone();
    if let Some(o) = cfg.repo.get(name) {
        exclude.extend(o.exclude.iter().cloned());
    }
    Options {
        exclude,
        include_generated: i.include_generated,
        include_bots: i.include_bots,
        ..Options::default()
    }
}

/// Shared handle on one repository and its insights cache.
#[derive(Clone)]
pub struct ApiState {
    pub(crate) repo: Arc<Repo>,
    pub(crate) index: Arc<Index>,
    /// Insights source of the last request, keyed by (resolved tip, index generation).
    cache: Arc<Mutex<Option<RecordsCache>>>,
    /// What the head tree costs to read, kept for the last commit asked about.
    head: Arc<Mutex<HeadMemo>>,
    /// `links.style` and the counting options a request leaves unset. Shared with every clone,
    /// so a config reload ([`ApiState::reconfigure`]) reaches routers already built.
    settings: Arc<RwLock<Settings>>,
    /// What an unreadable cache sets off: the server's own rebuild when it runs the API, so the
    /// pages and the API share one, else this state's ([`ApiState::rebuild_alone`]).
    on_unreadable: Arc<RwLock<Option<Rebuild>>>,
    /// A rebuild this state started itself is running.
    rebuilding: Arc<std::sync::atomic::AtomicBool>,
}

/// Deletes an unreadable cache and starts its rebuild. Returns at once: the answer meanwhile
/// is a 503 with `Retry-After`.
pub type Rebuild = Arc<dyn Fn() + Send + Sync>;

/// What the configuration decides for the API.
#[derive(Clone, Default)]
struct Settings {
    /// Form of the `link` field; `permalink` is always `/go`.
    style: LinkStyle,
    /// Counting options when a request names none: the `insights` configuration, the same
    /// defaults the web pages start from.
    defaults: Options,
    /// How much one answer may carry, from `[api]` (see `gitterrace_config::Api`): a vendored
    /// commit holds millions of diff lines, one minified line can be megabytes, and a path
    /// filter walks every skipped commit.
    caps: gitterrace_config::Api,
}

/// Per commit id: the ignore files (finding them walks the tree) and the line count of every
/// head file (a walk plus a cache read). Blobs are immutable, so a key never goes stale.
/// `(path, text)` of each ignore file of one commit.
type IgnoreFiles = Arc<Vec<(String, String)>>;

#[derive(Default)]
struct HeadMemo {
    ignores: Option<(String, IgnoreFiles)>,
    lines: Option<(String, Arc<Vec<gitterrace_index::HeadFile>>)>,
}

type RecordsCache = ((String, u64), Checked<dyn InsightsSource>);

impl ApiState {
    pub fn new(repo: Arc<Repo>, index: Arc<Index>) -> Self {
        Self {
            repo,
            index,
            cache: Arc::default(),
            head: Arc::default(),
            settings: Arc::default(),
            on_unreadable: Arc::default(),
            rebuilding: Arc::default(),
        }
    }

    /// Link style and counting defaults from the configuration: `links.style`, `insights.*`
    /// and this repository's `[repo.NAME] exclude`. The web's `/api/v1` and the MCP server both
    /// build their state through here, so the two answer with the same numbers.
    #[must_use]
    pub fn configured(self, cfg: &gitterrace_config::Config) -> Self {
        self.reconfigure(cfg);
        self
    }

    /// Puts `cfg` in force for this state and every clone of it, such as a router built
    /// earlier: a reloaded `config.toml` changes the next answer.
    pub fn reconfigure(&self, cfg: &gitterrace_config::Config) {
        self.index
            .set_block_cache(cfg.cache_bytes(gitterrace_config::Cache::Blocks));
        self.index.set_message_index(cfg.index.has_messages);
        let defaults = options_from(cfg, &self.repo.name());
        let mut s = self.write_settings();
        s.style = cfg.links.style;
        s.defaults = defaults;
        s.caps = cfg.api.clone();
    }

    fn write_settings(&self) -> std::sync::RwLockWriteGuard<'_, Settings> {
        self.settings
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn settings(&self) -> Settings {
        self.settings
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Answers only for committed history, see [`Repo::restrict_to_committed`].
    #[must_use]
    pub fn committed_only(self) -> Self {
        self.repo.restrict_to_committed();
        self
    }

    /// Opens the repository containing `path` and its cache.
    pub fn open(path: &Path) -> Result<Self, ApiError> {
        let repo = Repo::discover(path)?;
        let index = Index::open(&repo)?;
        Ok(Self::new(Arc::new(repo), Arc::new(index)))
    }

    /// Brings the cache up to date with `HEAD`. Blocking; the web layer does this in the
    /// background, the MCP server does it once at start.
    pub fn refresh_index(&self) -> Result<(), ApiError> {
        self.index.update(&*self.repo, DEFAULT_REV, &|_| {})?;
        Ok(())
    }

    /// Entity tag for every answer: HEAD, every ref's target, the cache format version, the
    /// cache generation and the counting defaults. Requests may name any `rev`, so moving any
    /// branch or tag must invalidate, not only a new HEAD commit. Insights answers also change
    /// while the cache fills and when the configured defaults change, with no ref moving.
    pub fn etag(&self) -> Option<String> {
        let head = self.repo.resolve(DEFAULT_REV).ok()?;
        let mut refs: Vec<(String, String)> = self
            .repo
            .refs()
            .ok()?
            .into_iter()
            .map(|r| (r.name, r.target))
            .collect();
        refs.sort();
        // FNV-1a: stable across Rust releases, unlike `DefaultHasher`.
        let defaults = serde_json::to_string(&self.settings().defaults).unwrap_or_default();
        let h = gitterrace_git::fnv1a64(
            refs.iter()
                .flat_map(|(name, target)| name.bytes().chain([0]).chain(target.bytes()).chain([0]))
                .chain(defaults.bytes())
                .chain([0]),
        );
        let version = gitterrace_index::FORMAT_VERSION;
        let generation = self.index.generation();
        Some(format!("\"{head}-{h:016x}-{version}-{generation}\""))
    }

    /// The source over the history reachable from `rev`, readable only through
    /// [`Checked::run`]: an answer built while the cache failed a read is refused.
    /// Shared between calls while the tip and the cache are unchanged. A source that met a
    /// storage error is dropped, so the next call opens the cache again.
    /// The checked source of `rev`, for [`ApiState::read`] and the cache tests: an answer
    /// reads through `read`, which turns a storage error into the rebuild.
    pub(crate) fn records(&self, rev: &str) -> Result<Checked<dyn InsightsSource>, ApiError> {
        let tip = self.repo.resolve(rev)?;
        let key = (tip.clone(), self.index.generation());
        if let Ok(mut g) = self.cache.lock() {
            if g.as_ref().is_some_and(|(_, v)| v.failure().is_some()) {
                g.take();
            }
            if let Some((k, v)) = g.as_ref()
                && *k == key
            {
                return Ok(v.share());
            }
        }
        // Another rev reads the same segments through a reach view. Without a segment set yet
        // HEAD's is built first: an answer never comes from anything but the set.
        let recs = if let Some(src) = self.view(rev, &tip)? {
            src
        } else {
            self.refresh_index()?;
            self.view(rev, &tip)?.ok_or_else(|| {
                ApiError::Index(std::io::Error::other("no segment set after a build"))
            })?
        };
        if let Ok(mut g) = self.cache.lock() {
            *g = Some((key, recs.share()));
        }
        Ok(recs)
    }

    /// The set when it holds `rev` exactly, else the reach view over it, `None` without a set.
    fn view(&self, rev: &str, tip: &str) -> Result<Option<Checked<dyn InsightsSource>>, ApiError> {
        self.open_view(rev, tip).map_err(|e| match e {
            ApiError::Index(io) => gitterrace_index::feed::SourceError::of(&io)
                .map_or(ApiError::Index(io), ApiError::Unreadable),
            other => other,
        })
    }

    fn open_view(
        &self,
        rev: &str,
        tip: &str,
    ) -> Result<Option<Checked<dyn InsightsSource>>, ApiError> {
        match self.index.source_for(&*self.repo, rev, tip)? {
            Some(Selected::Ready(src)) => Ok(Some(src)),
            Some(Selected::NeedsReindex { read }) => Err(ApiError::NeedsReindex {
                read,
                cap: self.index.fresh_cap(),
            }),
            None => Ok(None),
        }
    }

    /// Drops the shared source, the head memo and the full-id set, so an idle server keeps none.
    pub fn release_records(&self) {
        if let Ok(mut g) = self.cache.lock() {
            g.take();
        }
        if let Ok(mut g) = self.head.lock() {
            *g = HeadMemo::default();
        }
        self.repo.clear_visible();
    }

    /// Whether the head memo holds anything (a test hook for [`ApiState::release_records`]).
    /// Destructured, so a new field fails to compile here until the release test sees it.
    #[cfg(test)]
    pub(crate) fn head_memo_held(&self) -> bool {
        self.head.lock().is_ok_and(|g| {
            let HeadMemo { ignores, lines } = &*g;
            ignores.is_some() || lines.is_some()
        })
    }

    fn rev(rev: Option<&str>) -> Result<&str, ApiError> {
        checked(rev.filter(|r| !r.is_empty()).unwrap_or(DEFAULT_REV))
    }

    /// `(permalink, link)`: `permalink` is `/go/<repo-id>/<sha>[/<path>][#Lx-Ly]`, the web's
    /// permalink in path form (the MCP has no server address), and `link` the same in
    /// `links.style`, so `gitterrace://` by default. Both `None` for a repository without
    /// commits.
    fn links(
        &self,
        sha: &str,
        path: Option<&str>,
        lines: Option<(usize, usize)>,
    ) -> (Option<String>, Option<String>) {
        let Some(id) = self.repo.id() else {
            return (None, None);
        };
        let lines = lines.map(|(a, b)| Lines {
            start: u32::try_from(a).unwrap_or(u32::MAX),
            end: u32::try_from(b).unwrap_or(u32::MAX),
        });
        let name = self.repo.name();
        let go = gitterrace_actions::permalink("", Some(&id), &name, sha, path, lines);
        let link = styled(
            self.settings().style,
            "",
            Some(&id),
            &name,
            sha,
            path,
            lines,
        );
        (Some(go), Some(link))
    }

    pub fn repo_info(&self) -> Result<RepoResponse, ApiError> {
        let head = self.repo.resolve(DEFAULT_REV).ok();
        let (totals, every) = self.read(DEFAULT_REV, |records| {
            let ins = self.insights(DEFAULT_REV, &**records, self.settings().defaults);
            (
                ins.period_summary(gitterrace_index::Period::All).current,
                ins.every_totals().commits.value(),
            )
        })?;
        Ok(RepoResponse {
            head,
            head_branch: self.repo.head_branch(),
            index_version: self.etag(),
            name: self.repo.name(),
            totals: RepoTotals {
                additions: totals.additions,
                authors: totals.authors,
                commits: totals.commits,
                deletions: totals.deletions,
                every_commits: every,
                files: totals.files,
            },
        })
    }

    pub fn refs(&self) -> Result<RefsResponse, ApiError> {
        let refs = self.repo.refs()?.iter().map(wire::ref_item).collect();
        Ok(RefsResponse { refs })
    }

    pub fn log(
        &self,
        rev: Option<&str>,
        path: Option<&str>,
        page: usize,
        per_page: usize,
    ) -> Result<LogResponse, ApiError> {
        let rev = Self::rev(rev)?;
        let caps = self.settings().caps;
        let per_page = per_page.clamp(1, caps.log_page_commits);
        let page = page.max(1);
        let skip = (page - 1).saturating_mul(per_page);
        if skip > caps.log_skip_commits {
            return Err(ApiError::BadRequest(format!(
                "page too deep: at most {} commits can be skipped (api.log_skip_commits)",
                caps.log_skip_commits
            )));
        }
        // One extra row tells us whether another page exists.
        let rows = self.repo.log(rev, path, skip, per_page + 1)?;
        let has_more = rows.len() > per_page;
        let commits = rows
            .iter()
            .take(per_page)
            .map(|c| {
                let mut v = wire::log_commit(c);
                (v.permalink, v.link) = self.links(&c.id, None, None);
                v
            })
            .collect();
        Ok(LogResponse {
            commits,
            has_more,
            page,
            path: path.map(str::to_owned),
            per_page,
            rev: rev.to_owned(),
        })
    }

    pub fn commit(
        &self,
        id: &str,
        diff: bool,
        max_lines: Option<usize>,
    ) -> Result<CommitResponse, ApiError> {
        let id = self.repo.resolve(checked(id)?)?;
        let detail = self.repo.commit(&id)?;
        let most = self.settings().caps.diff_lines;
        let cap = max_lines.map_or(most, |m| m.min(most));
        let (mut files, truncated) = wire::files(&detail, diff, cap);
        for f in &mut files {
            (f.permalink, f.link) = self.links(&id, Some(&f.path), None);
        }
        let s = &detail.summary;
        let (permalink, link) = self.links(&id, None, None);
        Ok(CommitResponse {
            author: s.author.clone(),
            body: detail.body,
            committer: s.committer.clone(),
            files,
            id: s.id.clone(),
            link,
            parents: s.parents.clone(),
            permalink,
            subject: s.subject.clone(),
            truncated,
        })
    }

    pub fn tree(&self, rev: Option<&str>, path: Option<&str>) -> Result<TreeResponse, ApiError> {
        let rev = Self::rev(rev)?;
        let id = self.repo.resolve(rev)?;
        let path = path.unwrap_or("");
        let entries = self
            .repo
            .tree(&id, path)?
            .iter()
            .map(wire::tree_item)
            .collect();
        Ok(TreeResponse {
            commit: id,
            entries,
            path: path.to_owned(),
            rev: rev.to_owned(),
        })
    }

    /// Text content of a file. Binary files report `binary: true` and no text.
    /// `start_line` is 1-based; at most `limit` lines are returned.
    pub fn blob(
        &self,
        rev: Option<&str>,
        path: &str,
        start_line: Option<usize>,
        limit: Option<usize>,
    ) -> Result<BlobResponse, ApiError> {
        let rev = Self::rev(rev)?;
        let id = self.repo.resolve(rev)?;
        let blob = self.repo.blob(&id, path)?;
        let caps = self.settings().caps;
        let most_bytes = caps.blob_kb.saturating_mul(1024);
        let (text, total_lines, truncated) = blob.text.as_ref().map_or((None, None, false), |t| {
            let all: Vec<&str> = t.lines().collect();
            let start = start_line.unwrap_or(1).max(1) - 1;
            let limit = limit.unwrap_or(caps.blob_lines).min(caps.blob_lines);
            let end = all.len().min(start.saturating_add(limit));
            let slice = all.get(start..end).unwrap_or(&[]);
            let mut text = slice.join("\n");
            let over = text.len() > most_bytes;
            if over {
                let mut cut = most_bytes;
                while !text.is_char_boundary(cut) {
                    cut -= 1;
                }
                text.truncate(cut);
            }
            (
                Some(text),
                Some(all.len()),
                over || end < all.len() || start > 0,
            )
        });
        let shown = match (&text, total_lines) {
            (Some(t), Some(_)) if !t.is_empty() => {
                let first = start_line.unwrap_or(1).max(1);
                Some((first, first + t.lines().count() - 1))
            }
            _ => None,
        };
        let (permalink, link) = self.links(&id, Some(&blob.path), shown);
        Ok(BlobResponse {
            binary: blob.text.is_none(),
            commit: id,
            link,
            path: blob.path,
            permalink,
            rev: rev.to_owned(),
            size: blob.size,
            text,
            total_lines,
            truncated,
        })
    }

    pub fn blame(&self, rev: Option<&str>, path: &str) -> Result<BlameResponse, ApiError> {
        let rev = Self::rev(rev)?;
        let id = self.repo.resolve(rev)?;
        let all = self.repo.blame(&id, path)?;
        let most = self.settings().caps.blame_lines;
        let lines = all.iter().take(most).map(wire::blame_item).collect();
        let (permalink, link) = self.links(&id, Some(path), None);
        Ok(BlameResponse {
            commit: id,
            lines,
            link,
            path: path.to_owned(),
            permalink,
            rev: rev.to_owned(),
            truncated: all.len() > most,
        })
    }

    /// Statistics over `src` under `opts`, with the ignore rules of `rev` when asked: what
    /// the web pages build for the same options.
    fn insights<'a>(&self, rev: &str, src: &'a dyn InsightsSource, opts: Options) -> Insights<'a> {
        let ignore = opts.respect_ignore.then(|| self.ignore_set(rev)).flatten();
        Insights::from_source(src, opts, ignore)
    }

    /// The ignore rules committed at `rev`. The files are found once per commit.
    fn ignore_set(&self, rev: &str) -> Option<gitterrace_git::IgnoreSet> {
        let id = self.repo.resolve(rev).ok()?;
        let mut memo = self
            .head
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((k, files)) = &memo.ignores
            && *k == id
        {
            return Some(gitterrace_git::IgnoreSet::from_files(files.iter().cloned()));
        }
        let files = Arc::new(gitterrace_git::IgnoreSet::files_at(&self.repo, &id).ok()?);
        memo.ignores = Some((id, Arc::clone(&files)));
        Some(gitterrace_git::IgnoreSet::from_files(files.iter().cloned()))
    }

    /// Line count of every file in the tree of `tip`, built once per commit.
    fn head_lines(&self, tip: &str) -> Arc<Vec<gitterrace_index::HeadFile>> {
        let mut memo = self
            .head
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((k, files)) = &memo.lines
            && k == tip
        {
            return Arc::clone(files);
        }
        let mut cache = gitterrace_index::LineCache::load(self.index.dir());
        let files = gitterrace_index::head_files(&self.repo, tip, &mut cache).unwrap_or_default();
        let files = Arc::new(files);
        memo.lines = Some((tip.to_owned(), Arc::clone(&files)));
        files
    }

    /// One named statistic over the cached records reachable from `rev`, after the selection
    /// and under the options of `params`.
    pub fn insight(
        &self,
        name: &str,
        rev: Option<&str>,
        params: &insights::Params,
    ) -> Result<insights::InsightResponse, ApiError> {
        let rev = Self::rev(rev)?;
        let tip = self.repo.resolve(rev)?;
        let scope = insights::scope(params, self.settings().defaults)?;
        let mut commits_indexed = 0;
        let data = self.read(rev, |all| {
            commits_indexed = all.len();
            let ins = self.insights(&tip, &**all, scope.options().clone());
            let counted = insights::counted(&ins, &scope, &**all);
            insights::compute(name, &counted, params, || self.head_lines(&tip))
        })??;
        let data = insights::with_links(name, data, |p| self.links(&tip, Some(p), None));
        Ok(insights::InsightResponse {
            commits_indexed,
            query: scope.query_text(),
            data,
            insight: name.to_owned(),
            rev: rev.to_owned(),
        })
    }

    /// People with their commits, plus a commit-share bus factor: the fewest people who made
    /// half the commits. Counted as `/insights/contributors` counts them under the same
    /// query: the period, the selection and the options of `params`.
    pub fn contributors(
        &self,
        rev: Option<&str>,
        params: &insights::Params,
    ) -> Result<ContributorsResponse, ApiError> {
        let rev = Self::rev(rev)?;
        let tip = self.repo.resolve(rev)?;
        let scope = insights::scope(params, self.settings().defaults)?;
        let report = self.read(rev, |all| {
            let ins = self.insights(&tip, &**all, scope.options().clone());
            insights::counted(&ins, &scope, &**all).contributors(gitterrace_index::Metric::Commits)
        })?;
        let people = report.contributors;
        let bus_factor = gitterrace_index::bus_factor(people.iter().map(|c| c.commits as f64));
        Ok(ContributorsResponse {
            authors: people.into_iter().map(wire::contributor).collect(),
            bus_factor,
            rev: rev.to_owned(),
        })
    }
}

/// `rev` unless it uses syntax in [`REJECTED_REV`].
fn checked(rev: &str) -> Result<&str, ApiError> {
    if REJECTED_REV.iter().any(|p| rev.contains(p)) {
        return Err(ApiError::BadRequest(format!(
            "unsupported revision syntax in {rev:?}: use a branch, tag, sha or HEAD~n"
        )));
    }
    Ok(rev)
}
