//! The one way generated output reaches the tree.
//!
//! A generator returns [`Out`] values and never touches the file system. [`apply`] writes them
//! (`generate`) or compares them with the tree (`check`), so both commands run the same code and
//! a check cannot mutate the tree.

use std::path::Path;

use anyhow::{Context as _, Result};

/// One piece of generated output. Paths are relative to the repo root.
pub enum Out {
    /// A whole file.
    File { path: String, body: String },
    /// A whole binary file (a vendored font).
    Bytes { path: String, body: Vec<u8> },
    /// The text between `<!-- BEGIN GENERATED: <id> -->` and `<!-- END GENERATED: <id> -->` in a
    /// hand-written file.
    Block {
        path: String,
        id: String,
        body: String,
    },
}

impl Out {
    pub fn file(path: impl Into<String>, body: impl Into<String>) -> Self {
        Self::File {
            path: path.into(),
            body: body.into(),
        }
    }

    pub fn bytes(path: impl Into<String>, body: Vec<u8>) -> Self {
        Self::Bytes {
            path: path.into(),
            body,
        }
    }

    pub fn block(path: impl Into<String>, id: impl Into<String>, body: impl Into<String>) -> Self {
        Self::Block {
            path: path.into(),
            id: id.into(),
            body: body.into(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Write,
    Check,
}

/// Write `outs` or compare them with the tree. Returns what differed (a file, or `file#block`);
/// in [`Mode::Write`] those are the ones rewritten. A missing file, a missing or unclosed marker
/// is an error.
pub fn apply(root: &Path, outs: Vec<Out>, mode: Mode) -> Result<Vec<String>> {
    let mut stale = Vec::new();
    for out in outs {
        if let Out::Bytes { path, body } = out {
            let file = root.join(&path);
            if std::fs::read(&file).ok().as_deref() == Some(body.as_slice()) {
                continue;
            }
            if mode == Mode::Write {
                if let Some(parent) = file.parent() {
                    std::fs::create_dir_all(parent).with_context(|| format!("creating {path}"))?;
                }
                std::fs::write(&file, &body).with_context(|| format!("writing {path}"))?;
            }
            stale.push(path);
            continue;
        }
        let (path, label, next, current) = match out {
            Out::File { path, body } => {
                let current = read(&root.join(&path))?;
                let body = if Path::new(&path).extension().is_some_and(|e| e == "md") {
                    wrap_markdown(&body)
                } else {
                    body
                };
                (path.clone(), path, body, current)
            }
            Out::Bytes { .. } => continue,
            Out::Block { path, id, body } => {
                let current =
                    read(&root.join(&path))?.with_context(|| format!("{path} does not exist"))?;
                let body = if Path::new(&path).extension().is_some_and(|e| e == "md") {
                    wrap_markdown(&body)
                } else {
                    body
                };
                let next = splice(&current, &id, &body).with_context(|| path.clone())?;
                (path.clone(), format!("{path}#{id}"), next, Some(current))
            }
        };
        if current.as_deref() == Some(next.as_str()) {
            continue;
        }
        if mode == Mode::Write {
            let file = root.join(&path);
            if let Some(parent) = file.parent() {
                std::fs::create_dir_all(parent).with_context(|| format!("creating {path}"))?;
            }
            std::fs::write(&file, &next).with_context(|| format!("writing {path}"))?;
        }
        stale.push(label);
    }
    Ok(stale)
}

fn read(path: &Path) -> Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

/// Replace the body of block `id`. A blank line always closes a non-empty block, so a block
/// ending in a list cannot absorb the END marker as a lazy continuation.
fn splice(text: &str, id: &str, body: &str) -> Result<String> {
    let begin = format!("<!-- BEGIN GENERATED: {id} -->\n");
    let end = format!("<!-- END GENERATED: {id} -->");
    let start = text
        .find(&begin)
        .with_context(|| format!("no `{}` marker", begin.trim_end()))?
        + begin.len();
    let stop = start
        + text[start..]
            .find(&end)
            .with_context(|| format!("no `{end}` marker"))?;
    let body = body.trim_end_matches('\n');
    let filled = if body.is_empty() {
        String::new()
    } else {
        format!("{body}\n\n")
    };
    Ok(format!("{}{filled}{}", &text[..start], &text[stop..]))
}

/// Prose column limit for generated markdown, the same one the kit's markdownlint prose-length
/// rule enforces on every tracked page.
pub const PROSE_WIDTH: usize = 80;

/// Wraps the prose paragraphs of generated markdown at [`PROSE_WIDTH`]. Tables, fences, headings,
/// HTML comments and list items stay as written: they cannot be reflowed without changing what
/// they render. A paragraph with a line over the width is reflowed as a whole, so a source string
/// wrapped at another width leaves no one-word line behind. A word longer than the limit (a URL,
/// a long code span) keeps its own line.
pub fn wrap_markdown(md: &str) -> String {
    let mut out = String::with_capacity(md.len());
    let mut fence = false;
    let mut para: Vec<&str> = Vec::new();
    for line in md.split_inclusive('\n') {
        let text = line.trim_end_matches('\n');
        let trimmed = text.trim_start();
        if trimmed.starts_with("```") {
            fence = !fence;
        }
        let structure = fence
            || trimmed.starts_with("```")
            || trimmed.starts_with('|')
            || trimmed.starts_with('#')
            || trimmed.starts_with('<')
            || trimmed.starts_with("- ")
            || trimmed.starts_with("* ")
            || trimmed.starts_with('>')
            || trimmed.is_empty();
        let indent = |t: &str| t.len() - t.trim_start().len();
        if !para.is_empty() && (structure || indent(para[0]) != indent(text)) {
            flush(&mut out, &mut para);
        }
        if structure {
            out.push_str(line);
        } else {
            para.push(text);
        }
    }
    flush(&mut out, &mut para);
    // A paragraph at the end of input without a newline gets none.
    if !md.ends_with('\n') && out.ends_with('\n') {
        out.pop();
    }
    out
}

/// Writes the lines of one prose paragraph: as they are when each fits, else reflowed together.
fn flush(out: &mut String, para: &mut Vec<&str>) {
    if para.iter().all(|l| l.chars().count() <= PROSE_WIDTH) {
        for l in para.iter() {
            out.push_str(l);
            out.push('\n');
        }
    } else {
        let first = para[0];
        let indent = &first[..first.len() - first.trim_start().len()];
        let joined: Vec<&str> = para.iter().map(|l| l.trim()).collect();
        out.push_str(&wrap_line(&joined.join(" "), indent));
        out.push('\n');
    }
    para.clear();
}

/// Breaks one prose line at spaces. A code span never breaks: its spaces become no-break
/// spaces while wrapping, which `textwrap` keeps inside a word, and turn back after.
fn wrap_line(text: &str, indent: &str) -> String {
    let mut guarded = String::with_capacity(text.len());
    let mut in_code = false;
    for ch in text.chars() {
        if ch == '`' {
            in_code = !in_code;
        }
        guarded.push(if in_code && ch == ' ' { '\u{a0}' } else { ch });
    }
    let opts = textwrap::Options::new(PROSE_WIDTH)
        .initial_indent(indent)
        .subsequent_indent(indent)
        .break_words(false)
        .word_splitter(textwrap::WordSplitter::NoHyphenation)
        .word_separator(textwrap::WordSeparator::AsciiSpace);
    textwrap::fill(&guarded, &opts).replace('\u{a0}', " ")
}

/// The sorted names of the files directly in `dir` with extension `ext`.
pub fn files_in(dir: &Path, ext: &str) -> Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let name = entry?.file_name().to_string_lossy().into_owned();
        if Path::new(&name).extension().is_some_and(|e| e == ext) {
            names.push(name);
        }
    }
    names.sort();
    Ok(names)
}

/// Markers in the hand-written markdown that no generator fills. A block nobody generates would
/// rot silently, so this is a staleness finding like any other.
pub fn unowned_blocks(root: &Path, outs: &[Out]) -> Result<Vec<String>> {
    let owned: Vec<(&str, &str)> = outs
        .iter()
        .filter_map(|o| match o {
            Out::Block { path, id, .. } => Some((path.as_str(), id.as_str())),
            Out::File { .. } | Out::Bytes { .. } => None,
        })
        .collect();
    let mut pages = vec!["README.md".to_owned(), "AGENTS.md".to_owned()];
    if let Ok(entries) = std::fs::read_dir(root.join("docs")) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if Path::new(&name).extension().is_some_and(|e| e == "md") {
                pages.push(format!("docs/{name}"));
            }
        }
    }
    pages.sort();
    let mut out = Vec::new();
    for path in pages {
        let Some(text) = read(&root.join(&path))? else {
            continue;
        };
        for line in text.lines() {
            if let Some(id) = line
                .strip_prefix("<!-- BEGIN GENERATED: ")
                .and_then(|r| r.strip_suffix(" -->"))
                && !owned.contains(&(path.as_str(), id))
            {
                out.push(format!("{path}#{id} has no generator"));
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use crate::out::{Mode, Out, PROSE_WIDTH, apply, splice, unowned_blocks, wrap_markdown};

    #[test]
    fn generated_prose_wraps_and_structure_stays() {
        let long = "word ".repeat(40);
        let code = format!("`{}` tail", "x".repeat(120));
        let table = format!("| {} |", "c".repeat(150));
        let md = format!("{long}\n\n{table}\n\n```\n{long}\n```\n\n# {long}\n\n{code}\n");
        let out = wrap_markdown(&md);
        let fence = out.split("```").nth(1).expect("fence");
        assert!(fence.contains(long.trim_end()), "fence kept");
        assert!(out.contains(&table), "table kept");
        assert!(out.contains(&format!("# {long}")), "heading kept");
        assert!(
            out.contains(&format!("`{}`", "x".repeat(120))),
            "code span unbroken"
        );
        let prose = out.split("\n\n").next().expect("first paragraph");
        assert!(prose.lines().count() > 1);
        assert!(prose.lines().all(|l| l.chars().count() <= PROSE_WIDTH));
        assert_eq!(wrap_markdown(&out), out, "idempotent");
    }

    /// The class this guards: a generated paragraph whose source lines were wrapped at another
    /// width, which wrapping line by line leaves with one-word lines.
    #[test]
    fn a_paragraph_reflows_as_a_whole_without_one_word_lines() {
        let src = format!(
            "{} are\n`code span` and more words here.\nShort.\n\nNext.\n",
            "word ".repeat(16).trim_end()
        );
        let out = wrap_markdown(&src);
        let first = out.split("\n\n").next().expect("paragraph");
        assert!(
            first.lines().all(|l| l.chars().count() <= PROSE_WIDTH),
            "{out}"
        );
        assert!(
            first.lines().all(|l| l.split(' ').count() > 1),
            "one-word line:\n{out}"
        );
        assert!(out.ends_with("\n\nNext.\n"), "{out}");
        let fits = "a short line\nanother short line\n";
        assert_eq!(wrap_markdown(fits), fits, "fitting lines stay as written");
        assert_eq!(wrap_markdown(&out), out, "idempotent");
    }

    const PAGE: &str = "intro\n<!-- BEGIN GENERATED: a -->\nold\n<!-- END GENERATED: a -->\n\
                        middle\n<!-- BEGIN GENERATED: b -->\nkeep\n<!-- END GENERATED: b -->\n";

    #[test]
    fn splice_replaces_only_its_block_and_is_idempotent() {
        let out = splice(PAGE, "a", "- new\n").expect("splice");
        assert!(out.contains("<!-- BEGIN GENERATED: a -->\n- new\n\n<!-- END GENERATED: a -->"));
        assert!(out.contains("keep\n<!-- END GENERATED: b -->"));
        assert_eq!(splice(&out, "a", "- new").expect("splice"), out);
    }

    #[test]
    fn splice_refuses_a_missing_or_unclosed_marker() {
        assert!(splice(PAGE, "c", "x").is_err());
        assert!(splice("<!-- BEGIN GENERATED: a -->\nx\n", "a", "y").is_err());
    }

    #[test]
    fn a_block_without_a_generator_is_reported() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(tmp.path().join("README.md"), PAGE).expect("write");
        let outs = vec![Out::block("README.md", "a", "x")];
        assert_eq!(
            unowned_blocks(tmp.path(), &outs).expect("scan"),
            ["README.md#b has no generator"]
        );
    }

    #[test]
    fn check_reports_a_stale_block_and_writes_nothing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(tmp.path().join("README.md"), PAGE).expect("write");
        let outs = || vec![Out::block("README.md", "a", "fresh")];
        assert_eq!(
            apply(tmp.path(), outs(), Mode::Check).expect("check"),
            ["README.md#a"]
        );
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("README.md")).expect("read"),
            PAGE
        );
        apply(tmp.path(), outs(), Mode::Write).expect("write");
        assert!(
            apply(tmp.path(), outs(), Mode::Check)
                .expect("check")
                .is_empty()
        );
    }

    #[test]
    fn check_reports_without_writing_and_write_converges() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let outs = || vec![Out::file("docs/x.md", "new\n")];

        let stale = apply(root, outs(), Mode::Check).expect("check");
        assert_eq!(stale, ["docs/x.md"]);
        assert!(!root.join("docs/x.md").exists(), "check wrote to the tree");

        apply(root, outs(), Mode::Write).expect("write");
        assert!(apply(root, outs(), Mode::Check).expect("check").is_empty());
    }
}
