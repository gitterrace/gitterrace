//! One top bar: `_base.html` writes the `<header class="top">` and every page template extends
//! it. The repositories home is the one page that builds its own bar (it has no repository),
//! so a new template that opens a header of its own fails here.

use std::path::Path;

const OWN_BAR: &[&str] = &["repos.html"];

#[test]
fn only_the_base_template_writes_a_top_bar() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("templates");
    let mut bad = Vec::new();
    for e in std::fs::read_dir(dir).expect("templates") {
        let p = e.expect("entry").path();
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        let html = p.extension().is_some_and(|x| x == "html");
        if !html || name == "_base.html" || OWN_BAR.contains(&name) {
            continue;
        }
        let text = std::fs::read_to_string(&p).expect("read");
        if text.contains("<header class=\"top\"") || text.contains("class=\"topbar\"") {
            bad.push(name.to_owned());
        }
        if text.contains("<!doctype") && !text.contains("{% extends") {
            bad.push(format!(
                "{name}: a full page that does not extend _base.html"
            ));
        }
    }
    assert!(bad.is_empty(), "own header: {}", bad.join(", "));
}
