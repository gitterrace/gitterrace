# Landing page

The words on gitterrace.dev's front page, one section per block. The site
builds the page from this file, and from the package description for the
tagline. Each sentence lives here once.

## Why

We built gitterrace to read a repository before pushing it. Does the README say
what the code does? Do the docs and diagrams render? Is the history one you
would want to read? A code host answers only once it is public, and your editor
shows the code, not the page a reader sees. Now agents write much of the code,
so that reading matters more. Review what they wrote, then send each finding on
with its context: back to an agent, or as a task or ticket. Changes, unpushed
branches and worktrees, all in one place, on your machine.

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
