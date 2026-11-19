//! Comparing two revisions: the files that differ, with hunks.

use crate::error::git;
use crate::repo::Repo;
use crate::{Error, Result};

impl Repo {
    /// The files that differ between two revisions, with hunks. Two-dot compares the trees of
    /// `base` and `head`; `three_dot` compares from their merge base, so only what `head` did
    /// since the branches split shows. The second value is the id the head side was compared
    /// from. A pair with no common ancestor has no merge base and is an error in three-dot form.
    pub fn compare_files(
        &self,
        base: &str,
        head: &str,
        three_dot: bool,
    ) -> Result<(String, Vec<crate::FileDiff>)> {
        let repo = self.repo();
        let (b, h) = (self.resolve_id(&repo, base)?, self.resolve_id(&repo, head)?);
        let from = if three_dot {
            let none = || Error::RevNotFound(format!("{base}...{head}"));
            repo.merge_base(b, h)
                .map_err(|_| none())?
                .ok_or_else(none)?
                .detach()
        } else {
            b
        };
        let tree = |id: gix::ObjectId| -> Result<gix::Tree<'_>> {
            repo.find_commit(id).map_err(git)?.tree().map_err(git)
        };
        let (old, new) = (tree(from)?, tree(h)?);
        let mut cache = crate::diff::resource_cache(&repo)?;
        let raw = crate::diff::tree_changes(
            &repo,
            Some(&old),
            &new,
            &mut cache,
            gix::diff::Rewrites::default(),
        )?;
        let files = crate::diff::files_of(&repo, &raw, self.text_limit())?;
        Ok((from.to_string(), files))
    }
}
