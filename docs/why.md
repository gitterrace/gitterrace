# Why gitterrace

Before a push, we want to see the repository the way others will: the README
rendered, the docs linked, the history readable. A code host shows that only
after the push. Editor and terminal views show the code, but not as a reader
sees it.

With agents writing much of the code, and fast, that reading is how we catch
what nobody meant. Every finding needs a place to go: back to an agent, or as
a task or ticket. The [tools we knew](why.md#compared-with) could show the
code, but not send it on.

What it promises about your code, from where it listens to what it writes, is
in the [security model](security.md).

## Reading, then sending on

Send to and actions do the handing on, as a task, note, issue or hook. Unlike
an editor's integration, targets are generic entries in a small file you own,
any URL or command works, and the destination's own add screen asks you to
confirm. gitterrace never calls the destination itself.

## Insights

How a project moves (its rhythm, what churns, who touches what, which branches
and worktrees were left behind) is part of the same reading, so insights live
beside the history.

## Compared with

- A code host's web view: only what was pushed. gitterrace reads your clone,
  unpushed branches and worktrees included.
- gitk, tig, lazygit and gitui: desktop or terminal, and most also commit.
  gitterrace runs in a browser and only reads.
- gitweb and cgit: web browsers for git, read-only as well, but dated. They
  show plain listings: no rendered docs or diagrams, no insights, and no way
  to send a finding on.
- A self-hosted code server: a service to run. gitterrace is one binary you
  start and stop.
- Your editor's git view: tied to that editor. gitterrace works beside any.

It is not a host and not a replacement for git or your editor.

## How it is made

AI agents and humans build gitterrace together. The agents write and review
most of the code. A human sets the direction and decides what goes in. Rules
live in lint checks and tests, and docs are generated from the code where they
can be.
