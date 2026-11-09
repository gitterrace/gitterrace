# The insights store

How the insights cache is laid out on disk and read, so the server stays at some
MB however long the history. It sits beside the code it describes (`seg/`).
[The index internals](internals.md#how-the-index-is-stored) have
the short version.

## Problem

Held in memory as parsed records, the history of rust (345k commits, 4.9 M file
rows) is about 820 MB, plus what pages derive from it (people, commit lines,
rename follow). The goal: resident memory while serving is some MB, even during
a page sweep. A peak during the index build is fine.

Constraints: `unsafe_code = "forbid"` in our crates, so access is `pread`
(`FileExt::read_at`). No compatibility: a cache this binary cannot read is
deleted and rebuilt. The index stays a pure function of the repository
(byte-identical across thread counts).

## 1. Store candidates, measured

Probe: `target/design/storeproto` (not in the workspace). It loads the file rows
(path, commit, add, del, lines, rewritten) and writes them three ways, keyed by
(path, commit). Then a point query on the most-touched path and a full scan
summing additions. Page cache warm, M1 Max, release. RSS columns include the
probe's own 147 MB input vector on rust, so compare the deltas.

| Store                      | Repo | Write  | Size   | Path query | Full scan | RSS delta                      | Crates        | License             |
| -------------------------- | ---- | ------ | ------ | ---------- | --------- | ------------------------------ | ------------- | ------------------- |
| custom blocks + pread      | git  | 0.03 s | 6 MB   | 33 us      | 0.8 ms    | about 13 MB (the write buffer) | 1 (crc32fast) | ours                |
| redb 2                     | git  | 0.92 s | 16 MB  | 435 us     | 40 ms     | 35 MB                          | 2             | MIT/Apache          |
| SQLite (rusqlite, bundled) | git  | 0.53 s | 6 MB   | 239 us     | 101 ms    | 44 MB                          | 12, C build   | MIT + public domain |
| custom blocks + pread      | rust | 0.66 s | 113 MB | 54 us      | 168 ms    | write buffer only              | 1             | ours                |
| redb 2                     | rust | 19.9 s | 257 MB | 2.0 ms     | 890 ms    | +170 MB                        | 2             | MIT/Apache          |
| SQLite                     | rust | 26.5 s | 121 MB | 558 us     | 568 ms    | +10 MB (8 MB page cache)       | 12            |                     |

The custom file stored 24 bytes per row uncompressed. Linux is estimated, not
run: its commits file is 5x rust's, so about 25 M file rows. Custom at 24 B per
row is about 600 MB and under 300 MB with the varint encoding below. redb and
SQLite scale with their rust numbers (about 1.3 GB and 600 MB, and 100 to 130 s
added to the build).

Other criteria:

- Crash safety. redb and SQLite give it with transactions. The custom set gets
  it more cheaply: every file is immutable once written, and one `MANIFEST`
  swapped by rename decides which files are live (section 2), so a crash leaves
  either the old set or the new one, never a mix. Concurrent writers are already
  serialized by the existing `lock` flock.
- mmap via a safe crate. Not needed: pread on a 1 MB block costs microseconds
  from the page cache, and the OS file cache is not our dirty memory. mmap would
  also count pages in RSS, which confuses the measurement.
- Build time. rusqlite bundled compiles C (about 40 s cold here). redb is pure
  Rust and light.

**Pick: the custom immutable block file.** It is 10 to 40x faster to write, 5 to
40x faster to query, the smallest, needs one tiny dependency, and its crash
model (immutable segments plus rename) is simpler than a transactional engine.
We never update in place, so a DB's main strength (random writes) buys nothing.
redb is the alternative if segment merging (section 4) proves fiddly.

## 2. On-disk format

One directory `.git/gitterrace/`, files listed in `CACHE_FILES`. Every file
except `MANIFEST`, `VERSION` and `lock` is immutable: written under a temp name,
fsynced, never opened for writing again.

- `VERSION`: `v` and `FORMAT_VERSION` in `store.rs`, the version of the whole
  cache. A bare number is a cache from before the first release, reset like any
  older one; only a `v` number greater than ours is a newer build's. The
  container version `VERSION` in `seg/format.rs` moves only when the container
  bytes change, so the two numbers differ. Read first so a newer build's cache
  is recognized without parsing anything else.
- `MANIFEST`: the only entry point. It names every live file with its length and
  CRC32, the generation, the canonical-order fingerprint (section 4), and a
  `retired` list. Written to `MANIFEST.tmp`, fsynced, renamed, then the
  directory is fsynced. A reader opens only files the manifest names and checks
  length and CRC of the footer on open. A file on disk that the manifest does
  not name is ignored and swept. A crash anywhere before the rename leaves the
  previous manifest, which names the previous complete set.
- `seg-<gen>-<n>.gts`: immutable segments, each covering a run of the canonical
  order. A fresh build writes one. An update adds one. Merged back to one when
  there are more than 8.
- `str-<gen>-<n>.gts`: string deltas, immutable, one per segment: the paths,
  identities (author, email as written) and kind/scope names first introduced by
  that segment. Ids are the concatenation in manifest order. Nothing is ever
  appended in place. A merge rewrites the strings whole alongside the merged
  segment.
- `lines.json`: head file line counts, written whole under a temp name and named
  in the manifest.

Windows: deleting a file another reader has open fails there. Superseded files
go to the manifest's `retired` list and are removed at the next open or merge
when the delete succeeds, retried otherwise. A missing retired file is fine.
Reads go through one shim `read_exact_at` in `seg/io.rs`:
`std::os::unix::fs::FileExt::read_exact_at` on Unix, a loop over
`std::os::windows::fs::FileExt::seek_read` on Windows (which moves the file
cursor, so each reader holds its own `File` handle). CI runs the segment tests
on Windows.

A segment is a sequence of blocks followed by a footer:

1. **Commit table, columnar.** Per commit: oid (20 B), author id (u32), time
   (i64 delta-varint), offset (i16), flags (merge, shallow), kind and scope ids,
   parent count plus parent ordinals (index into the walk, not oids), and per
   option-independent line sums (add, del, rewritten, both raw and code). About
   48 B per commit: git 4 MB, rust 17 MB, linux 72 MB.
2. **File rows, grouped by commit.** (path id, add, del, lines, rewritten, code
   triple when it differs, old path id, binary), varint. About 12 B per row: git
   3 MB, rust 60 MB, linux about 300 MB. Used by coupling, compare, arbitrary
   selections.
3. **Path postings.** Per path id, the commit ordinals and row offsets that
   touch it, delta-varint (about 3 B per entry), plus the rename edges (old to
   new, ordinal). Path logs and `file_history` become O(touches of that path).
4. **Author postings.** Per identity id, commit ordinals. Author chips become
   O(that author).
5. **Aggregates** (the part that makes pages O(result)). Keyed by raw identity
   id and raw path id, never by option, so options apply on read:
   - author x day: commits, add, del, rewritten (raw and code). git 26k, rust
     124k rows, about 24 B: rust 3 MB, linux about 10 MB.
   - month x path: touches, add, del, rewritten, bot share split (bot touches
     kept apart). git 68k, rust 715k rows, 20 B: rust 14 MB, linux about 50 MB.
   - path x author: touches, lines (ownership). git 50k, rust 680k, 16 B: rust
     11 MB.
   - month x language: derivable from month x path at read, not stored.
6. **Footer** (fixed 64 B tail plus a table of contents): magic `GTS1`, format
   version, segment ordinal range, first and last oid, per section offset,
   length and CRC32, per block CRC32, and a CRC over the footer itself. A reader
   validates the footer and the CRC of every block it reads. Any mismatch is the
   "cannot read" case of section 5.

Hostile bytes: the cache lives in `.git/` of a repository someone else may have
prepared, so the reader trusts no field. Every length, count, offset and varint
is checked against the file length and a fixed maximum before use (varints at
most 10 bytes, a block at most 1 MiB, a section table at most 4096 entries, a
string at most 64 KiB). No buffer is sized by a field before that field's block
CRC passed and the bound held. Arithmetic on offsets is checked. Any violation
is `Unreadable`.

### 2.1 Byte layout

All integers little endian. CRC is CRC32 (IEEE, `crc32fast`). One container
serves every kind of immutable file (segment, string delta, order list).

Container file:

| Part   | Bytes                      | Fields                                                                              |
| ------ | -------------------------- | ----------------------------------------------------------------------------------- |
| Header | 16                         | magic `GTS1`, version u32, kind u32 (1 segment, 2 strings, 3 order, 4 messages, 5 chain), zero u32 |
| Blocks | each at most 1 MiB         | payload only, back to back from offset 16                                           |
| TOC    | 24 per entry, at most 4096 | section u16, zero u16, items u32, offset u64, len u32, crc u32 (of the payload)     |
| Footer | 64, the last bytes         | see below                                                                           |

Footer (64 B): magic `GTSF` (4), version u32, kind u32, zero u32, toc offset
u64, toc count u32, toc crc u32, item total u32, zero u32, file length u64,
string base (paths u32, identities u32, labels u32: the cumulative string counts
this file was written against, zero for kind 3), footer crc u32 over the first
60 bytes.

Blocks are back to back, not aligned: an update segment of a few commits would
otherwise cost a whole aligned block per section, and pread does not need
alignment.

Open checks, in order, each a bounded read: file length at least 80 (header plus
footer), footer crc, magic, version, kind, file length equal to the footer's,
toc count at most 4096, toc range inside `[16, len - 64]`, toc crc, then per
entry: len at most 1 MiB, `offset + len` checked and inside `[16, toc offset]`,
entries sorted by (section, offset) and not overlapping. A block's crc is
checked when the block is read, before any field in it is parsed.

Section ids are fixed and never reused. A dropped section keeps its slot, a
reader skips ids it does not use: 1 commits, 2 file rows, 3 path postings, 4
rename edges, 5 author postings, 6 author x day, 7 month x path (reserved,
hotspots are served from postings), 8 path x author, 9 month x language
(reserved until its source is chosen), 16 paths, 17 identities, 18 labels, 32
order, 33 rank (per segment, u32 per local ordinal: its position in the order
file, read by pread at index, never held as a map), 34 rows, 35 row index, 36
oid index. A new section is a new id, a changed encoding is a `VERSION` bump.

Section 1 (commit columns): one fixed 80 B item per local
ordinal, in ordinal order, so a pass over commits reads it alone and never the
file rows of 34. Item: author id u32, time i64 and offset i32 exactly as in 34
(raw, clamped only at read), flags u8 (bit 0 merge, other bits zero), zero u8,
zero u16, kind + 1 u32 and scope + 1 u32 (0 none), file count u32, then six u64
sums (add, del, rewritten, and the same without whitespace), never clamped.
Blocks hold whole items (`items * 80 == len`). On open the item count of 1 must
equal those of 33, 35 and 36, else `Unreadable`. Every read that sees both 1 and
34 for a commit compares them, and a disagreement is `Unreadable`. A merge
rewrites 1 in the new canonical order like every other section.

Strings (kind 2), one per segment: sections paths, identities, labels. Each
block holds items encoded as u16 length plus UTF-8 bytes (a string at most 65535
B). Ids are positions in the concatenation of the deltas of segments 0..=n in
manifest order, so a string id is stable once written and means the same in
every segment that can see it. Commit ordinals stay segment-local.

Three rules on strings and ranks. A string delta is never retired on its own:
rewritten history retires a segment but keeps its delta live, and only a full
merge compacts and renumbers strings by canonical first use. On open, each
segment's and delta's string base must equal the cumulative count of the deltas
before it in manifest order, else `Unreadable`. No oid to rank map is held in
memory (section 33 above). A writer given a string over 65535 B returns an
error, never truncates.

Order (kind 3, `ord-<gen>.gts`): the reachable oids in canonical order, 20 B
each. It gives the topo rank that the canonical key needs across segments.
Rewritten whole with every manifest.

Messages (kind 4, `msg-<gen>-<n>.gts`, only while `index.has_messages` is
on): what commit search matches, one item per rank, in section 40. Items:
flag u8 (0 text kept, 1 too large and read from git), committer time (zigzag
varint), parent count and parent ranks (varints, a rank past the set is
`Unreadable`), then for flag 0 author name, email and message as
length-prefixed bytes (the message is the walk's lossy UTF-8). Items are packed
to about 64 KiB and each block is one zstd frame of at most 1 MiB inflated. In
manifest order the files cover ranks 0 to the end without a gap: a build
writes one, an update one for its new ranks, a merge one in the new order
(byte for byte a fresh build's). Fewer items than ranks means not built yet
and search walks git. More, or a block that fails its crc or does not
inflate, is `Unreadable`. Search decodes the blocks on up to 8 threads, runs
the regex on `message \n name <email>` (the tip's mailmap applied to each
distinct author once), replays git's newest-first commit-date walk over the
stored times and parents, and reads from git only the commits it lists.

Chain (kind 5, `chain-<gen>.gts`): the first-parent chain of
the newest rank, newest first, in section 48. A step is rank u32, oid (20 B),
raw author time i64, offset i32, then a varint row count and per file row the
path id, added and removed lines as varints: the numbers the languages tally
reads, keyed by path so the language and the path filter stay decided at read
time. Section 49 is one 16 B meta item, written last: the manifest's order
fingerprint u64, its commit count u32 and the step count u32. A chain whose
fingerprint or count is not the manifest's is stale and not read. Every build,
append and merge writes it again from the stored rows (no git read, one record
decode per chain step) before its manifest goes live, and names it in that
manifest, so no published set lacks its chain. On read the first rank must be
the newest, each next rank lower, every path id below the string count and
every count a u32, else `Unreadable`.
On rust it is about 2% of the store.

Manifest (`MANIFEST`, not a container, read whole, at most 1 MiB):

| Field                                                                                                             | Bytes |
| ----------------------------------------------------------------------------------------------------------------- | ----- |
| magic `GTM1`, version u32, generation u64                                                                         | 16    |
| order fingerprint u64 (FNV-1a 64 over the ordered oid bytes), shallow fingerprint u64, commit count u32, zero u32 | 24    |
| live count u32 (at most 4096), then per file: kind u8, name len u8, name, file len u64, footer crc u32            |       |
| retired count u32 (at most 4096), then per name: len u8, name                                                     |       |
| crc u32 over everything before it                                                                                 | 4     |

Names match `[a-z0-9.-]{1,64}` with no `..`, so a manifest cannot name a path
outside the directory. Live files are listed in canonical order (segments, then
their string deltas in the same order, then order and `lines.json`). The reader
checks each named file's length and the crc of its 64 B footer against the
manifest on open. The shallow fingerprint is the FNV-1a 64 of the sorted
`.git/shallow` oids, 0 when not shallow: a deepened or grafted clone changes
topo ranks, so a different value on open is a full rebuild, as in section 5.

Write sequence: each file to `<name>.tmp`, fsync, rename, then `MANIFEST.tmp`,
fsync, rename to `MANIFEST`, fsync the directory. Then delete retired files,
keeping a name in `retired` while its delete fails (Windows, open by a reader).
Any crash leaves the previous manifest whole.

Canonical order (the rule the aggregates and the store build on): Kahn's
algorithm over the reachable commit graph, edges parent to child, where the
ready set is a min-heap keyed by (committer time, oid). Pop the least, append
it, decrement its children. Commit ordinals are positions in that list. Topo
rank is the position in the order file, so the canonical key is (rank, time,
oid) and two commits of different segments compare by rank alone.

Totals (aggregates plus columns plus postings): git about 12 MB, rust about 120
MB, linux about 550 MB. The same history as JSON records: 44 MB, 496 MB, 2.5 GB.

Determinism: ordinals and ids follow the canonical order of section 4, sections
are written in id order, all maps are sorted before writing, no hash iteration
order reaches the file.

### 2.2 Commit rows

Three more sections in the segment container, so already named in `MANIFEST`
with length and footer crc. Sections 1 and 2 stay reserved for the columnar
tables, 7 and 9 untouched.

- 34 rows: one record per local ordinal, ordinal order, little endian: oid 20 B,
  raw identity id u32 (`name\0email`), time i64 as written, offset i32, flags u8
  (bit 0 merge), kind u32 and scope u32 (0 none, else id + 1), parent count u16
  then 20 B per parent, file count u32, then per file: path u32, old u32 (0
  none, else id + 1), add, del, lines, rewritten u32, flags u8 (bit 0 binary,
  bit 1 code present), code add, del, rewritten u32 when bit 1. A record may
  continue into the next block of the section.
- 35 row index: u64 per local ordinal at `ordinal * 8`, the byte offset of the
  record in the concatenated payload of section 34. Read by pread, never loaded.
  Offsets strictly increase.
- 36 oid index: 24 B per commit (oid, local ordinal u32), sorted by oid, binary
  searched by pread (never held as a map).

Rules for these sections:

- No buffer is sized by a count. File entries are pushed one at a time as they
  parse, so a record claiming 2^24 files with a valid crc fails at the end of
  its bytes, never allocates for it.
- A record's start block is found by binary search over the block offsets. Only
  the following blocks it needs are read (each crc checked), and one record is
  at most 64 MiB, past that `Unreadable`.
- Time is stored raw and `plausible_time` is applied at read, as the oracle
  does.
- The writer spools rows to `rows-<gen>.gts.tmp` in the cache directory
  (sections must ascend, rows come after 33). The spool is never named in the
  manifest and is swept after a crash.
- The merge rewrites 34 to 36, and two-step-then-merge byte equality covers
  them.

Identity, one rule: segments key identities by the raw `name\0email` as written.
The mailmap of the shown revision is applied at read through a per-identity
vector (raw id to canonical name and email), built once per tip, so a `.mailmap`
edit never rebuilds. A NUL inside a name or email is rejected on write and
`Unreadable` on read. The email match stays case-insensitive.

### 2.3 Views over a commit that is not the indexed tip

The index holds the reachable set of HEAD. A view over another rev R (a branch,
a tag, a selected commit) is served in two parts, never by a second index or a
resident record set:

1. Indexed part: `rev_list(R)` is mapped to ranks through the oid index (36),
   giving a rank bit set (one bit per indexed commit, rust 43 KB). The source
   for R is a filter over the HEAD source: commits, rows and postings are the
   HEAD ones restricted to the set. Aggregates are not precomputed for R: they
   fold from the commit rows of the set, streamed in rank order, memoized per
   (R, page) inside the budget. When R is an ancestor of HEAD this is all of R.
2. Unindexed part (commits of R the HEAD walk never reached): each is one
   bounded on-demand git read of that commit and its diff, memoized inside the
   32 MB budget (LRU, evicted like a page memo). Ranks for these follow the
   indexed ones in R's canonical order.

Guards: `memory_budget` sweeps every insights page for a non-HEAD rev (a side
branch with unindexed commits) and stays in the budget, and the equivalence
oracle has one cell for it.

## 3. Memory model

Resident, under one budget (default 32 MB, the cache budget):

- Footers and section tables of open segments: a few KB each.
- The string tables for ids that a page shows: paths and identities are loaded
  per block on demand into the bounded cache, not as a whole table (rust paths
  are 10 MB as JSON).
- The mailmap and bot decision per identity id: a `Vec<u32>` canonical id plus a
  bit, 9,784 identities on rust, under 100 KB. Built per tip from the `.mailmap`
  of that revision.
- The path filter decision per path id (exclude globs, generated, ignore): a bit
  vector, rust 188k paths, 24 KB. Per option set, at most 4 kept.
- Rendered page memo, inside the same budget.

Every cache that keeps bytes between requests takes a named share of
`resources.cache_mb` (`Cache` and `SHARES` in gitterrace-config `budget.rs`,
which sum to the whole budget). The segment block cache is one of them (a
quarter), not a fixed amount beside it. An ast-grep rule refuses a cache bound
written as a constant. After the idle release only the page memo remains. What
still separates a released server from a just-started one is allocator state,
not data: jemalloc's per-core arenas each keep partly used slabs and a thread
cache.

Streamed: everything else. A page reads the aggregate blocks for its time range
through a 1 MB block buffer, folds into a result sized by the answer (top-k
heap, a per-bucket array), and drops the buffer.

Options without loading everything:

- **bots**: aggregates keep identity ids, the per-identity bot bit drops them
  while folding.
- **mailmap**: aggregates key raw identities, canonical id via the per-tip
  vector, so editing `.mailmap` needs no rebuild.
- **period**: aggregates are day or month keyed and sorted by time, so a range
  is a seek. Partial months at a period edge come from the commit table for
  those days only.
- **exclude, generated, ignore, whitespace**: per-path bit and the raw/code
  column choice. Commit line sums under a non-default path filter cannot be
  precomputed: those pages stream file rows (rust 60 MB sequential from the page
  cache, about 150 ms measured), with the result memoized.
- **selection chips**: postings give the commit ordinal set, rows are streamed
  for those only.

Targets, as numbers:

- **Memory**: `phys_footprint` on macOS, `Private_Dirty` from
  `/proc/self/smaps_rollup` on Linux (`ru_maxrss` only where smaps is missing).
  At most 32 MB budget + 16 MB margin above a just-started server, idle and
  after a full sweep of every insights page and API endpoint. File-backed page
  cache is not counted.
- **Latency**: every page under the 300 ms warm goal. Aggregate pages read a few
  MB. The pages that stream rows (coupling, filtered views, chips) cost more:
  the targets, and the file pages that miss theirs, are in
  [benchmarks](../../../../docs/benchmarks.md#budgets).

## 4. Build path

The index build emits records in walk order. The segment writer
(`ContainerWriter` in `seg/write.rs`) consumes them:

- Commit columns and file rows are written block by block as they arrive
  (bounded buffer).
- Postings and aggregates need grouping by path or author. They accumulate in
  memory during the build (rust: about 30 MB of aggregates, about 50 MB of
  postings) and are sorted and written at the end. That is the accepted build
  peak. For linux, postings spill to sorted runs in a temp file once over 128
  MB, merged at the end (external sort, deterministic because runs are cut by
  record count, not by time or thread).
- Then fsync, rename `seg-<n>.tmp` to `seg-<n>.gts`, fsync the directory. Before
  the rename the segment does not exist for readers.

Incremental update when HEAD moves: the new commits become one new small segment
plus its string delta, then a new manifest. Readers merge segments at read time:
aggregates are additive, postings concatenate. Rewritten history: a segment
holding commits that are not reachable any more is retired and its reachable
part rebuilt, the rule "unreachable records are ignored", applied per segment.
When there are more than 8 segments, a background merge writes one segment from
them (streamed k-way merge, bounded memory) and swaps it in with a new manifest.
A merge always renumbers into the canonical order.

Canonical order. Walk order is not stable: a merged long-lived branch brings
commits dated before ones already written, so a two-step build would number them
differently. The canonical order of a reachable set is defined from the commit
graph alone: topological, parents before children, ties broken by (committer
time, oid) ascending. Commit ordinals are positions in that order. String ids
are assigned by first use in that order (path ids by first touching commit, then
by the row's position in the commit, which is the diff's sorted path order). A
fresh build sorts the reachable ids into this order before diffing, so the
streamed writer emits in it. The manifest stores a fingerprint (hash of the
ordered oid list) so a reader can tell a merged set from a multi-segment one.

What is guaranteed:

- One-step build: byte-identical across thread counts and repeats
  (`tests/deterministic.rs`, threads 1, 4, 10, 10).
- Two-step build (HEAD~k, then HEAD), then merge: byte-identical to the one-step
  build. The merge renumbers, so this holds by construction.
- Two-step build before merge: only output-equivalent (every page and API answer
  equal), not byte-equal.

Segment-local ids. Until a merge, commit ordinals, postings and string ids are
local to their segment: segment `n`'s ordinal 0 is its own first commit, and its
string delta numbers only what it introduced. A reader never compares ordinals
of two segments. Across segments it orders commits by the canonical key (topo
rank, committer time, oid), where the topo rank comes from the manifest's
ordered oid list, and maps string ids through each segment's delta. Global
ordinals exist only after a merge, which renumbers everything into one segment.

## 5. One rule for unreadable caches

No migration, no reading old caches, no negotiation. On open, the reader checks
`VERSION` and every segment footer (magic, version, CRCs). If anything is
another version, another format, truncated or corrupt, the whole cache directory
content is deleted (through the existing safe `cache::remove`, which refuses
links) and rebuilt. It is never partly served: the server answers the "indexing"
page until the rebuild is done. A block that fails its CRC at page time is
treated the same way: the request returns the indexing page, the index rebuilds.
A cache format error surfacing as HTTP 500 cannot reach a handler, because store
errors of kind `Unreadable` are mapped to rebuild in the index crate, not to an
error.

One exception: a cache written by a _newer_ build is left untouched (two
binaries would otherwise rebuild over each other). The older binary shows the
`newer_version` page with one sentence (which format it found, which it reads,
how to proceed) and no statistics.

A CRC failure at page time rebuilds the whole index (tens of minutes on linux
for one flipped bit, accepted). The index logs the file, the section and the
block offset to stderr first, so a bad disk is visible.

Guard test (`crates/gitterrace-index/tests/unreadable.rs`): for each of: old
`VERSION` with JSON record files, unknown older `VERSION`, a missing or torn
`MANIFEST`, a manifest naming a missing file or a wrong length or CRC, a segment
cut at 0, 1, half and len-1 bytes, one flipped byte in each section, a missing
footer, and a crash at every step of the write sequence (simulated by stopping
after each file operation): open, assert the cache is rebuilt or the previous
set served whole, and assert the rebuilt files equal a fresh build byte for
byte. A newer `VERSION` must stay untouched and render the `newer_version` page.
A web test requests every `WARM_PAGES` path against each corrupted cache and
asserts no 500.
