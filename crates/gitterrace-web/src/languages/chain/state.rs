//! What a build of the chain keeps for the next one: the running totals at the tip and a
//! checkpoint every `gap` commits, saved next to the segments (see `gitterrace_index::StateFile`).
//!
//! The file is only ever a head start. A reader that finds it damaged, of another version, or
//! for other rules gets no state and builds from nothing, which gives the same skeleton.

/// The tally after one commit, at a checkpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Check {
    pub(crate) id: String,
    pub(crate) net: Vec<i64>,
    pub(crate) added: u64,
    pub(crate) removed: u64,
}

/// The chain's tally at `tip`, `n` commits from the root, and its checkpoints.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct State {
    pub(crate) n: usize,
    pub(crate) gap: usize,
    pub(crate) tip: String,
    pub(crate) names: Vec<String>,
    pub(crate) net: Vec<i64>,
    pub(crate) added: u64,
    pub(crate) removed: u64,
    /// At positions `gap - 1`, `2 * gap - 1` and so on.
    pub(crate) checks: Vec<Check>,
}

/// Bump when the tally or its meaning changes. Part of the key too, so a new build of the
/// program never reads another's totals.
const PAYLOAD_VERSION: u32 = 1;
/// Longest list or name read back: a file a hostile repository planted cannot make the
/// reader reserve more than the file holds. Not configurable: a format bound.
const MAX_ITEMS: usize = 1 << 20;

/// Whatever the tally depends on besides the commits: the program, the path rules and the
/// ignore files they apply. A change of any of them makes every saved total wrong.
pub(crate) fn key(rules: &str, ignores: &[(String, String)]) -> u64 {
    let mut b = Vec::new();
    b.extend_from_slice(env!("CARGO_PKG_VERSION").as_bytes());
    b.extend_from_slice(&PAYLOAD_VERSION.to_le_bytes());
    b.extend_from_slice(rules.as_bytes());
    for (path, text) in ignores {
        for part in [path, text] {
            b.extend_from_slice(&(part.len() as u64).to_le_bytes());
            b.extend_from_slice(part.as_bytes());
        }
    }
    gitterrace_git::fnv1a64(b)
}

impl State {
    pub(super) const fn empty() -> Self {
        Self {
            n: 0,
            gap: crate::languages::chain::MIN_GAP,
            tip: String::new(),
            names: Vec::new(),
            net: Vec::new(),
            added: 0,
            removed: 0,
            checks: Vec::new(),
        }
    }

    pub(crate) fn encode(&self) -> Vec<u8> {
        let mut b = Vec::new();
        put(&mut b, self.n as u64);
        put(&mut b, self.gap as u64);
        text(&mut b, &self.tip);
        put(&mut b, self.names.len() as u64);
        for n in &self.names {
            text(&mut b, n);
        }
        nets(&mut b, &self.net);
        put(&mut b, self.added);
        put(&mut b, self.removed);
        put(&mut b, self.checks.len() as u64);
        for c in &self.checks {
            text(&mut b, &c.id);
            nets(&mut b, &c.net);
            put(&mut b, c.added);
            put(&mut b, c.removed);
        }
        b
    }

    /// The state `bytes` hold, `None` for anything that is not exactly one.
    pub(crate) fn decode(bytes: &[u8]) -> Option<Self> {
        let mut r = Reader(bytes);
        let n = r.count()?;
        let gap = r.count()?;
        let tip = r.text()?;
        let names = (0..r.count()?)
            .map(|_| r.text())
            .collect::<Option<Vec<_>>>()?;
        let net = r.nets()?;
        let (added, removed) = (r.u64()?, r.u64()?);
        let checks = (0..r.count()?)
            .map(|_| {
                Some(Check {
                    id: r.text()?,
                    net: r.nets()?,
                    added: r.u64()?,
                    removed: r.u64()?,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        // A state that does not add up is damaged: a gap of 0 would divide by zero later, and
        // the checkpoints must be exactly the ones a chain of `n` has.
        let sound = r.0.is_empty()
            && gap > 0
            && net.len() == names.len()
            && checks.len() == n / gap
            && checks.iter().all(|c| c.net.len() <= names.len());
        sound.then_some(Self {
            n,
            gap,
            tip,
            names,
            net,
            added,
            removed,
            checks,
        })
    }
}

fn put(b: &mut Vec<u8>, v: u64) {
    b.extend_from_slice(&v.to_le_bytes());
}

fn text(b: &mut Vec<u8>, s: &str) {
    put(b, s.len() as u64);
    b.extend_from_slice(s.as_bytes());
}

fn nets(b: &mut Vec<u8>, v: &[i64]) {
    put(b, v.len() as u64);
    for x in v {
        b.extend_from_slice(&x.to_le_bytes());
    }
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        let (head, rest) = self.0.split_at_checked(n)?;
        self.0 = rest;
        Some(head)
    }

    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }

    /// A length or count, bounded by what the rest of the input could hold.
    fn count(&mut self) -> Option<usize> {
        let n = usize::try_from(self.u64()?).ok()?;
        (n <= MAX_ITEMS.max(self.0.len())).then_some(n)
    }

    fn text(&mut self) -> Option<String> {
        let n = self.count()?;
        String::from_utf8(self.take(n)?.to_vec()).ok()
    }

    fn nets(&mut self) -> Option<Vec<i64>> {
        let n = self.count()?;
        if n.checked_mul(8)? > self.0.len() {
            return None;
        }
        (0..n)
            .map(|_| Some(i64::from_le_bytes(self.take(8)?.try_into().ok()?)))
            .collect()
    }
}
