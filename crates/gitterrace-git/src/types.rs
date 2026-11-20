pub use gitterrace_types::api::Signature;
pub use gitterrace_types::{ChangeKind, LineKind, RefKind, TreeEntryKind};

/// One commit as shown in a log list.
#[derive(Debug, Clone)]
pub struct CommitSummary {
    pub id: String,
    pub parents: Vec<String>,
    pub author: Signature,
    pub committer: Signature,
    pub subject: String,
}

/// A commit with its full message and diff against the first parent.
#[derive(Debug, Clone)]
pub struct CommitDetail {
    pub summary: CommitSummary,
    pub body: String,
    pub files: Vec<FileDiff>,
}

/// Changes to one file. `hunks` is empty for binary files.
#[derive(Debug, Clone)]
pub struct FileDiff {
    pub path: String,
    pub kind: ChangeKind,
    pub old_path: Option<String>,
    pub additions: u32,
    pub deletions: u32,
    pub binary: bool,
    pub hunks: Vec<Hunk>,
    /// Line count of the file after the change (0 when binary or deleted).
    pub lines: u32,
    /// Lines in change blocks that both removed and added (the smaller side), a measure of rework.
    pub rewritten: u32,
    /// The same counts with whitespace-only and blank lines ignored.
    pub code: CodeCounts,
    /// Old and new file mode (octal value, e.g. `0o100644`) when the change altered the mode.
    pub modes: Option<(u16, u16)>,
}

/// Additions, deletions and rewritten lines once whitespace differences are ignored.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CodeCounts {
    pub additions: u32,
    pub deletions: u32,
    pub rewritten: u32,
}

#[derive(Debug, Clone)]
pub struct Hunk {
    pub header: String,
    pub lines: Vec<HunkLine>,
}

#[derive(Debug, Clone)]
pub struct HunkLine {
    pub kind: LineKind,
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct TreeEntry {
    pub name: String,
    pub kind: TreeEntryKind,
    pub size: Option<u64>,
}

/// File content at a revision. `text` is `None` for binary content.
#[derive(Debug, Clone)]
pub struct Blob {
    pub path: String,
    pub size: u64,
    pub text: Option<String>,
    /// The first bytes of a binary blob read for its text, so a view can name its format
    /// from the magic number. Empty for text and for a blob too large to read.
    pub head: Vec<u8>,
}

/// A blob for `/raw`: `bytes` is `None` when the caller skipped reading it.
#[derive(Debug, Clone)]
pub struct RawBlob {
    pub id: String,
    pub size: u64,
    pub bytes: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct RefInfo {
    pub name: String,
    pub kind: RefKind,
    pub target: String,
    pub time: i64,
    pub annotation: Option<crate::TagAnnotation>,
}

#[derive(Debug, Clone)]
pub struct BlameLine {
    pub line_no: u32,
    pub commit: String,
    pub author: String,
    pub time: i64,
    pub text: String,
}
