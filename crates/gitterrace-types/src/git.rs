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

string_enum! {
    /// What became of the code a finding points at. A status says what happened to the code,
    /// never whether the finding was dealt with, so no word here claims that.
    pub enum AnchorStatus [serde] {
        Untouched => "untouched",
        LinesChanged => "lines changed",
        FileChanged => "file changed",
        FileGone => "file gone",
    }
}

string_enum! {
    /// What an anchor was checked against.
    pub enum AnchorBasis [serde] {
        Head => "head",
        Worktree => "worktree",
    }
}

#[cfg(test)]
mod tests {
    use crate::codec::assert_round_trip;
    use crate::{AnchorBasis, AnchorStatus, ChangeKind, LineKind, RefKind, TreeEntryKind};

    /// Class guard: a status never claims a finding was dealt with. A word like that would turn
    /// a fact about the code into a claim about the review.
    #[test]
    fn no_anchor_status_claims_the_finding_was_dealt_with() {
        use crate::StringEnum;
        for s in AnchorStatus::ALL {
            let t = s.as_str();
            for word in ["resolved", "fixed", "done", "stale", "open", "closed"] {
                assert!(!t.contains(word), "{t:?} says {word}");
            }
        }
    }

    #[test]
    fn git_kinds_round_trip() {
        assert_round_trip::<RefKind>();
        assert_round_trip::<ChangeKind>();
        assert_round_trip::<LineKind>();
        assert_round_trip::<TreeEntryKind>();
        assert_round_trip::<AnchorStatus>();
        assert_round_trip::<AnchorBasis>();
    }
}
