# Security policy

## Reporting a vulnerability

Please do not open a public issue for a security vulnerability. Report it privately through
the repository's [private vulnerability reporting](https://github.com/gitterrace/gitterrace/security/advisories/new).

Include:

- what you found, and where
- steps to reproduce, as specific as you can make them
- what an attacker could do with it
- a proof of concept, if you have one

You will get an acknowledgement within a few days. We aim for a fix or a mitigation within
90 days, sooner for anything easy to exploit. If you hear nothing, follow up.

## What gitterrace does, and where the risk is

gitterrace serves a git repository as web pages on your own machine. It reads the
repository and never writes to it. What it shows (file contents, commit messages, branch
names) comes from the repository, so treat it as untrusted input from whoever wrote the
repository.

In scope:

- **The local server and its guard.** It binds to `127.0.0.1` by default. It refuses
  requests whose Host is not loopback and refuses cross-site requests, and pages carry a
  strict content security policy. A page from another site or another local port reaching
  gitterrace is in scope.
- **Rendering repository content.** Markdown, code, diffs and SVG from the repository are
  sanitised before display. Script running from repository content is in scope.
- **Actions.** Actions run programs you configure in `actions.toml` in your own config
  folder, never one from a repository. A run needs a per-process token, and values from
  the repository fill whole arguments with no shell. A way to run an unconfigured program,
  to inject an option or shell syntax through repository text, or to trigger an action
  from another page is in scope.
- **Send to.** Send to opens a URL built from a template you configure, with each value
  percent-encoded. A repository value that escapes its URL parameter, or a template that
  reaches a dangerous scheme (`javascript:`, `data:`, `file:`), is in scope.
- **The MCP server.** It is read-only, over stdio. A way to read outside the served
  repository is in scope.
- **The cache.** gitterrace writes only a disposable cache under `.git/gitterrace/`. Writing
  outside it, or a symlink that redirects a cache write, is in scope.

Out of scope:

- binding gitterrace to a public address on purpose (`--addr 0.0.0.0`). There is no login,
  and the docs say to keep it local.
- programs you configured yourself in `actions.toml`, doing what you told them to
- vulnerabilities in dependencies, which belong upstream (tell us if gitterrace is affected)
- scanner output without a realistic path to exploitation

## Supported versions

Only the latest release gets security fixes.

## Credit

Reporters are credited in the release notes when the fix ships, unless they prefer to stay
anonymous. Say which in your report.
