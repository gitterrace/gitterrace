//! The file rail beside a diff: one list row per changed file, linking to the file's `#fN`
//! anchor. At most `RAIL_ROWS` rows are written and a link to the first file left out follows,
//! so a diff of thousands of files stays inside its DOM budget. The page script marks the
//! current and the viewed rows.

use askama::Template;

use crate::views::FileStatRow;

/// The most rows the rail lists, as many as the review panel: the full list stays in the files
/// card. Not configurable.
const RAIL_ROWS: usize = 50;

struct RailRow {
    anchor: String,
    title: String,
    base: String,
    dir: String,
    additions: String,
    deletions: String,
}

#[derive(Template)]
#[template(path = "file_rail.html")]
struct Rail {
    rows: Vec<RailRow>,
    /// The anchor of the first file not listed, empty when all are.
    more_anchor: String,
    more: usize,
}

/// The rail for `files`, or `""` when there is none to list (the page then keeps one column).
pub(crate) fn render<'a>(files: impl IntoIterator<Item = &'a FileStatRow>) -> String {
    let all: Vec<&FileStatRow> = files.into_iter().collect();
    if all.is_empty() {
        return String::new();
    }
    let rows = all.iter().take(RAIL_ROWS).map(|f| row(f)).collect();
    let more = all.len().saturating_sub(RAIL_ROWS);
    let more_anchor = all.get(RAIL_ROWS).map(|f| f.anchor.clone()).unwrap_or_default();
    // A template that fails leaves the rail out: the files card still lists every file.
    Rail { rows, more_anchor, more }.render().unwrap_or_default()
}

fn row(f: &FileStatRow) -> RailRow {
    // A rename reads `old -> new`: the rail names the file as it is now.
    let path = f.label.rsplit(" -> ").next().unwrap_or(&f.label);
    let (dir, base) = path.rsplit_once('/').unwrap_or(("", path));
    RailRow {
        anchor: f.anchor.clone(),
        title: f.label.clone(),
        base: base.to_owned(),
        dir: if dir.is_empty() { String::new() } else { format!("{dir}/") },
        additions: f.additions.clone(),
        deletions: f.deletions.clone(),
    }
}

#[cfg(test)]
mod tests {
    use crate::{file_rail::render, views::FileStatRow};

    fn row(i: usize, label: &str) -> FileStatRow {
        FileStatRow {
            anchor: format!("f{i}"),
            label: label.to_owned(),
            additions: "3".to_owned(),
            deletions: "1".to_owned(),
        }
    }

    #[test]
    fn rail_links_every_file_to_the_anchor_the_card_uses() {
        let html = render(&[row(0, "src/a.rs"), row(1, "b.rs")]);
        assert!(html.contains("href=\"#f0\""));
        assert!(html.contains("href=\"#f1\""));
        assert!(html.contains("<b>a.rs</b><span class=\"dir\">src/</span>"));
        assert!(html.contains("<b>b.rs</b><span class=\"dir\"></span>"));
    }

    #[test]
    fn rail_names_a_renamed_file_by_its_new_path() {
        let html = render(&[row(0, "old/x.rs -> new/y.rs")]);
        assert!(html.contains("<b>y.rs</b><span class=\"dir\">new/</span>"));
        assert!(html.contains("title=\"old/x.rs -&#62; new/y.rs\""));
    }

    #[test]
    fn rail_escapes_paths_and_is_empty_without_files() {
        assert!(render(&[row(0, "a<b>.rs")]).contains("a&#60;b&#62;.rs"));
        assert_eq!(render(&Vec::<FileStatRow>::new()), "");
    }

    #[test]
    fn rail_stops_at_the_cap_and_links_the_first_file_left_out() {
        let files: Vec<FileStatRow> = (0..120).map(|i| row(i, "d/f.rs")).collect();
        let html = render(&files);
        assert_eq!(html.matches("<li ").count(), super::RAIL_ROWS);
        assert!(html.contains("href=\"#f50\">and 70 more files"));
        let few: Vec<FileStatRow> = (0..3).map(|i| row(i, "d/f.rs")).collect();
        assert!(!render(&few).contains("more files"));
    }
}
