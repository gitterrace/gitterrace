//! A Jupyter notebook (`.ipynb`) rendered as its cells. The file is untrusted JSON: markdown
//! cells go through the sanitizing markdown renderer, code is escaped by the highlighter, text
//! outputs are escaped, and only PNG and JPEG outputs show, as `data:` images (the CSP's
//! `img-src` allows `data:`). HTML, JavaScript and every other rich output is dropped.

use askama::Template;
use serde_json::Value;

use crate::markdown;

#[cfg(test)]
mod wiring_tests;

/// Most cells rendered; the rest are counted below the last one. Not configurable: a read
/// bound on a file from the repository, which may be hostile.
const MAX_CELLS: usize = 500;
/// Most bytes of outputs (text and image data) rendered across the notebook. Not
/// configurable: a read bound on a file from the repository, which may be hostile.
const NOTEBOOK_OUTPUT_CAP: usize = 4 << 20;

enum Out {
    Text(String),
    Image { mime: &'static str, data: String },
    Dropped,
}

enum Cell {
    Markdown(String),
    Code { count: String, html: String, outputs: Vec<Out> },
    Raw(String),
}

#[derive(Template)]
#[template(path = "notebook.html")]
struct Notebook {
    cells: Vec<Cell>,
    cut: usize,
}

pub(crate) fn is_notebook(path: &str) -> bool {
    path.to_ascii_lowercase().ends_with(".ipynb")
}

/// The blob page's rendered pane for a notebook, shown with its source like Markdown; `None`
/// for any other file or a notebook that does not parse, which then shows as source.
pub(crate) fn doc(
    text: &str,
    path: &str,
    rev: &str,
    r: crate::views::Render,
) -> Option<markdown::Doc> {
    if !is_notebook(path) {
        return None;
    }
    let dir = path.rsplit_once('/').map_or("", |(d, _)| d);
    let html = render(text, || markdown::Base::new(rev, dir).highlight(r.highlight_bytes))?;
    Some(markdown::Doc { html, outline: Vec::new() })
}

/// The notebook's cells as HTML, or `None` when the text is not a notebook.
pub(crate) fn render(src: &str, base: impl Fn() -> markdown::Base) -> Option<String> {
    let v: Value = serde_json::from_str(src).ok()?;
    let all = v.get("cells")?.as_array()?;
    let lang = v
        .pointer("/metadata/language_info/name")
        .or_else(|| v.pointer("/metadata/kernelspec/language"))
        .and_then(Value::as_str)
        .unwrap_or("python");
    let mut budget = NOTEBOOK_OUTPUT_CAP;
    let cells = all.iter().take(MAX_CELLS).map(|c| cell(c, lang, &base, &mut budget)).collect();
    Notebook { cells, cut: all.len().saturating_sub(MAX_CELLS) }.render().ok()
}

/// A notebook string is either a string or a list of lines.
fn text(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).collect(),
        _ => String::new(),
    }
}

fn cell(c: &Value, lang: &str, base: &impl Fn() -> markdown::Base, budget: &mut usize) -> Cell {
    let src = text(c.get("source"));
    match c.get("cell_type").and_then(Value::as_str) {
        Some("markdown") => Cell::Markdown(markdown::render_doc(&src, base()).html),
        Some("code") => {
            let most = base().highlight_bytes;
            Cell::Code {
                count: c
                    .get("execution_count")
                    .and_then(Value::as_u64)
                    .map_or_else(|| " ".to_owned(), |n| n.to_string()),
                html: crate::highlight::block(&src, lang, most)
                    .unwrap_or_else(|| crate::fmt::esc_text(&src)),
                outputs: c
                    .get("outputs")
                    .and_then(Value::as_array)
                    .map(|o| o.iter().map(|o| output(o, budget)).collect())
                    .unwrap_or_default(),
            }
        }
        _ => Cell::Raw(src),
    }
}

fn output(o: &Value, budget: &mut usize) -> Out {
    let out = match o.get("output_type").and_then(Value::as_str) {
        Some("stream") => Out::Text(text(o.get("text"))),
        Some("error") => Out::Text(format!("{}: {}", text(o.get("ename")), text(o.get("evalue")))),
        Some("execute_result" | "display_data") => rich(o.get("data")),
        _ => Out::Dropped,
    };
    let size = match &out {
        Out::Text(t) => t.len(),
        Out::Image { data, .. } => data.len(),
        Out::Dropped => 0,
    };
    if size > *budget {
        return Out::Dropped;
    }
    *budget -= size;
    out
}

