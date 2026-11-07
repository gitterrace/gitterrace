//! The legal pages' sources: they must exist, and a deploy must not carry a placeholder.
//!
//! `check` runs in every gate and only needs the sources to be there. `deploy_check` runs before
//! `wrangler deploy` and refuses any `{{PLACEHOLDER}}` the owner has not filled in yet.

use std::path::Path;

use anyhow::{Context as _, Result};

use crate::docs_index::LEGAL;

/// Every `{{...}}` with a capital letter inside, with its line number. Braces, not angle
/// brackets: Markdown would hand `<STREET>` to the HTML parser, which drops it silently, and an
/// unfilled imprint would look finished.
pub(crate) fn placeholders(text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let mut rest = line;
        while let Some((_, after)) = rest.split_once("{{") {
            let Some((inner, tail)) = after.split_once("}}") else {
                break;
            };
            if inner.chars().any(|c| c.is_ascii_uppercase()) && !inner.contains(['{', '}']) {
                out.push((n + 1, format!("{{{{{inner}}}}}")));
            }
            rest = tail;
        }
    }
    out
}

/// The legal sources as (repo path, text). An empty or missing set fails, so a moved file cannot
/// make a scan pass by finding nothing.
pub(crate) fn sources(root: &Path) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    for (rel, _) in LEGAL {
        let text = std::fs::read_to_string(root.join(rel))
            .with_context(|| format!("{rel}: the legal page source is missing"))?;
        if text.trim().is_empty() {
            anyhow::bail!("{rel}: the legal page source is empty");
        }
        out.push((rel.to_owned(), text));
    }
    if out.is_empty() {
        anyhow::bail!("no legal page sources are registered, nothing was checked");
    }
    Ok(out)
}

/// The problems that block a deploy: one line per placeholder left in a legal page.
pub(crate) fn deploy_problems(sources: &[(String, String)]) -> Vec<String> {
    let mut bad = Vec::new();
    for (rel, text) in sources {
        for (line, ph) in placeholders(text) {
            bad.push(format!("{rel}:{line}: {ph}"));
        }
    }
    bad
}

/// `cargo xtask deploy-check`: fails while a legal page still has a placeholder.
pub fn deploy_check(root: &Path) -> Result<Vec<String>> {
    Ok(deploy_problems(&sources(root)?))
}

/// What `check` requires of the sources: they are there and not empty.
pub fn check(root: &Path) -> Result<()> {
    sources(root).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_braced_text_with_a_capital_is_a_placeholder() {
        let text = "a {{STREET_AND_NUMBER}} b\n{{lower}} {{X: why}} {x}\n";
        let found: Vec<_> = placeholders(text).into_iter().map(|p| p.1).collect();
        assert_eq!(found, ["{{STREET_AND_NUMBER}}", "{{X: why}}"]);
    }

    /// Source mode: the deploy gate refuses a placeholder and passes a filled page.
    #[test]
    fn a_planted_placeholder_refuses_the_deploy_and_a_filled_page_passes() {
        let filled = vec![("p.md".to_owned(), "Jane Doe, Main Street 1\n".to_owned())];
        assert!(deploy_problems(&filled).is_empty());
        let planted = vec![(
            "p.md".to_owned(),
            "Jane\n{{STREET_AND_NUMBER}}\n".to_owned(),
        )];
        let bad = deploy_problems(&planted);
        assert_eq!(bad, ["p.md:2: {{STREET_AND_NUMBER}}"]);
    }

    /// Every gitterrace.dev mailbox the legal pages name is one of the two the site defines,
    /// and security.txt's first Contact is the security one.
    #[test]
    fn legal_pages_and_security_txt_use_the_defined_mailboxes() {
        let root = crate::repo_root();
        for (rel, text) in sources(&root).expect("sources") {
            for word in text.split(|c: char| !(c.is_ascii_alphanumeric() || "@.-_".contains(c))) {
                if word.ends_with("@gitterrace.dev") {
                    assert!(
                        [crate::site::SUPPORT_EMAIL, crate::site::SECURITY_EMAIL].contains(&word),
                        "{rel}: {word} is not a mailbox the site defines"
                    );
                }
            }
        }
        let ws = crate::facts::load(&root).expect("facts");
        let outs = crate::site::render(&root, &ws).expect("render");
        let txt = outs
            .iter()
            .find_map(|o| match o {
                crate::out::Out::File { path, body } if path.ends_with("security.txt") => {
                    Some(body.clone())
                }
                _ => None,
            })
            .expect("security.txt");
        let first = txt.lines().next().expect("a line");
        assert_eq!(
            first,
            format!("Contact: mailto:{}", crate::site::SECURITY_EMAIL)
        );
        assert!(
            txt.lines().filter(|l| l.starts_with("Contact:")).count() == 2,
            "{txt}"
        );
    }

    fn foreign_mailboxes(label: &str, text: &str) -> Vec<String> {
        text.split(|c: char| !(c.is_ascii_alphanumeric() || "@.-_".contains(c)))
            .filter(|w| w.ends_with("@gitterrace.dev"))
            .filter(|w| ![crate::site::SUPPORT_EMAIL, crate::site::SECURITY_EMAIL].contains(w))
            .map(|w| format!("{label}: {w}"))
            .collect()
    }

    fn markdown_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("read_dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                markdown_files(&path, out);
            } else if path.extension().is_some_and(|e| e == "md") {
                out.push(path);
            }
        }
    }

    /// Any gitterrace.dev address in a generated site file or a published doc is one of the two
    /// the site defines. The planted address is seen red.
    #[test]
    fn no_file_names_a_gitterrace_mailbox_the_site_does_not_define() {
        let planted = foreign_mailboxes(
            "plant",
            "write hello@gitterrace.dev or support@gitterrace.dev",
        );
        assert_eq!(planted, ["plant: hello@gitterrace.dev"]);

        let root = crate::repo_root();
        let ws = crate::facts::load(&root).expect("facts");
        let mut bad = Vec::new();
        for out in crate::site::render(&root, &ws).expect("render") {
            if let crate::out::Out::File { path, body } = out {
                bad.extend(foreign_mailboxes(&path, &body));
            }
        }
        let mut files = vec![
            root.join("README.md"),
            root.join("CONTRIBUTING.md"),
            root.join("SECURITY.md"),
        ];
        markdown_files(&root.join("docs"), &mut files);
        for file in files {
            if let Ok(text) = std::fs::read_to_string(&file) {
                bad.extend(foreign_mailboxes(&file.display().to_string(), &text));
            }
        }
        assert!(bad.is_empty(), "{bad:?}");
    }

    #[test]
    fn the_real_sources_exist_and_carry_no_placeholder() {
        let root = crate::repo_root();
        check(&root).expect("sources exist");
        let bad = deploy_check(&root).expect("scan");
        assert!(bad.is_empty(), "{bad:?}");
    }

    /// A scan target that matches nothing fails, so a moved file cannot pass vacuously.
    #[test]
    fn a_missing_or_empty_source_fails_instead_of_passing_vacuously() {
        let dir = std::env::temp_dir().join(format!("gt-legal-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("docs/legal")).expect("mkdir");
        let err = sources(&dir).expect_err("missing").to_string();
        assert!(err.contains("source is missing"), "{err}");
        for (rel, _) in LEGAL {
            std::fs::write(dir.join(rel), " \n").expect("write");
        }
        let err = sources(&dir).expect_err("empty").to_string();
        assert!(err.contains("source is empty"), "{err}");
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }
}
