# Why gitterrace

Agents now write much of the code, and fast. Many changes land without anyone
reading the diff. The repository still holds the whole story, but nobody looks.

gitterrace is a place to look again, calmly and after the fact: what changed
today, which files keep churning, which branches and worktrees the agents left
behind. It reads the repository on your machine, offline, in a browser. When a
glance is one click away, you take it, and that often catches the change nobody
meant.

## Reading, then sending on

When you spot something, hand it on with its context as a task, note, issue or
hook. Unlike an editor's integration, targets are generic entries in a small
file you own, any URL or command works, and the destination's own add screen
asks you to confirm. gitterrace never calls the destination itself.

## Insights

How a project moves (its rhythm, what churns, who touches what) is part of the
same reading, so insights live beside the history.

## What it does with your code

- It serves on `127.0.0.1:4242` by default, so only your machine can reach it.
  Serving beyond loopback needs `--public`, which is read-only
  ([running it online](online.md)).
- It sends nothing anywhere: no telemetry, no update check, nothing loaded from
  outside.
- It never changes the repository's files or history.
- It writes only a disposable cache under `.git/gitterrace/` (plus one line in
  `.git/info/exclude` when you export into the worktree), your config and a
  state folder.

See the [security model](security.md), and [SECURITY.md](../SECURITY.md) to
report a vulnerability.

## Compared with

- A code host's web view: only what was pushed. gitterrace reads your clone,
  unpushed branches and worktrees included.
- gitk, tig, lazygit and gitui: desktop or terminal, and most also commit.
  gitterrace runs in a browser and only reads.
- A self-hosted code server: a service to run. gitterrace is one binary you
  start and stop.
- Your editor's git view: tied to that editor. gitterrace works beside any.

It is not a host and not a replacement for git or your editor.

## How it is made

AI agents and humans build gitterrace together. The agents write and review
most of the code. A human sets the direction and decides what goes in. Rules
live in lint checks and tests, and docs are generated from the code where they
can be.
