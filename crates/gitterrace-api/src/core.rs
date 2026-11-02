//! Transport-neutral reads. Both the HTTP router and the MCP tools call these,
//! so the two surfaces cannot drift apart. Everything here is blocking.

use std::path::Path;
use std::sync::{Arc, Mutex};

use gitterrace_actions::{Lines, LinkStyle, styled};
use gitterrace_git::Repo;
use gitterrace_index::{CommitRecord, Index, Insights, Options};
use serde_json::{Value, json};

use crate::error::ApiError;
use crate::{insights, json};

const DEFAULT_REV: &str = "HEAD";
/// Largest `per_page` the log accepts.
pub const MAX_PAGE: usize = 500;
/// Most lines one blob answer carries.
pub const MAX_BLOB_LINES: usize = 5000;
/// Lines alone do not bound an answer: one minified line can be megabytes.
pub const MAX_BLOB_BYTES: usize = 256 * 1024;
/// Most lines one blame answer carries.
pub const MAX_BLAME_LINES: usize = 5000;
/// Most diff lines one commit answer carries, whatever `max_lines` asks: a vendored-code
/// commit can hold millions, each a JSON object.
pub const MAX_DIFF_LINES: usize = 20_000;
/// Deepest the log pages: a path filter walks every skipped commit with tree lookups.
const MAX_LOG_SKIP: usize = 50_000;
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
        include_generated: i.generated,
        include_bots: i.bots,
        ..Options::default()
    }
}

/// Shared handle on one repository and its insights cache.
#[derive(Clone)]
pub struct ApiState {
    pub(crate) repo: Arc<Repo>,
    pub(crate) index: Arc<Index>,
    /// Parsed records of the last request, keyed by (resolved tip, index generation).
    cache: Arc<Mutex<Option<RecordsCache>>>,
    /// Form of the `link` field (`links.style`); `permalink` is always `/go`.
    style: LinkStyle,
    /// Counting options when a request names none: the `insights` configuration, the same
    /// defaults the web pages start from.
    defaults: Options,
}

type RecordsCache = ((String, u64), Arc<Vec<CommitRecord>>);

impl ApiState {
    pub fn new(repo: Arc<Repo>, index: Arc<Index>) -> Self {
        Self {
            repo,
            index,
            cache: Arc::default(),
            style: LinkStyle::default(),
            defaults: Options::default(),
        }
    }

    /// Counting options from the configuration, used when a request leaves one unset.
    #[must_use]
    pub fn with_defaults(mut self, defaults: Options) -> Self {
        self.defaults = defaults;
        self
    }

    /// Link style and counting defaults from the configuration: `links.style`, `insights.*`
    /// and this repository's `[repo.NAME] exclude`. The web's `/api/v1` and the MCP server both
    /// build their state through here, so the two answer with the same numbers.
    #[must_use]
    pub fn configured(self, cfg: &gitterrace_config::Config) -> Self {
        let defaults = options_from(cfg, &self.repo.name());
        self.with_link_style(LinkStyle::from_config(&cfg.links.style))
            .with_defaults(defaults)
    }

    /// The `link` field in `style` from now on.
    #[must_use]
    pub const fn with_link_style(mut self, style: LinkStyle) -> Self {
        self.style = style;
        self
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
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for (name, target) in &refs {
            for b in name.bytes().chain([0]).chain(target.bytes()).chain([0]) {
                h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
            }
        }
        let defaults = serde_json::to_string(&self.defaults).unwrap_or_default();
        for b in defaults.bytes().chain([0]) {
            h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
        }
        let version = gitterrace_index::FORMAT_VERSION;
        let generation = self.index.generation();
        Some(format!("\"{head}-{h:016x}-{version}-{generation}\""))
    }

    /// Records reachable from `rev`, shared between calls while the tip and the cache file are
    /// unchanged.
    pub(crate) fn records(&self, rev: &str) -> Result<Arc<Vec<CommitRecord>>, ApiError> {
        let tip = self.repo.resolve(rev)?;
        let key = (tip, self.index.generation());
        if let Ok(g) = self.cache.lock()
            && let Some((k, v)) = g.as_ref()
            && *k == key
        {
            return Ok(Arc::clone(v));
        }
        let recs = Arc::new(self.index.records(&*self.repo, rev)?);
        if let Ok(mut g) = self.cache.lock() {
            *g = Some((key, Arc::clone(&recs)));
        }
        Ok(recs)
    }

