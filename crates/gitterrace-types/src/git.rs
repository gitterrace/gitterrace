//! The kinds a git read reports, with the text the API and pages write for them.

use crate::string_enum;

string_enum! {
    /// What a reference is.
    pub enum RefKind [serde] {
        Branch => "branch",
        Tag => "tag",
        Remote => "remote",
    }
}

string_enum! {
    /// How a commit changed one file.
    pub enum ChangeKind [serde] {
        Added => "added",
        Modified => "modified",
        Deleted => "deleted",
        Renamed => "renamed",
    }
}

string_enum! {
    /// One line of a diff hunk.
    pub enum LineKind [serde] {
        Context => "context",
        Added => "added",
        Removed => "removed",
    }
}

string_enum! {
    /// What a tree entry holds.
    pub enum TreeEntryKind [serde] {
        Dir => "dir",
        File => "file",
        Symlink => "symlink",
        Submodule => "submodule",
    }
}

#[cfg(test)]
mod tests {
    use crate::codec::assert_round_trip;
    use crate::{ChangeKind, LineKind, RefKind, TreeEntryKind};

    #[test]
    fn git_kinds_round_trip() {
        assert_round_trip::<RefKind>();
        assert_round_trip::<ChangeKind>();
        assert_round_trip::<LineKind>();
        assert_round_trip::<TreeEntryKind>();
    }
}
