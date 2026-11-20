use gix::bstr::ByteSlice;

use crate::Signature;

/// The `.mailmap` blob of a commit, never the worktree one. One over
/// [`crate::ignore::MAX_CONFIG_BYTES`] is ignored: it is parsed on every request.
pub(crate) fn mailmap_blob(repo: &gix::Repository, commit: gix::ObjectId) -> Option<gix::ObjectId> {
    repo.find_commit(commit)
        .ok()
        .and_then(|c| c.tree().ok())
        .and_then(|t| t.find_entry(".mailmap").map(|e| e.object_id()))
        .filter(|id| {
            repo.find_header(*id).is_ok_and(|h| h.size() <= crate::ignore::MAX_CONFIG_BYTES)
        })
}

pub(crate) fn parse_mailmap(
    repo: &gix::Repository,
    blob: Option<gix::ObjectId>,
) -> gix::mailmap::Snapshot {
    blob.and_then(|id| repo.find_object(id).ok())
        .map_or_else(gix::mailmap::Snapshot::default, |b| {
            gix::mailmap::Snapshot::from_bytes(&b.data)
        })
}

pub(crate) fn mailmap(repo: &gix::Repository, commit: gix::ObjectId) -> gix::mailmap::Snapshot {
    parse_mailmap(repo, mailmap_blob(repo, commit))
}

pub(crate) fn signature(
    map: &gix::mailmap::Snapshot,
    sig: gix::actor::SignatureRef<'_>,
) -> Signature {
    let time = sig.time().unwrap_or_default();
    let resolved = map.resolve(sig);
    Signature {
        name: resolved.name.to_str_lossy().into_owned(),
        email: resolved.email.to_str_lossy().into_owned(),
        time: time.seconds,
        offset: time.offset,
    }
}
