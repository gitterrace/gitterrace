# Architecture

```
git objects --> gitterrace-git --(on demand)--> gitterrace-web --> browser
                    |                           ^
                    +--> gitterrace-index ----------+
                         .git/gitterrace/commits.jsonl (append-only)
```

## Index format (v1)

- `.git/gitterrace/VERSION` holds the cache format version (`FORMAT_VERSION` in
  `crates/gitterrace-index/src/store.rs`). A mismatch deletes and rebuilds, so a
  format change only costs one rebuild.
- `.git/gitterrace/commits.jsonl`: one `CommitRecord` per line, keyed by id.
- `update` walks `rev_list`, skips ids already present, appends the rest.
  Records whose commits are no longer reachable are ignored by `records`, so
  rewritten history needs no cleanup.
- A torn last line (crash mid-append) is dropped on read.

## Pages

| Route                     | Source                                        |
| ------------------------- | --------------------------------------------- |
| `/`                       | overview: README, recent commits, key numbers |
| `/log/{rev}?path=&page=`  | git                                           |
| `/commit/{id}`            | git                                           |
| `/tree/{rev}/{path}`      | git                                           |
| `/blob/{rev}/{path}`      | git                                           |
| `/blame/{rev}/{path}`     | git                                           |
| `/refs`                   | git                                           |
| `/stats`                  | index: activity, heatmap, authors             |
| `/stats/files`            | index: churn, hotspots                        |
| `/stats/types`            | index: conventional-commit type and scope     |
| `/api/progress`           | index build progress (JSON)                   |
