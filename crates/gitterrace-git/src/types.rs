/// Author or committer identity, mailmap applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    pub name: String,
    pub email: String,
    /// Seconds since the Unix epoch.
    pub time: i64,
    /// Offset from UTC in seconds.
    pub offset: i32,
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
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
}

#[derive(Debug, Clone)]
pub struct Hunk {
    pub header: String,
    pub lines: Vec<HunkLine>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Added,
    Removed,
}

#[derive(Debug, Clone)]
pub struct HunkLine {
    pub kind: LineKind,
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeEntryKind {
    Dir,
    File,
    Symlink,
    Submodule,
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    Branch,
    Tag,
    Remote,
}

#[derive(Debug, Clone)]
pub struct RefInfo {
    pub name: String,
    pub kind: RefKind,
    pub target: String,
    pub time: i64,
}

#[derive(Debug, Clone)]
pub struct BlameLine {
    pub line_no: u32,
    pub commit: String,
    pub author: String,
    pub time: i64,
    pub text: String,
}
