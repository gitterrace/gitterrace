//! `THIRD-PARTY-NOTICES`: the licence texts of the assets compiled into the binary. A font under
//! the OFL must travel with its copyright notice and licence, and the mermaid bundle with its MIT
//! notice, in every archive and image. The file is shipped by the release archives and the
//! container image.

use std::path::Path;

use anyhow::{Context as _, Result};

use crate::out::Out;

/// The file, relative to the repo root.
const PATH: &str = "THIRD-PARTY-NOTICES";

/// Each embedded third-party asset and the file holding its notice, in output order.
const SOURCES: [(&str, &str); 4] = [
    ("Fraunces", "vendor/schwarzwald/kit/fonts/fraunces/OFL.txt"),
    ("Bricolage Grotesque", "vendor/schwarzwald/kit/fonts/bricolage-grotesque/OFL.txt"),
    ("JetBrains Mono", "vendor/schwarzwald/kit/fonts/jetbrains-mono/OFL.txt"),
    ("mermaid", "crates/gitterrace-web/static/vendor/mermaid.LICENSE"),
];

pub fn render(root: &Path) -> Result<Out> {
    let mut body = String::from(
        "Third-party notices\n\nThe assets below are compiled into git-terrace. Each is shipped\n\
         under the licence that follows its name.\n",
    );
    for (name, file) in SOURCES {
        let text = std::fs::read_to_string(root.join(file))
            .with_context(|| format!("reading the notice for {name}: {file}"))?;
        body.push_str("\n## ");
        body.push_str(name);
        body.push_str("\n\n");
        body.push_str(text.trim_end());
        body.push('\n');
    }
    Ok(Out::file(PATH, body))
}

#[cfg(test)]
mod tests {
    use super::{SOURCES, render};
    use crate::out::Out;

    fn body() -> String {
        match render(&crate::repo_root()).expect("render") {
            Out::File { body, .. } => body,
            _ => panic!("notices are one text file"),
        }
    }

    #[test]
    fn every_source_text_and_header_is_in_the_file() {
        let body = body();
        let root = crate::repo_root();
        for (name, file) in SOURCES {
            assert!(body.contains(&format!("## {name}\n")), "{name}: no header");
            let text = std::fs::read_to_string(root.join(file)).expect("read");
            assert!(body.contains(text.trim_end()), "{name}: text missing");
        }
    }

    #[test]
    fn rendering_twice_gives_identical_output() {
        assert_eq!(body(), body());
    }

    #[test]
    fn every_embedded_font_and_vendored_script_has_a_notice() {
        let root = crate::repo_root();
        let assets =
            std::fs::read_to_string(root.join("crates/gitterrace-web/src/routes/assets.rs"))
                .expect("assets.rs");
        let listed = |needle: &str| SOURCES.iter().any(|(_, f)| f.contains(needle));
        for dir in ["fraunces", "bricolage-grotesque", "jetbrains-mono"] {
            assert!(assets.contains(dir), "{dir} no longer embedded: update this list");
        }
        // A font folder under the kit that assets.rs embeds needs a notice entry.
        let fonts = root.join("vendor/schwarzwald/kit/fonts");
        for entry in std::fs::read_dir(fonts).expect("fonts dir") {
            let name = entry.expect("entry").file_name().to_string_lossy().into_owned();
            if assets.contains(&format!("{name}/")) || assets.contains(&name) {
                assert!(listed(&format!("fonts/{name}/")), "embedded font {name}: no notice");
            }
        }
        // Every vendored script needs a LICENSE beside it, listed in SOURCES.
        let vendor = root.join("crates/gitterrace-web/static/vendor");
        for entry in std::fs::read_dir(vendor).expect("vendor dir") {
            let name = entry.expect("entry").file_name().to_string_lossy().into_owned();
            let stem = name.split('.').next().unwrap_or(&name);
            assert!(listed(&format!("vendor/{stem}.LICENSE")), "vendored {name}: no notice");
        }
    }
}
