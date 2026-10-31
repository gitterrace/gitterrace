<h1 align="center">
<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/brand/readme-header-dark.svg">
  <img alt="gitterrace" src="assets/brand/readme-header-light.svg" width="600">
</picture>
</h1>

<p align="center"><b>gitterrace: your repository's history, step by step. A GitHub-style view of any git repo, offline, one binary. Run <code>git terrace</code>.</b></p>

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
