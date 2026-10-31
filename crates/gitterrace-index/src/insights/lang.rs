//! File name to language, the table repoview uses.

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
    use super::language;

    #[test]
    fn matches_repoview() {
        assert_eq!(language("src/a/main.RS"), "Rust");
        assert_eq!(language("a/Makefile"), "Make");
        assert_eq!(language(".gitignore"), "Config");
        assert_eq!(language("x/.prettierrc"), "Config");
        assert_eq!(language("README"), "(no extension)");
        assert_eq!(language("a/b.xyz"), ".xyz");
        assert_eq!(language("types.d.ts"), "TypeScript");
        assert_eq!(language("a.tar.gz"), ".gz");
    }
}
