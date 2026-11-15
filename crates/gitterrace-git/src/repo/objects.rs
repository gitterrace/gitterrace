//! Trees and blobs at a revision.

use gix::bstr::ByteSlice;

use crate::error::git;
use crate::repo::Repo;
use crate::{Blob, Error, RawBlob, Result, TreeEntry, TreeEntryKind};

/// Enough of a binary blob to name its format: the longest magic checked sits in the first
/// 512 bytes (tar's `ustar` at 257).
const HEAD_BYTES: usize = 512;

/// The bytes `Blob::head` keeps: a prefix of binary data, nothing of text.
fn head_of(data: &[u8], text: bool) -> Vec<u8> {
    if text {
        Vec::new()
    } else {
        data[..data.len().min(HEAD_BYTES)].to_vec()
    }
}

impl Repo {
    /// Entries of the directory at `path` in `rev`, dirs first then by name.
    pub fn tree(&self, rev: &str, path: &str) -> Result<Vec<TreeEntry>> {
        let repo = self.repo();
        let tip = self.resolve_id(&repo, rev)?;
        let root = repo.find_commit(tip).map_err(git)?.tree().map_err(git)?;
        let path = path.trim_matches('/');
        let tree = if path.is_empty() {
            root
        } else {
            let entry = crate::treepath::lookup(&root, path)?
                .ok_or_else(|| Error::PathNotFound(path.to_string()))?;
            if !entry.mode().is_tree() {
                return Err(Error::PathNotFound(path.to_string()));
            }
            entry.object().map_err(git)?.into_tree()
        };
        let mut out = Vec::new();
        for entry in tree.iter() {
            let entry = entry.map_err(git)?;
            let mode = entry.mode();
            let kind = if mode.is_tree() {
                TreeEntryKind::Dir
            } else if mode.is_commit() {
                TreeEntryKind::Submodule
            } else if mode.is_link() {
                TreeEntryKind::Symlink
            } else {
                TreeEntryKind::File
            };
            let size = match kind {
                TreeEntryKind::File | TreeEntryKind::Symlink => {
                    repo.find_header(entry.oid()).ok().map(|h| h.size())
                }
                _ => None,
            };
            out.push(TreeEntry {
                name: entry.filename().to_str_lossy().into_owned(),
                kind,
                size,
            });
        }
        out.sort_by(|a, b| {
            let dir = |e: &TreeEntry| e.kind != TreeEntryKind::Dir;
            dir(a).cmp(&dir(b)).then_with(|| a.name.cmp(&b.name))
        });
        Ok(out)
    }

    /// One resolve and one tree lookup for `/raw`: the blob id and size, plus the bytes unless
    /// `skip(id_hex, size)` says they are not needed (too large, or the client has them).
    pub fn raw_blob(
        &self,
        rev: &str,
        path: &str,
        skip: impl FnOnce(&str, u64) -> bool,
    ) -> Result<RawBlob> {
        let repo = self.repo();
        let tip = self.resolve_id(&repo, rev)?;
        let id = Self::blob_id_at(&repo, tip, path.trim_matches('/'))?;
        let size = repo.find_header(id).map_err(git)?.size();
        let id_hex = id.to_string();
        let bytes = if skip(&id_hex, size) {
            None
        } else {
            Some(repo.find_object(id).map_err(git)?.detach().data)
        };
        Ok(RawBlob {
            id: id_hex,
            size,
            bytes,
        })
    }

    /// Raw bytes of the file at `path` in `rev`.
    pub fn blob_bytes(&self, rev: &str, path: &str) -> Result<Vec<u8>> {
        let repo = self.repo();
        let tip = self.resolve_id(&repo, rev)?;
        Self::blob_bytes_at(&repo, tip, path.trim_matches('/'))
    }

    fn blob_bytes_at(repo: &gix::Repository, tip: gix::ObjectId, path: &str) -> Result<Vec<u8>> {
        let id = Self::blob_id_at(repo, tip, path)?;
        Ok(repo.find_object(id).map_err(git)?.detach().data)
    }

    pub(super) fn blob_id_at(
        repo: &gix::Repository,
        tip: gix::ObjectId,
        path: &str,
    ) -> Result<gix::ObjectId> {
        let tree = repo.find_commit(tip).map_err(git)?.tree().map_err(git)?;
        Ok(crate::treepath::lookup(&tree, path)?
            .filter(|e| !e.mode().is_tree() && !e.mode().is_commit())
            .ok_or_else(|| Error::PathNotFound(path.to_string()))?
            .object_id())
    }

    /// [`Repo::blob`] and, when the blob is at most `max_bytes`, its raw bytes (else empty),
    /// from one content read: a media page wants both.
    pub fn blob_with_bytes(
        &self,
        rev: &str,
        path: &str,
        max_bytes: u64,
    ) -> Result<(Blob, Vec<u8>)> {
        let repo = self.repo();
        let tip = self.resolve_id(&repo, rev)?;
        let path = path.trim_matches('/');
        let id = Self::blob_id_at(&repo, tip, path)?;
        let size = repo.find_header(id).map_err(git)?.size();
        let limit = self.text_limit();
        let data = if size <= limit.max(max_bytes) {
            repo.find_object(id).map_err(git)?.detach().data
        } else {
            Vec::new()
        };
        let text = (size <= limit && !crate::diff::is_binary(&data))
            .then(|| std::str::from_utf8(&data).ok().map(str::to_owned))
            .flatten();
        let bytes = if size <= max_bytes { data } else { Vec::new() };
        let blob = Blob {
            path: path.to_string(),
            size,
            head: head_of(&bytes, text.is_some()),
            text,
        };
        Ok((blob, bytes))
    }

    /// The file at `path` in `rev`. `text` is `None` for binary blobs and for blobs over
    /// [`MAX_TEXT_BYTES`](crate::MAX_TEXT_BYTES), which are never read.
    pub fn blob(&self, rev: &str, path: &str) -> Result<Blob> {
        let repo = self.repo();
        let tip = self.resolve_id(&repo, rev)?;
        let path = path.trim_matches('/');
        let id = Self::blob_id_at(&repo, tip, path)?;
        let size = repo.find_header(id).map_err(git)?.size();
        let (text, head) = if size > self.text_limit() {
            (None, Vec::new())
        } else {
            let data = repo.find_object(id).map_err(git)?.detach().data;
            if crate::diff::is_binary(&data) {
                let head = head_of(&data, false);
                (None, head)
            } else {
                match String::from_utf8(data) {
                    Ok(t) => (Some(t), Vec::new()),
                    Err(e) => (None, head_of(e.as_bytes(), false)),
                }
            }
        };
        Ok(Blob {
            path: path.to_string(),
            size,
            text,
            head,
        })
    }
}
