# Why gitterrace

Agents now write much of the code, and fast. Many changes land without anyone
reading the diff: there are too many, they arrive while you do something else,
and they mostly work. The repository still holds the whole story, but nobody
looks at it anymore.

gitterrace is a place to look again. Not every line, and not as a gate before
each merge, but calmly and after the fact: what changed today, which files keep
churning, which branches and worktrees the agents left behind, what the working
tree holds right now. It reads the repository on your machine, offline, in a
browser, without sending anything anywhere first.

Reading is cheap when it is pleasant. If one glance at a commit or a hot file is
a click away, you take that glance, and that is often enough to catch the change
nobody meant.

## Reading, then sending on

While reading you spot something: a line that looks wrong, a commit worth a
follow-up, a file that keeps changing. You want to hand it on with its context
attached, as a task, a note, an issue or a hook.

Editors can already do part of this, and that is good: some let you send a
selection straight on from where you read it. We wanted something a little
different:

- not tied to one editor, so any tool that has a URL or a command can be a
  target
- generic targets, written in a small file you own, instead of built-in
  integrations
- the destination's own add screen, so you confirm there and nothing is written
  behind your back
- room for hooks, so a selection can start a script of yours

gitterrace never calls the destination itself. It opens a URL or runs your
program, and the decision stays with you.

## Insights

How a project moves is part of the same reading: its rhythm, what churns, where
the hot spots are, who touches what. We find that interesting, at least while
doing this kind of work, so it lives beside the history and is built from the
same repository.

## What it does with your code

- It serves on `127.0.0.1:4242` by default, so only your machine can reach it.
  Serving beyond loopback needs `--public`, which is read-only
  ([running it online](online.md)).
- It sends nothing anywhere: no telemetry, no update check, and its pages load
  nothing from outside.
- It reads the repository and never changes its files or history.
- It writes only a disposable cache under `.git/gitterrace/` (plus one line in
  `.git/info/exclude` when you export into the worktree), your config and a
  state folder.

The [security model](security.md) shows which check holds each promise, and
[SECURITY.md](../SECURITY.md) says how to report a vulnerability.

## What it is not

- Not a host. There is nothing to sign in to and no network it needs.
- Not a replacement for git or your editor. It is a place to read.

## Compared with

- A code host's web view: similar pages, but only for what was pushed there.
  gitterrace reads the clone on your disk, including branches and worktrees
  nobody pushed.
- gitk, tig, lazygit and gitui: a desktop window or a terminal, and most of them
  also stage and commit. gitterrace runs in a browser and only reads.
- A self-hosted code server: a service to install and keep running. gitterrace
  is one binary you start in a repository and stop when you are done.
- Your editor's git view: part of that editor. gitterrace works beside any of
  them.

## How it is made

gitterrace is built by a group of AI agents and humans together. The agents
write and review most of the code. A human sets the direction and decides what
goes in. We say so plainly because it shapes how the project is kept: rules live
in lint checks and tests rather than in memory, and the docs are generated from
the code wherever they can be, so they cannot drift.
