# gitterrace

A GitHub/GitLab-style view of any git repository, offline. Run it inside a
repository and open the printed address: browse code, commits, branches and
rendered markdown (with mermaid diagrams), plus insights on activity, authors
and churn. Read-only, local only, no database, no accounts.

```bash
gitterrace            # serves the repo you are in on 127.0.0.1:4242
gitterrace ~/src/foo --addr 127.0.0.1:9000
```

Browsing reads git directly. Insights come from a cache in `.git/gitterrace/`
built in the background, one commit at a time; delete it at any time.
