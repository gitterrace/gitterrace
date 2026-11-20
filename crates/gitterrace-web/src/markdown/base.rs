//! Where a markdown file lives and what its render may cost.

/// Where a markdown file lives, so relative links can be resolved.
pub(crate) struct Base {
    pub(crate) rev: String,
    /// Directory of the markdown file inside the repo, no slashes at the ends.
    pub(crate) dir: String,
    /// Largest code block highlighted, in bytes (`limits.highlight_kb`).
    pub(crate) highlight_bytes: usize,
}

impl Base {
    /// `rev` is the revision exactly as typed in the URL, `dir` the file's directory.
    pub(crate) fn new(rev: &str, dir: &str) -> Self {
        Self {
            rev: crate::fmt::enc_seg(rev),
            dir: dir.trim_matches('/').to_owned(),
            highlight_bytes: crate::highlight::HIGHLIGHT_BYTES,
        }
    }

    /// The highlight limit in force, from the request's settings.
    #[must_use]
    pub(crate) const fn highlight(mut self, bytes: usize) -> Self {
        self.highlight_bytes = bytes;
        self
    }
}