/// The one shown form of a rich output: an image, else plain text. HTML, JS, SVG and the
/// rest are never read.
fn rich(data: Option<&Value>) -> Out {
    let Some(d) = data else { return Out::Dropped };
    for mime in ["image/png", "image/jpeg"] {
        let b64: String = text(d.get(mime)).chars().filter(|c| !c.is_ascii_whitespace()).collect();
        if !b64.is_empty()
            && b64.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='))
        {
            return Out::Image { mime, data: b64 };
        }
    }
    d.get("text/plain").map_or(Out::Dropped, |t| Out::Text(text(Some(t))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nb(cells: &str) -> String {
        format!(r#"{{"metadata":{{"language_info":{{"name":"python"}}}},"cells":[{cells}]}}"#)
    }

    fn go(src: &str) -> Option<String> {
        render(src, || markdown::Base::new("HEAD", ""))
    }

    #[test]
    fn markdown_cells_render_as_markdown() {
        let h = go(&nb(r##"{"cell_type":"markdown","source":["# Title\n","*em*"]}"##))
            .expect("notebook");
        assert!(h.contains("<em>em</em>"), "{h}");
        assert!(h.contains("Title"));
    }

    #[test]
    fn code_cells_are_highlighted_with_their_count() {
        let h =
            go(&nb(r#"{"cell_type":"code","execution_count":3,"source":"import os\nx = '<b>'",
            "outputs":[{"output_type":"stream","text":["hi <i>\n"]}]}"#))
            .expect("notebook");
        assert!(h.contains("[3]"));
        assert!(h.contains("<span class="), "{h}");
        assert!(h.contains("<pre class=\"nb-out\">hi &#60;i&#62;"), "{h}");
        assert!(!h.contains("<b>") && !h.contains("<i>"));
    }

    #[test]
    fn html_and_script_outputs_are_dropped() {
        let h =
            go(&nb(r#"{"cell_type":"code","source":"","outputs":[{"output_type":"display_data",
            "data":{"text/html":"<script>alert(1)</script>","application/javascript":"alert(2)"}},
            {"output_type":"execute_result",
            "data":{"image/png":"iVBO\"><script>","text/html":"<img onerror=x>"}}]}"#))
            .expect("notebook");
        assert!(!h.contains("<script") && !h.contains("alert") && !h.contains("onerror"), "{h}");
        assert!(!h.contains("data:image"));
        assert!(h.contains("Output not shown"));
    }

    #[test]
    fn png_outputs_show_as_data_images() {
        let h =
            go(&nb(r#"{"cell_type":"code","source":"","outputs":[{"output_type":"display_data",
            "data":{"image/png":"iVBORw0K\nGgo=","text/plain":"<Figure>"}}]}"#))
            .expect("notebook");
        assert!(h.contains("src=\"data:image/png;base64,iVBORw0KGgo=\""), "{h}");
    }

    #[test]
    fn cells_past_the_cap_are_counted_not_rendered() {
        let one = r#"{"cell_type":"raw","source":"r"}"#;
        let h = go(&nb(&vec![one; MAX_CELLS + 7].join(","))).expect("notebook");
        assert_eq!(h.matches("class=\"nb-cell\"").count(), MAX_CELLS);
        assert!(h.contains("7 more cells not shown."));
    }

    #[test]
    fn outputs_past_the_byte_budget_are_dropped() {
        let big = "x".repeat(NOTEBOOK_OUTPUT_CAP / 2 + 1);
        let o = format!(r#"{{"output_type":"stream","text":"{big}"}}"#);
        let h = go(&nb(&format!(r#"{{"cell_type":"code","source":"","outputs":[{o},{o}]}}"#)))
            .expect("notebook");
        assert_eq!(h.matches(&big).count(), 1);
    }

    #[test]
    fn invalid_json_is_not_a_notebook() {
        assert!(go("{not json").is_none());
        assert!(go(r#"{"cells":3}"#).is_none());
    }
}
