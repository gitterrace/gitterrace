//! Generated blocks of `docs/architecture.md`.

use std::fmt::Write as _;

pub(crate) fn routes() -> String {
    let mut s = String::from("| Route | Shows |\n| --- | --- |\n");
    for p in gitterrace_web::sitemap::PAGES {
        let _ = writeln!(s, "| {} | {} |", p.shown, p.does);
    }
    s
}

/// The highlighted languages as one comma-separated paragraph, from the syntax set the binary
/// ships, so the list cannot drift from what the code view does.
pub(crate) fn languages() -> String {
    let names = gitterrace_web::highlighted_languages();
    format!("{} languages: {}.\n", names.len(), names.join(", "))
}

/// The files that hold the user's own data, from the list every cache reset spares.
pub(crate) fn user_files() -> String {
    let names: Vec<String> =
        gitterrace_index::USER_FILES.iter().map(|n| format!("`{n}`")).collect();
    let list = match names.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
        _ => names.concat(),
    };
    format!("{list} hold user state, so a wipe or an older build spares them.\n")
}

pub(crate) fn cache() -> String {
    let mut s = String::from("| File | Holds |\n| --- | --- |\n");
    for (name, doc) in gitterrace_index::CACHE_FILES {
        let _ = writeln!(s, "| `{name}` | {doc} |");
    }
    let _ = write!(
        s,
        "\nThe format version is `{}`, stored as `v{}`.\n",
        gitterrace_index::FORMAT_VERSION,
        gitterrace_index::FORMAT_VERSION
    );
    s
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    /// Every test the security pages name in backticks exists as a `fn` in the workspace, so a
    /// renamed or deleted test cannot leave a door "held" by nothing.
    #[test]
    fn every_test_the_security_pages_name_exists() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut source = String::new();
        let mut stack = vec![root.join("crates")];
        while let Some(dir) = stack.pop() {
            for e in std::fs::read_dir(&dir).expect("read dir").flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|x| x == "rs") {
                    source.push_str(&std::fs::read_to_string(&p).unwrap_or_default());
                }
            }
        }
        let mut bad = Vec::new();
        for page in ["docs/security.md", "docs/architecture.md"] {
            let text = std::fs::read_to_string(root.join(page)).expect("page");
            for word in text.split('`').skip(1).step_by(2) {
                let is_test =
                    word.len() > 24 && word.chars().all(|c| c.is_ascii_lowercase() || c == '_');
                if is_test && !source.contains(&format!("fn {word}(")) {
                    bad.push(format!("{page}: {word}"));
                }
            }
        }
        assert!(bad.is_empty(), "tests named in the docs do not exist: {bad:?}");
    }
}