    fn rev(rev: Option<&str>) -> Result<&str, ApiError> {
        checked(rev.filter(|r| !r.is_empty()).unwrap_or(DEFAULT_REV))
    }

    /// Sets `permalink` (`/go/<repo-id>/<sha>[/<path>][#Lx-Ly]`, the web's permalink in path
    /// form, since the MCP has no server address) and `link` (the same in `links.style`, so
    /// `gitterrace://` by default) on `v`. Both null for a repository without commits.
    fn put_links(
        &self,
        v: &mut Value,
        sha: &str,
        path: Option<&str>,
        lines: Option<(usize, usize)>,
    ) {
        let Some(id) = self.repo.id() else {
            v["permalink"] = Value::Null;
            v["link"] = Value::Null;
            return;
        };
        let lines = lines.map(|(a, b)| Lines {
            start: u32::try_from(a).unwrap_or(u32::MAX),
            end: u32::try_from(b).unwrap_or(u32::MAX),
        });
        let name = self.repo.name();
        let go = gitterrace_actions::permalink("", Some(&id), &name, sha, path, lines);
        let link = styled(self.style, "", Some(&id), &name, sha, path, lines);
        v["permalink"] = json!(go);
        v["link"] = json!(link);
    }

    pub fn repo_info(&self) -> Result<Value, ApiError> {
        let head = self.repo.resolve(DEFAULT_REV).ok();
        let records = self.records(DEFAULT_REV)?;
        let totals = self
            .insights(DEFAULT_REV, &records, self.defaults.clone())
            .period_summary(gitterrace_index::Period::All)
            .current;
        Ok(json!({
            "name": self.repo.name(),
            "head": head,
            "head_branch": self.repo.head_branch(),
            "index_version": self.etag(),
            "totals": totals,
        }))
    }

    pub fn refs(&self) -> Result<Value, ApiError> {
        let refs: Vec<Value> = self.repo.refs()?.iter().map(json::ref_info).collect();
        Ok(json!({ "refs": refs }))
    }

    pub fn log(
        &self,
        rev: Option<&str>,
        path: Option<&str>,
        page: usize,
        per_page: usize,
    ) -> Result<Value, ApiError> {
        let rev = Self::rev(rev)?;
        let per_page = per_page.clamp(1, MAX_PAGE);
        let page = page.max(1);
        let skip = (page - 1).saturating_mul(per_page);
        if skip > MAX_LOG_SKIP {
            return Err(ApiError::BadRequest(format!(
                "page too deep: at most {MAX_LOG_SKIP} commits can be skipped"
            )));
        }
        // One extra row tells us whether another page exists.
        let rows = self.repo.log(rev, path, skip, per_page + 1)?;
        let has_more = rows.len() > per_page;
        let commits: Vec<Value> = rows
            .iter()
            .take(per_page)
            .map(|c| {
                let mut v = json::summary(c);
                self.put_links(&mut v, &c.id, None, None);
                v
            })
            .collect();
        Ok(json!({
            "rev": rev, "path": path, "page": page, "per_page": per_page,
            "has_more": has_more, "commits": commits,
        }))
    }

    pub fn commit(
        &self,
        id: &str,
        diff: bool,
        max_lines: Option<usize>,
    ) -> Result<Value, ApiError> {
        let id = self.repo.resolve(checked(id)?)?;
        let detail = self.repo.commit(&id)?;
        let cap = max_lines.map_or(MAX_DIFF_LINES, |m| m.min(MAX_DIFF_LINES));
        let mut v = json::detail(&detail, diff, Some(cap));
        self.put_links(&mut v, &id, None, None);
        if let Some(files) = v["files"].as_array_mut() {
            for f in files {
                if let Some(p) = f["path"].as_str().map(str::to_owned) {
                    self.put_links(f, &id, Some(&p), None);
                }
            }
        }
        Ok(v)
    }

    pub fn tree(&self, rev: Option<&str>, path: Option<&str>) -> Result<Value, ApiError> {
        let rev = Self::rev(rev)?;
        let id = self.repo.resolve(rev)?;
        let path = path.unwrap_or("");
        let entries: Vec<Value> = self
            .repo
            .tree(&id, path)?
            .iter()
            .map(json::tree_entry)
            .collect();
        Ok(json!({ "rev": rev, "commit": id, "path": path, "entries": entries }))
    }

