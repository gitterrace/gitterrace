# Security policy

## Reporting a vulnerability

Do not open a public issue. Report it through
[private vulnerability reporting](https://github.com/gitterrace/gitterrace/security/advisories/new),
with what you found and where, steps to reproduce, the impact and a proof of
concept if you have one.

You get an acknowledgement within a few days. We aim for a fix or mitigation
within 90 days, sooner for anything easy to exploit. If you hear nothing,
follow up.

## Scope

gitterrace serves a git repository as web pages on your machine and never
changes its files or history. Treat everything it shows as untrusted input from
whoever wrote the repository.

In scope:

- **The local server and its guard.** It binds to `127.0.0.1` by default,
  refuses a Host that is not loopback or in `server.hosts`, refuses cross-site
  requests and sends a strict content security policy: scripts from its own
  origin plus hashed theme snippets, inline styles only for mermaid, and no
  style attribute in its own markup. A page from another site or local port
  reaching gitterrace is in scope.
- **Public mode.** `--public` is the only way to serve beyond loopback. Its
  promises are in [running it online](docs/online.md). Any read beyond them, any
  other method, or an answer to a Host outside `server.hosts` is in scope.
- **Adding and removing a repository.** `POST` and `DELETE /_repos` answer only
  loopback binds, refuse a browser `Origin` or `Sec-Fetch-Site`, need a
  per-process token, and accept only a served name or a repository's canonical
  top folder. The token is in a `0600` file in a `0700` folder (on Windows,
  protected by your profile folder's permissions). Before sending a path,
  `git terrace` checks the listener: `GET /_repos?nonce=N` must answer the
  SHA-256 of the token and the nonce, so the token never leaves the server. A
  listener that cannot answer is never sent a path.
  Reaching these endpoints from a page, another user or a public server is in
  scope.
- **Static export.** It writes plain files only below its output folder,
  refuses to write through a symlink and leaves out non-inert media. A leaked
  working tree file or host path, or a write outside the folder, is in scope.
- **Rendering repository content.** Markdown, code, diffs and SVG are sanitized.
  Script running from repository content is in scope.
- **The `.git` folder.** A served repository's `.git/config`, `.gitattributes`
  and `objects/info/alternates` are trusted: gitterrace reads and follows them
  as git does. A repository cloned as a shared or referenced clone of an
  untrusted source, or one whose `.git` folder was tampered with, is untrusted
  input. The worktree view runs the filter drivers `.gitattributes` assigns,
  and public mode turns that view off. A commit-graph is read only from the
  repository's own objects. Reading data from a repository outside the served
  tree through alternates, past what git itself would show, is in scope.
- **Actions.** Actions run programs from `actions.toml` in your config folder,
  never from a repository, only on a loopback server that is not public, with a
  per-process token. Repository values fill whole arguments with no shell.
  Running an unconfigured program, injecting an option or shell syntax, or
  triggering an action from another page is in scope.
- **Send to.** Each value is percent-encoded into your URL template. A value
  that escapes its parameter, or a template reaching a dangerous scheme
  (`javascript:`, `data:`, `file:`), is in scope.
- **The MCP server.** Read-only, over stdio. Reading outside the served
  repository is in scope.
- **The cache.** In a repository gitterrace writes only `.git/gitterrace/`,
  plus one line in `.git/info/exclude` when you export into the worktree, each
  by temp file and a rename that refuses a symlink. Any other write, or a
  symlink that redirects one, is in scope.

Out of scope:

- serving a private repository publicly without a login in front. Public mode
  is read-only, not private ([running it online](docs/online.md)).
- programs you configured in `actions.toml`, doing what you told them to
- vulnerabilities in dependencies, which belong upstream (tell us if gitterrace
  is affected)
- scanner output without a realistic path to exploitation

## Supported versions

Only the latest release gets security fixes.

## Credit

Reporters are credited in the release notes unless they prefer to stay
anonymous. Say which in your report.
