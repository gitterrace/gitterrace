//! The languages page as typed data: the area chart, the range card and the table of languages
//! now. `languages_body.html` writes the page, `chart_loc.svg` the chart, both escaping every
//! value; the plot data for `loc.js` is JSON with `<` written as `\u003c`.

use askama::Template;
use askama::filters::HtmlSafe;

use crate::charts::view::{P, Tick};
use crate::empty::Empty;
use crate::seg::Seg;

/// One stacked band of the chart.
#[derive(Debug)]
pub(super) struct Band {
    pub(super) class: String,
    pub(super) d: String,
    pub(super) tip: String,
}

/// A date under the plot.
#[derive(Debug)]
pub(super) struct DateLabel {
    pub(super) x: P,
    pub(super) anchor: &'static str,
    pub(super) text: String,
}

#[derive(Debug, Template)]
#[template(path = "chart_loc.svg")]
pub(super) struct Area {
    pub(super) right: P,
    pub(super) ticks: Vec<Tick>,
    pub(super) bands: Vec<Band>,
    pub(super) dates: Vec<DateLabel>,
    /// The selected range: x and width, zero for none.
    pub(super) sel: (P, P),
    pub(super) none: bool,
    pub(super) ph: P,
    pub(super) xh_y2: P,
}

impl HtmlSafe for Area {}

/// A language of the range card.
#[derive(Debug)]
pub(super) struct RangeRow {
    pub(super) class: String,
    pub(super) name: String,
    pub(super) before: String,
    pub(super) after: String,
    pub(super) share: String,
    pub(super) up: bool,
    pub(super) change: String,
    pub(super) w: u64,
}

/// A commit of the range list.
#[derive(Debug)]
pub(super) struct RangeCommit {
    /// Finished HTML from `fmt::sha_link`.
    pub(super) sha: String,
    pub(super) subject: String,
    pub(super) day: String,
}

#[derive(Debug)]
pub(super) struct RangeCard {
    pub(super) title: &'static str,
    pub(super) from: String,
    pub(super) to: String,
    pub(super) presets: Seg,
    pub(super) count: String,
    pub(super) count_noun: &'static str,
    pub(super) added: String,
    pub(super) removed: String,
    pub(super) net_up: bool,
    pub(super) net: String,
    pub(super) rows: Vec<RangeRow>,
    pub(super) list_title: String,
    pub(super) commits: Vec<RangeCommit>,
    /// Set when the list is cut: how many it shows.
    pub(super) cut: Option<usize>,
}

/// A part of the share bar.
#[derive(Debug)]
pub(super) struct SharePart {
    pub(super) class: String,
    pub(super) w: u32,
    pub(super) tip: String,
}

/// A row of the languages-now table.
#[derive(Debug)]
pub(super) struct NowRow {
    pub(super) class: String,
    pub(super) name: String,
    pub(super) lines: String,
    pub(super) share: String,
    pub(super) spark_class: String,
    pub(super) points: String,
    pub(super) up: bool,
    pub(super) change: String,
}

#[derive(Debug)]
pub(super) struct Now {
    pub(super) bar: Vec<SharePart>,
    pub(super) window: String,
    pub(super) rows: Vec<NowRow>,
}

/// The chart, its data and the sections under it; `None` when the chain is empty.
#[derive(Debug)]
pub(super) struct Shown {
    pub(super) lead: String,
    pub(super) axis: Seg,
    pub(super) scale: Seg,
    pub(super) total: String,
    /// Legend entries: class and language.
    pub(super) legend: Vec<(String, String)>,
    pub(super) area: Area,
    /// The plot data, JSON safe inside a script element.
    pub(super) json: String,
    pub(super) range: RangeCard,
    pub(super) now: Now,
}

#[derive(Debug, Template)]
#[template(path = "languages_body.html")]
pub(super) struct Body {
    pub(super) empty: Option<Empty>,
    pub(super) shown: Option<Shown>,
}

impl HtmlSafe for Body {}
