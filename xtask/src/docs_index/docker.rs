//! The container port prose names, checked against the `Dockerfile`: every `docker run -p`
//! mapping and every `http://localhost:N` must use the port the image exposes, and the image's
//! `--addr` must listen on that same port.

use std::path::Path;

/// The port of the `Dockerfile`'s `EXPOSE`, and a problem when its `--addr` disagrees.
pub(super) fn port(root: &Path) -> (Option<u16>, Vec<String>) {
    let Ok(file) = std::fs::read_to_string(root.join("Dockerfile")) else {
        return (None, Vec::new());
    };
    let exposed = file
        .lines()
        .find_map(|l| l.strip_prefix("EXPOSE "))
        .and_then(|p| p.trim().parse::<u16>().ok());
    let addr = file
        .lines()
        .find(|l| l.starts_with("ENTRYPOINT"))
        .and_then(|l| l.split("\"--addr\", \"").nth(1))
        .and_then(|a| a.split('"').next())
        .and_then(|a| a.rsplit_once(':'))
        .and_then(|(_, p)| p.parse::<u16>().ok());
    let mut bad = Vec::new();
    if exposed.is_none() {
        bad.push("Dockerfile: no EXPOSE port".to_owned());
    } else if addr != exposed {
        bad.push(format!(
            "Dockerfile: --addr port {addr:?} is not the EXPOSE port {exposed:?}"
        ));
    }
    (exposed, bad)
}

/// Ports in `text` that are not `port`: the container side of `-p host:N:N` and `localhost:N`.
pub(super) fn check(path: &str, text: &str, port: u16) -> Vec<String> {
    let mut bad = Vec::new();
    let words = text.split(|c: char| c.is_whitespace() || c == '`');
    let mut words = words.peekable();
    while let Some(w) = words.next() {
        let mapped = (w == "-p")
            .then(|| words.peek().copied())
            .flatten()
            .and_then(|m| m.rsplit(':').next());
        let local = w
            .split("localhost:")
            .nth(1)
            .map(|p| p.trim_end_matches(|c: char| !c.is_ascii_digit()));
        for found in [mapped, local].into_iter().flatten() {
            if found.parse::<u16>().is_ok_and(|p| p != port) {
                bad.push(format!(
                    "{path}: container port {found} is not the Dockerfile's {port}"
                ));
            }
        }
    }
    bad
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A wrong port in a run command or a URL goes red, and the published pages and the
    /// Dockerfile agree.
    #[test]
    fn prose_that_names_another_container_port_fails() {
        let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/.."));
        let (port, bad) = super::port(root);
        assert_eq!(bad, Vec::<String>::new());
        let port = port.expect("EXPOSE");
        for wrong in [
            "`docker run --rm -p 127.0.0.1:9090:9090 -v x gitterrace`",
            "Open `http://localhost:9090`.",
        ] {
            assert_eq!(check("x.md", wrong, port).len(), 1, "{wrong}");
        }
        assert!(check("x.md", &format!("`http://localhost:{port}/r/`"), port).is_empty());
        for doc in ["README.md", "docs/online.md"] {
            let text = std::fs::read_to_string(root.join(doc)).expect("doc");
            assert!(text.contains(&format!("localhost:{port}")), "{doc}");
            assert_eq!(check(doc, &text, port), Vec::<String>::new());
        }
    }
}
