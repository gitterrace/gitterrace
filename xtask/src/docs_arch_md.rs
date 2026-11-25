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