    /// Text content of a file. Binary files report `binary: true` and no text.
    /// `start_line` is 1-based; at most `limit` lines are returned.
    pub fn blob(
        &self,
        rev: Option<&str>,
        path: &str,
        start_line: Option<usize>,
        limit: Option<usize>,
    ) -> Result<Value, ApiError> {
        let rev = Self::rev(rev)?;
        let id = self.repo.resolve(rev)?;
        let blob = self.repo.blob(&id, path)?;
        let (text, total_lines, truncated) = blob.text.as_ref().map_or((None, None, false), |t| {
            let all: Vec<&str> = t.lines().collect();
            let start = start_line.unwrap_or(1).max(1) - 1;
            let limit = limit.unwrap_or(MAX_BLOB_LINES).min(MAX_BLOB_LINES);
            let end = all.len().min(start.saturating_add(limit));
            let slice = all.get(start..end).unwrap_or(&[]);
            let mut text = slice.join("\n");
            let over = text.len() > MAX_BLOB_BYTES;
            if over {
                let mut cut = MAX_BLOB_BYTES;
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
        let mut v = json!({
            "rev": rev, "commit": id, "path": blob.path, "size": blob.size,
            "binary": blob.text.is_none(), "total_lines": total_lines,
            "truncated": truncated, "text": text,
        });
        self.put_links(&mut v, &id, Some(&blob.path), shown);
        Ok(v)
    }

    pub fn blame(&self, rev: Option<&str>, path: &str) -> Result<Value, ApiError> {
        let rev = Self::rev(rev)?;
        let id = self.repo.resolve(rev)?;
        let all = self.repo.blame(&id, path)?;
        let lines: Vec<Value> = all
            .iter()
            .take(MAX_BLAME_LINES)
            .map(json::blame_line)
            .collect();
        let mut v = json!({
            "rev": rev, "commit": id, "path": path,
            "truncated": all.len() > MAX_BLAME_LINES, "lines": lines,
        });
        self.put_links(&mut v, &id, Some(path), None);
        Ok(v)
    }

    /// Statistics over `records` under `opts`, with the ignore rules of `rev` when asked: what
    /// the web pages build for the same options.
    fn insights<'a>(&self, rev: &str, records: &'a [CommitRecord], opts: Options) -> Insights<'a> {
        let ignore = opts
            .respect_ignore
            .then(|| gitterrace_git::IgnoreSet::at(&self.repo, rev).ok())
            .flatten();
        Insights::new(records, opts, ignore)
    }

    /// One named statistic over the cached records reachable from `rev`, after the selection
    /// and under the options of `params`.
    pub fn insight(
        &self,
        name: &str,
        rev: Option<&str>,
        params: &insights::Params,
    ) -> Result<Value, ApiError> {
        let rev = Self::rev(rev)?;
        let tip = self.repo.resolve(rev)?;
        let all = self.records(rev)?;
        let records = params.select.apply(&all);
        let opts = params.options.apply(self.defaults.clone());
        let ins = self.insights(&tip, &records, opts);
        let mut data = insights::compute(name, &ins, params, || {
            let mut cache = gitterrace_index::LineCache::load(self.index.dir());
            gitterrace_index::head_files(&self.repo, &tip, &mut cache).unwrap_or_default()
        })?;
        let rows = match name {
            "file_churn" => data.as_array_mut(),
            "hotspots" => data["files"].as_array_mut(),
            _ => None,
        };
        for row in rows.into_iter().flatten() {
            if let Some(p) = row["path"].as_str().map(str::to_owned) {
                self.put_links(row, &tip, Some(&p), None);
            }
        }
        Ok(json!({ "rev": rev, "insight": name, "commits_indexed": all.len(), "data": data }))
    }

    /// People with their commits, plus a commit-share bus factor: the fewest people who made
    /// half the commits. Bots are left out unless the configuration counts them.
    pub fn contributors(&self, rev: Option<&str>) -> Result<Value, ApiError> {
        let rev = Self::rev(rev)?;
        let records = self.records(rev)?;
        let opts = self.defaults.clone();
        let bots = opts.include_bots;
        let report = self
            .insights(rev, &records, opts)
            .contributors(gitterrace_index::Period::All);
        let people: Vec<_> = report
            .contributors
            .into_iter()
            .filter(|c| bots || !c.bot)
            .collect();
        let bus_factor = gitterrace_index::bus_factor(people.iter().map(|c| c.commits as f64));
        Ok(json!({ "rev": rev, "bus_factor": bus_factor, "authors": people }))
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
