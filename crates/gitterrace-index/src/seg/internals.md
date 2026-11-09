# The index internals

How the insights cache is stored, read and kept small, for contributors to
`gitterrace-index`. The [architecture](../../../../docs/architecture.md#cache)
page has the short version, and the [store design](store.md) the byte format.

## How the index is stored

The index is a set of immutable files. A segment holds a run of commits as
columns, file rows, path and author postings, and the aggregates that insights
pages read (author by day, path by author, month by path). A string file beside
it holds the paths, identities and labels that segment introduced. Nothing is
ever written to again once it is renamed into place.

`MANIFEST` is the only entry point. It names every live file with its length and
CRC32, and is swapped by rename, so a crash leaves the previous complete set.
Files it does not name are ignored and swept. A new HEAD adds one small segment,
and many segments are merged back into one.

Pages never load the history. A reader opens a segment, checks its footer, and
reads blocks with `pread`. Each block is checked against its CRC before any
field in it is parsed, and every length and offset is bounded, since `.git/` may
belong to someone else.

One rule covers damage: if the cache cannot be read, it is deleted and rebuilt.
That means a different format version, a torn manifest, a bad CRC, or a changed
shallow set. It is never partly served. The page shows indexing until the
rebuild is done. A cache written by a newer build is left alone and the page
says so.

## How commit search reads the index

Commit search matches a regex on each commit's full message plus its author as
`name <email>` after the mailmap, newest first. Walking git for that decodes
every commit, so a miss on a large history takes seconds. The message column
keeps what the search needs in the index instead.

What it holds: one item per commit, in the order of the segments. An item is
the message as the walk decodes it, the author as written, the committer time
and the parents. The mailmap is not baked in: the tip's `.mailmap` is applied to
each distinct author once per search, so editing it never rebuilds anything.
Items are packed into blocks of about 64 KiB, each compressed with zstd. The
blocks live in `msg-<gen>-<n>.gts` files of the same container format as the
segments. `MANIFEST` names each file with its length and CRC32, like every other
file, so the column is part of the one live set.

How a search runs: the blocks are read with `pread`, checked against their CRC,
inflated on up to 8 threads, and the regex runs on each item. The newest-first
order is git's own walk replayed over the stored times and parents, ties
included, so the list is the one the walk gives. It stops at
`limits.search_commits` and says when more matched. Only then does the search
open commit objects, one per listed row, for its subject and dates. A miss lists
no rows, so it reads no commit object at all. An item over 512 KiB (a generated
message) is stored without its text and checked from git.

How it stays current: the refresh task that updates the index after HEAD moves
also adds the column's new commits, under the same writer lock, as one more
file per update. When the segments are merged, the column is rewritten in the
new order beside them. A column that does not reach the searched commit (being
built, or a rev that is not HEAD) is not used, and that search walks git.

Damage: the column follows the rule above. A bad CRC, a file cut short, a block
that does not inflate, or another format version means the cache is deleted and
rebuilt. It never yields a wrong or partial list. The request that met the
damage logs it once and walks git for its answer, and insights pages show
indexing until the rebuild is done. With the setting off, the column is dropped
at the next update and search walks git as it always did.

The setting is `[index] has_messages`, on by default. On rust (345k commits) it
costs 31 MB of cache, 12% more, and 16 s to build the first time. A miss there
takes 68 ms instead of 3.1 s.

Why not a search library: Tantivy indexes words or trigrams and cannot run our
regex over a whole message. Our code would still pick candidates, run the regex
and order the hits, which is most of the work. Measured on rust, its index was
113 MB against the column's 31 MB, and a wide hit (`fix`) took 670 ms against
the walk's 22 ms, because every candidate is checked before the order is known.
A regex without a literal of three characters still scanned everything (3.1 s).
It added 3.2 MB to the binary and 59 crates. It panicked on a truncated file
instead of returning an error, and it reads through mmap, which breaks the rule
that serving reads bounded blocks with `pread`.

SQLite full-text search (FTS5 with a trigram index) answers faster: on rust a
miss took under 1 ms, a regex hit 19 ms and a wide hit on `fix` 77 ms, with our
regex checking each candidate. A regex without plain text still scanned
everything (265 ms). Its index was 362 MB against the column's 31 MB, and the
first build took 33 s against 3.8 s. It added 1.7 MB to the binary and 9 crates,
among them SQLite's C source. It keeps no checksums: of 6 damage cases 5 gave an
error, and garbage in the middle of the file went unseen, since the query never
read the 16 damaged pages. The column stays because it is under a tenth of the
size, builds about nine times faster, needs no C database engine (its only C is
zstd), and never serves from a damaged file without noticing.

## Memory

While serving, resident memory is a budget of `resources.cache_mb` (default 32
MB) for caches and rendered pages, plus 16 MB of margin. It does not grow with
the history. A page folds the blocks of its time range through a small buffer
into a result sized by the answer. The OS file cache is not counted. The peak
during an index build is higher and is accepted.

After 30 s without a request the server drops every cache but the rendered
pages, and the allocator hands freed pages back to the OS at once. Builds from
this repository run jemalloc with one arena (`.cargo/config.toml`), so an idle
server returns to about the footprint it started with. The
full format is in the [store design](store.md).

## The InsightsSource seam

Pages read history through the `InsightsSource` trait in
`gitterrace-index/src/feed/`. It gives commit columns for a range, file rows for
chosen commits, the aggregates, postings and strings, all as streams.
`feed/segments.rs` serves it from the files. `feed/memory.rs` serves it from
records in memory and is the oracle that tests compare the segments against. A
page does not know which one it has.
