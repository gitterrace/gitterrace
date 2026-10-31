use gix::bstr::ByteSlice;

use crate::Signature;

/// Mailmap read from the `.mailmap` blob of a commit, never from the worktree.
pub(crate) fn mailmap(repo: &gix::Repository, commit: gix::ObjectId) -> gix::mailmap::Snapshot {
    let blob = repo
        .find_commit(commit)
        .ok()
        .and_then(|c| c.tree().ok())
        .and_then(|t| t.find_entry(".mailmap").map(|e| e.object_id()))
        .and_then(|id| repo.find_object(id).ok());
    match blob {
        Some(b) => gix::mailmap::Snapshot::from_bytes(&b.data),
        None => gix::mailmap::Snapshot::default(),
    }
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
