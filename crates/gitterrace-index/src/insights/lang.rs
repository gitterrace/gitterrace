//! File name to language, by extension and well-known file names.

const LANGUAGES: &[(&str, &[&str])] = &[
    ("Rust", &["rs"]),
    ("TypeScript", &["ts", "tsx", "mts", "cts"]),
    ("JavaScript", &["js", "jsx", "mjs", "cjs"]),
    ("Svelte", &["svelte"]),
    ("Vue", &["vue"]),
    ("Markdown", &["md", "mdx", "markdown"]),
    ("Python", &["py", "pyi"]),
    ("Shell", &["sh", "bash", "zsh", "fish"]),
    ("PowerShell", &["ps1", "psm1", "psd1"]),
    ("TOML", &["toml"]),
    ("JSON", &["json", "jsonc", "json5"]),
    ("YAML", &["yml", "yaml"]),
    ("CSS", &["css", "scss", "sass", "less"]),
    ("HTML", &["html", "htm"]),
    ("Go", &["go"]),
    ("Java", &["java"]),
    ("Kotlin", &["kt", "kts"]),
    ("Swift", &["swift"]),
    ("C/C++", &["c", "h", "cc", "cpp", "cxx", "hpp", "hh"]),
    ("C#", &["cs"]),
    ("Ruby", &["rb"]),
    ("SQL", &["sql"]),
    ("XML", &["xml", "plist", "xsd"]),
    ("SVG", &["svg"]),
    ("Text", &["txt"]),
    ("Nix", &["nix"]),
];

const BY_NAME: &[(&str, &str)] = &[
    ("Makefile", "Make"),
    ("GNUmakefile", "Make"),
    ("Dockerfile", "Docker"),
    ("Justfile", "Make"),
    ("justfile", "Make"),
    ("Containerfile", "Docker"),
    ("Gemfile", "Ruby"),
    ("Rakefile", "Ruby"),
    ("LICENSE", "Text"),
    ("LICENCE", "Text"),
    ("COPYING", "Text"),
    ("NOTICE", "Text"),
    ("Caddyfile", "Config"),
    ("Procfile", "Config"),
];

/// The part of `path` its language depends on.
///
/// `(true, extension)` when it has a usable extension and is no well-known name, else
/// `(false, file name)`. Paths with one key have
/// one language, so a caller over many paths asks [`language`] once per key, for the first
/// path it meets with that key. The flag keeps the dotfile `.rs` apart from the extension.
pub fn language_key(path: &str) -> (bool, &str) {
    let name = path.rsplit('/').next().unwrap_or(path);
    if BY_NAME.iter().any(|(n, _)| *n == name) {
        return (false, name);
    }
    match name.rfind('.') {
        Some(i) if i > 0 && i + 1 < name.len() => (true, &name[i + 1..]),
        _ => (false, name),
    }
}

/// Language of `path`: well-known file names first, then the extension. Unknown extensions
/// are their own language (`.xyz`), dotfiles are `Config`, the rest `(no extension)`.
pub fn language(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    if let Some((_, lang)) = BY_NAME.iter().find(|(n, _)| *n == name) {
        return (*lang).to_owned();
    }
    // Python's suffix: text after the last dot, but not for a leading dot only.
    let ext = match name.rfind('.') {
        Some(i) if i > 0 && i + 1 < name.len() => Some(name[i + 1..].to_lowercase()),
        _ => None,
    };
    if let Some(ext) = ext {
        if let Some((lang, _)) = LANGUAGES
            .iter()
            .find(|(_, exts)| exts.contains(&ext.as_str()))
        {
            return (*lang).to_owned();
        }
        return format!(".{ext}");
    }
    if name.starts_with('.') {
        return "Config".to_owned();
    }
    "(no extension)".to_owned()
}

#[cfg(test)]
mod tests {
    use super::{language, language_key};

    #[test]
    fn maps_extensions_and_known_names() {
        assert_eq!(language("src/a/main.RS"), "Rust");
        assert_eq!(language("a/Makefile"), "Make");
        assert_eq!(language(".gitignore"), "Config");
        assert_eq!(language("x/.prettierrc"), "Config");
        assert_eq!(language("README"), "(no extension)");
        assert_eq!(language("a/b.xyz"), ".xyz");
        assert_eq!(language("types.d.ts"), "TypeScript");
        assert_eq!(language("a.tar.gz"), ".gz");
    }

    /// Paths with one key have one language, whatever comes before the key.
    #[test]
    fn paths_with_one_key_have_one_language() {
        let paths = [
            "src/a/main.RS",
            "x/main.rs",
            "lib.rs",
            "a/Makefile",
            "Makefile",
            "b/Makefile.am",
            ".gitignore",
            "x/.gitignore",
            "x/.prettierrc",
            "README",
            "docs/README",
            "a/b.xyz",
            "c.xyz",
            "types.d.ts",
            "x.ts",
            "a.tar.gz",
            "b.gz",
            "dir.x/README",
            "a.",
            "b.",
            "x/é.rs",
            "..rs",
            ".rs",
            "rs",
        ];
        for p in paths {
            for q in paths {
                if language_key(p) == language_key(q) {
                    assert_eq!(language(p), language(q), "{p} and {q}");
                }
            }
        }
        assert_eq!(language_key("x/y/main.rs"), language_key("lib.rs"));
    }
}
