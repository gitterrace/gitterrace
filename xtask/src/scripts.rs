//! The example scripts in `docs/actions/`: file name, what each does (the first sentence of its
//! header comment) and what it needs (its `Needs` paragraph). The scripts are the source, so
//! the tables cannot drift from them.

use std::path::Path;

use anyhow::{Context as _, Result};

use crate::out::files_in;

pub struct Script {
    pub file: String,
    pub summary: String,
    pub needs: String,
}

pub fn load(root: &Path) -> Result<Vec<Script>> {
    let dir = root.join("docs/actions");
    let mut out = Vec::new();
    for file in files_in(&dir, "sh")? {
        let text = std::fs::read_to_string(dir.join(&file)).with_context(|| file.clone())?;
        let comment: Vec<&str> = text
            .lines()
            .skip(1)
            .take_while(|l| l.starts_with('#'))
            .map(|l| l.trim_start_matches('#').trim())
            .collect();
        let first = comment
            .split(|l| l.is_empty())
            .next()
            .unwrap_or(&[])
            .join(" ");
        let summary = first
            .split_once(". ")
            .map_or_else(|| first.clone(), |(a, _)| format!("{a}."));
        let at = comment.iter().position(|l| l.starts_with("Needs"));
        let needs = at.map_or_else(String::new, |i| {
            let mut paragraph: Vec<&str> = comment[i..]
                .iter()
                .take_while(|l| !l.is_empty())
                .copied()
                .collect();
            if let Some(first) = paragraph.first_mut() {
                *first = first.trim_start_matches("Needs").trim_start();
            }
            paragraph.join(" ")
        });
        out.push(Script {
            file,
            summary,
            needs,
        });
    }
    Ok(out)
}
