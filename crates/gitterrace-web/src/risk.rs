//! Knowledge risk on the ownership pages: the section under the Ownership table and the page of
//! one person, `/stats/ownership/{author}`.
//!
//! Every number is `Insights::knowledge_risk_in`, the one the API serves as
//! `/insights/knowledge_risk`. A directory is read one level at a time: the section reads one
//! key of its own, `drill`, which is part of the page memo key (`App::canonical_query`).

use askama::Template;
use axum::extract::{Path, Query, RawQuery, State};
use axum::response::Html;
use gitterrace_index::{
    Counted, HeadFile, Insights, KnowledgeRisk, Period, RISK_LIMIT, RiskPerson, RiskQuery,
};
use serde::Deserialize;

use crate::{
    app::{App, blocking},
    empty::Empty,
    error::{Error, Result},
    filter::FilterBar,
    fmt,
    insights::{Defaults, InsightsQuery, Kpi, cached_keyed, counted, head_lines, insights},
    pagehead::PageHead,
    routes::Shared,
    views::Chrome,
    when::When,
};

/// The page of the ownership section the people links lead to.
const BASE: &str = "/stats/ownership";

/// The one key of the section besides the shared filter.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct Drill {
    /// The directory whose children are listed, the root when empty.
    pub(crate) drill: Option<String>,
}

impl Drill {
    pub(crate) fn path(&self) -> Option<&str> {
        self.drill.as_deref().map(|p| p.trim_matches('/')).filter(|p| !p.is_empty())
    }
}

pub(crate) struct DirRow {
    pub(crate) label: Vec<String>,
    /// The link that lists this directory's children, empty for the files directly in the path.
    pub(crate) drill: String,
    pub(crate) lines: String,
    pub(crate) files: String,
    pub(crate) owner: String,
    pub(crate) owner_href: String,
    pub(crate) share: String,
    pub(crate) bus: usize,
    pub(crate) active: bool,
    pub(crate) at_risk: bool,
    pub(crate) last: When,
}

pub(crate) struct OrphanRow {
    pub(crate) path: Vec<String>,
    pub(crate) href: String,
    pub(crate) lines: String,
    pub(crate) last: When,
}

struct FileRow {
    path: Vec<String>,
    href: String,
    lines: String,
    share: String,
}

/// What both pages show of the knowledge risk.
pub(crate) struct RiskView {
    pub(crate) summary: String,
    pub(crate) here: String,
    pub(crate) up: Option<String>,
    pub(crate) dirs: Vec<DirRow>,
    pub(crate) orphans: Vec<OrphanRow>,
    pub(crate) orphan_note: String,
    pub(crate) none: Option<Empty>,
    /// Whose directories these are, on a person's page.
    pub(crate) of: Option<String>,
}

/// Where the people links and the drill links point: the page, its filters and the key.
struct Links<'a> {
    q: &'a InsightsQuery,
    d: &'a Defaults,
    page: &'a str,
}

impl Links<'_> {
    fn drill(&self, path: Option<&str>) -> String {
        let set: Vec<(&str, &str)> = path.map(|p| ("drill", p)).into_iter().collect();
        format!("{}#risk", self.q.link_with(self.d, self.page, &set))
    }

    fn person(&self, name: &str) -> String {
        let page = format!("{BASE}/{}", fmt::enc_seg(name));
        self.q.link_with(self.d, &page, &[])
    }
}

fn parent(path: &str) -> Option<&str> {
    path.rsplit_once('/').map(|(p, _)| p)
}

fn label(path: &str, here: Option<&str>) -> String {
    match (path.is_empty(), here) {
        (true, _) => "(root files)".to_owned(),
        (false, Some(h)) if h == path => format!("(files in {path})"),
        _ => path.to_owned(),
    }
}

fn view(rep: &KnowledgeRisk, links: &Links<'_>, here: Option<&str>, of: Option<&str>) -> RiskView {
    let now = fmt::now();
    let dirs: Vec<DirRow> = rep
        .dirs
        .iter()
        .map(|r| DirRow {
            label: fmt::path_parts(&label(&r.path, here)),
            drill: if r.path.is_empty() || here == Some(&r.path) {
                String::new()
            } else {
                links.drill(Some(&r.path))
            },
            lines: fmt::thousands(r.lines),
            files: fmt::thousands(r.files),
            owner_href: links.person(&r.top_owner),
            owner: r.top_owner.clone(),
            share: format!("{:.0}%", r.top_share * 100.0),
            bus: r.bus_factor,
            active: r.owner_active,
            at_risk: r.at_risk,
            last: fmt::ago(r.last_active, now),
        })
        .collect();
    let orphans = rep
        .orphans
        .iter()
        .map(|o| OrphanRow {
            path: fmt::path_parts(&o.path),
            href: fmt::file_link(&o.path),
            lines: fmt::thousands(o.lines),
            last: fmt::ago(o.last_change, now),
        })
        .collect();
    let summary = if rep.at_risk_dirs == 0 {
        "No directory here rests on one person who has gone quiet.".to_owned()
    } else {
        format!(
            "{} ({}) rest on one person who has not committed in a year.",
            fmt::count(rep.at_risk_dirs, "directory", "directories"),
            fmt::count(rep.at_risk_lines, "line", "lines")
        )
    };
    let orphan_note = if rep.orphan_files == 0 {
        String::new()
    } else {
        format!(
            "{} ({}) have no active author.",
            fmt::count(rep.orphan_files, "file", "files"),
            fmt::count(rep.orphan_lines, "line", "lines")
        )
    };
    RiskView {
        summary,
        here: here.map_or_else(|| "the repository".to_owned(), str::to_owned),
        up: here.map(|h| links.drill(parent(h))),
        none: dirs.is_empty().then(|| Empty::new("No directory here has counted changes.")),
        dirs,
        orphans,
        orphan_note,
        of: of.map(str::to_owned),
    }
}

