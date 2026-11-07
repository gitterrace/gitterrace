//! The chart models and their markup. Each chart is typed data rendered by its `chart_*.svg`
//! template; the templates escape every value with [`Esc`], the same `& < > "` set as
//! [`crate::fmt::esc`], so a label or link can never close an attribute or open a tag.

use std::fmt;

use askama::Template;
use askama::filters::{Escaper, HtmlSafe};

/// The escaper of the `.svg` templates (`askama.toml`): `& < > "`, as [`crate::fmt::esc`].
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Esc;

impl Escaper for Esc {
    fn write_escaped_str<W: fmt::Write>(&self, mut dest: W, s: &str) -> fmt::Result {
        let mut last = 0;
        for (i, c) in s.char_indices() {
            let rep = match c {
                '&' => "&amp;",
                '<' => "&lt;",
                '>' => "&gt;",
                '"' => "&quot;",
                _ => continue,
            };
            dest.write_str(&s[last..i])?;
            dest.write_str(rep)?;
            last = i + 1;
        }
        dest.write_str(&s[last..])
    }
}

/// A coordinate as the charts print it: one decimal.
#[derive(Debug, Clone, Copy)]
pub(crate) struct P(pub(crate) f64);

impl fmt::Display for P {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.1}", self.0)
    }
}

/// A value gridline with its label on the left.
#[derive(Debug)]
pub(crate) struct Tick {
    pub(crate) y: P,
    /// The label baseline, just below the line.
    pub(crate) ty: P,
    pub(crate) label: String,
}

/// A label under a column.
#[derive(Debug)]
pub(crate) struct XLabel {
    pub(crate) x: P,
    pub(crate) anchor: &'static str,
    pub(crate) text: String,
}

/// One row of a chart's data table: what the row is, then its values as the chart shows them.
#[derive(Debug)]
pub(crate) struct Row {
    pub(crate) label: String,
    pub(crate) cells: Vec<String>,
}

/// The values of a chart as a table: column heads (the first names the row labels), one row
/// per mark or column, and where the chart's tips name a total, that total.
#[derive(Debug)]
pub(crate) struct Table {
    pub(crate) head: Vec<String>,
    pub(crate) rows: Vec<Row>,
    pub(crate) total: Option<Row>,
}

/// The text form of a chart: its name, a one-sentence summary it is described by, and its
/// values as a table behind a "Show as table" disclosure. `id` ties the three together.
#[derive(Debug)]
pub(crate) struct Fig {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) summary: String,
    pub(crate) table: Table,
}

/// The parts every full-width chart shares: its text form, whether its marks are links, the column
/// geometry, the gridlines and the labels under the columns.
#[derive(Debug)]
pub(crate) struct Frame {
    pub(crate) fig: Fig,
    pub(crate) linked: bool,
    pub(crate) sl: P,
    pub(crate) bw: P,
    pub(crate) plot: P,
    pub(crate) ticks: Vec<Tick>,
    pub(crate) labels: Vec<XLabel>,
}

/// One column of [`Bars`].
#[derive(Debug)]
pub(crate) struct BarCol {
    /// Link and its accessible name, for a chart whose bars narrow the page. The link carries
    /// the tip, so keyboard focus shows it.
    pub(crate) link: Option<(String, String)>,
    pub(crate) tip: String,
    pub(crate) hx: P,
    pub(crate) x: P,
    pub(crate) y: P,
    pub(crate) h: P,
}

#[derive(Debug, Template)]
#[template(path = "chart_bars.svg")]
pub(crate) struct Bars {
    pub(crate) f: Frame,
    pub(crate) cols: Vec<BarCol>,
}

/// One part of a stacked column.
#[derive(Debug)]
pub(crate) struct Seg {
    pub(crate) class: String,
    pub(crate) y: P,
    pub(crate) h: P,
}

#[derive(Debug)]
pub(crate) struct StackCol {
    pub(crate) tip: String,
    pub(crate) hx: P,
    pub(crate) x: P,
    pub(crate) segs: Vec<Seg>,
}

#[derive(Debug, Template)]
#[template(path = "chart_stacked.svg")]
pub(crate) struct Stacked {
    pub(crate) f: Frame,
    pub(crate) cols: Vec<StackCol>,
    /// Legend entries: class and series name.
    pub(crate) legend: Vec<(String, String)>,
    pub(crate) other: bool,
}

#[derive(Debug)]
pub(crate) struct DivCol {
    pub(crate) tip: String,
    pub(crate) hx: P,
    pub(crate) x: P,
    pub(crate) up_y: P,
    pub(crate) ha: P,
    pub(crate) hd: P,
}

#[derive(Debug, Template)]
#[template(path = "chart_diverging.svg")]
pub(crate) struct Diverging {
    pub(crate) f: Frame,
    pub(crate) mid: P,
    pub(crate) cols: Vec<DivCol>,
}

/// A reading of [`PercentLine`]: its hover band and the dot on the line.
#[derive(Debug)]
pub(crate) struct LinePoint {
    pub(crate) tip: String,
    pub(crate) x: P,
    pub(crate) cx: P,
    pub(crate) cy: P,
}

#[derive(Debug, Template)]
#[template(path = "chart_line.svg")]
pub(crate) struct PercentLine {
    pub(crate) f: Frame,
    pub(crate) points: Vec<LinePoint>,
    /// The path data of the line.
    pub(crate) d: String,
}

#[derive(Debug, Template)]
#[template(path = "chart_spark.svg")]
pub(crate) struct Spark {
    pub(crate) class: String,
    pub(crate) points: String,
}

#[derive(Debug)]
pub(crate) struct ScatterDot {
    pub(crate) href: String,
    pub(crate) title: String,
    pub(crate) cx: P,
    pub(crate) cy: P,
    pub(crate) r: P,
}

#[derive(Debug, Template)]
#[template(path = "chart_scatter.svg")]
pub(crate) struct Scatter {
    pub(crate) f: Frame,
    /// Labels along the bottom: x and value.
    pub(crate) xticks: Vec<(P, String)>,
    pub(crate) dots: Vec<ScatterDot>,
    pub(crate) x_title: String,
}

#[derive(Debug)]
pub(crate) struct HeatCell {
    pub(crate) level: u32,
    pub(crate) x: usize,
    pub(crate) tip: String,
}

#[derive(Debug)]
pub(crate) struct HeatRow {
    pub(crate) y: usize,
    pub(crate) ly: usize,
    pub(crate) day: &'static str,
    pub(crate) cells: Vec<HeatCell>,
}

#[derive(Debug, Template)]
#[template(path = "chart_heat.svg")]
pub(crate) struct Heatmap {
    pub(crate) fig: Fig,
    pub(crate) w: usize,
    pub(crate) h: usize,
    pub(crate) side: usize,
    /// Hour labels: x and two-digit hour.
    pub(crate) hours: Vec<(usize, String)>,
    pub(crate) rows: Vec<HeatRow>,
}

// The derived `Display` renders through an escaping template, so a page template may print a
// chart as it is.
impl HtmlSafe for Bars {}
impl HtmlSafe for Stacked {}
impl HtmlSafe for Diverging {}
impl HtmlSafe for PercentLine {}
impl HtmlSafe for Spark {}
impl HtmlSafe for Scatter {}
impl HtmlSafe for Heatmap {}
