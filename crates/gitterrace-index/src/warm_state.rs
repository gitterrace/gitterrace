//! State files of the warm-up, kept next to the segments so a later build extends what an
//! earlier one computed instead of starting over.
//!
//! Every file goes through [`crate::cache`], so a link planted at its name is removed and never
//! followed, and a write is a temp file in the same folder and one rename: a crash or a full
//! disk leaves the old file whole or none, never half of a new one. A reader checks the
//! magic, version, the key its caller derives from everything the state depends on, and a
//! checksum, and takes anything else as no state: the caller then builds from scratch, which
//! is always correct.

use std::io;
use std::path::Path;

use crate::cache;

/// The languages chain's state, see `languages/chain/state.rs` in the web crate.
pub(crate) const CHAIN_STATE_FILE: &str = "chain.state";

const MAGIC: &[u8; 4] = b"GTWS";
const VERSION: u32 = 1;
const HEADER: usize = 4 + 4 + 8 + 8;
const TRAILER: usize = 8;

/// Which state file a warmer keeps. A closed list: a name outside it is not in
/// [`crate::CACHE_FILES`] and the cache would sweep it as foreign.
#[derive(Clone, Copy, Debug)]
pub enum StateFile {
    Chain,
}

impl StateFile {
    const fn name(self) -> &'static str {
        match self {
            Self::Chain => CHAIN_STATE_FILE,
        }
    }
}

/// The payload saved under `key`, or `None` when there is no file, it is damaged, or it was
/// saved under another key.
pub(crate) fn load(dir: &Path, file: StateFile, key: u64) -> Option<Vec<u8>> {
    let bytes = cache::read(dir, file.name()).ok().flatten()?;
    decode(&bytes, key)
}

pub(crate) fn save(dir: &Path, file: StateFile, key: u64, payload: &[u8]) -> io::Result<()> {
    cache::write_atomic(dir, file.name(), &encode(key, payload))
}

fn encode(key: u64, payload: &[u8]) -> Vec<u8> {
    let mut b = Vec::with_capacity(HEADER + payload.len() + TRAILER);
    b.extend_from_slice(MAGIC);
    b.extend_from_slice(&VERSION.to_le_bytes());
    b.extend_from_slice(&key.to_le_bytes());
    b.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    b.extend_from_slice(payload);
    b.extend_from_slice(&gitterrace_git::fnv1a64(payload.iter().copied()).to_le_bytes());
    b
}

fn decode(bytes: &[u8], key: u64) -> Option<Vec<u8>> {
    let head = bytes.get(..HEADER)?;
    if head.get(..4)? != MAGIC || u32::from_le_bytes(head.get(4..8)?.try_into().ok()?) != VERSION {
        return None;
    }
    if u64::from_le_bytes(head.get(8..16)?.try_into().ok()?) != key {
        return None;
    }
    let len = usize::try_from(u64::from_le_bytes(head.get(16..24)?.try_into().ok()?)).ok()?;
    let payload = bytes.get(HEADER..HEADER.checked_add(len)?)?;
    let sum = bytes.get(HEADER + len..HEADER + len + TRAILER)?;
    if bytes.len() != HEADER + len + TRAILER
        || u64::from_le_bytes(sum.try_into().ok()?)
            != gitterrace_git::fnv1a64(payload.iter().copied())
    {
        return None;
    }
    Some(payload.to_vec())
}

#[cfg(test)]
#[path = "warm_state_tests.rs"]
mod tests;