/// The risk of the whole tree under `drill`, for the Ownership page.
pub(crate) fn section(
    ins: &Insights<'_>,
    c: &Counted<'_>,
    files: &[HeadFile],
    q: &InsightsQuery,
    d: &Defaults,
    drill: &Drill,
) -> RiskView {
    let query = RiskQuery { path: drill.path(), author: None, limit: RISK_LIMIT };
    let rep = ins.knowledge_risk_in(files, c, &query).unwrap_or_else(empty);
    view(&rep, &Links { q, d, page: BASE }, drill.path(), None)
}

const fn empty() -> KnowledgeRisk {
    KnowledgeRisk {
        dirs: Vec::new(),
        orphans: Vec::new(),
        orphan_files: 0,
        orphan_lines: 0,
        person: None,
        at_risk_lines: 0,
        at_risk_dirs: 0,
    }
}

#[derive(Template)]
#[template(path = "ownership_person.html")]
struct PersonPage {
    c: Chrome,
    head: PageHead,
    fb: FilterBar,
    kpis: Vec<Kpi>,
    name: String,
    back: String,
    files: Vec<FileRow>,
    /// The empty state when the person is the top owner of no file.
    none: Option<Empty>,
    risk: RiskView,
}

/// "3 months ago", or a dash for someone with no commit in the history.
fn last_active(time: i64, now: i64) -> String {
    if time == i64::MIN { "-".to_owned() } else { fmt::ago(time, now).text }
}

fn person_lead(p: &RiskPerson) -> String {
    if p.files == 0 {
        return format!("{} is the top owner of no file.", p.name);
    }
    format!(
        "{} is the top owner of {}, {:.0}% of the code.",
        p.name,
        fmt::count(p.files, "file", "files"),
        p.share_of_repo * 100.0
    )
}

/// `/stats/ownership/{author}`: what one person owns, by lines changed.
pub(crate) async fn person(
    State(app): Shared,
    Path(author): Path<String>,
    RawQuery(raw): RawQuery,
    Query(q): Query<InsightsQuery>,
    Query(drill): Query<Drill>,
) -> Result<Html<String>> {
    let name = author.clone();
    if !known(&app, name).await? {
        return Err(Error::Missing("Nobody of that name has committed here.".to_owned()));
    }
    let key = format!("{author}\u{0}{}", App::canonical_query(raw.as_deref()));
    cached_keyed(&app, "ow", key, move |a, r, c| {
        let f = &q.filter;
        let d = a.defaults();
        let (period, scope) = q.scoped(&d, Period::All);
        let ins = insights(a, r, scope.options().clone());
        let files = head_lines(a);
        let set = counted(&ins, &scope, r);
        let query = RiskQuery { path: drill.path(), author: Some(&author), limit: RISK_LIMIT };
        let rep = ins.knowledge_risk_in(&files, &set, &query).unwrap_or_else(empty);
        let page = format!("{BASE}/{}", fmt::enc_seg(&author));
        let links = Links { q: &q, d: &d, page: &page };
        let now = fmt::now();
        let p = rep.person.clone().unwrap_or_else(|| RiskPerson {
            name: author.clone(),
            files: 0,
            lines: 0,
            share_of_repo: 0.0,
            last_active: i64::MIN,
            top_files: Vec::new(),
        });
        let risk = view(&rep, &links, drill.path(), Some(&author));
        let fb = f.bar(&page, &**r, period);
        PersonPage {
            c,
            head: fb.head("ow", &person_lead(&p), &q, &d, &page),
            fb,
            kpis: vec![
                Kpi("Files owned", fmt::thousands(p.files)),
                Kpi("Lines", fmt::thousands(p.lines)),
                Kpi("Share of the code", format!("{:.0}%", p.share_of_repo * 100.0)),
                Kpi("Last active", last_active(p.last_active, now)),
            ],
            back: q.link_with(&d, BASE, &[]),
            none: p.top_files.is_empty().then(|| {
                Empty::new(format!("No file has {author} as its top owner in these commits."))
            }),
            files: p
                .top_files
                .iter()
                .map(|t| FileRow {
                    path: fmt::path_parts(&t.path),
                    href: fmt::file_link(&t.path),
                    lines: fmt::thousands(t.lines),
                    share: format!("{:.0}%", t.share * 100.0),
                })
                .collect(),
            name: author,
            risk,
        }
    })
    .await
}

/// Whether a person of this name is in the history. A history still indexing is let through:
/// the page then says so.
async fn known(app: &std::sync::Arc<App>, name: String) -> Result<bool> {
    blocking(app, move |a| {
        let head = a.repo.resolve(&fmt::selected())?;
        let Some(src) = a.source(&head)? else {
            return Ok(true);
        };
        Ok(src.run(|s| insights(a, s, a.defaults().into_options()).knows(&name)).unwrap_or(true))
    })
    .await
}
