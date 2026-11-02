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

- **The local server and its guard.** It binds to `127.0.0.1` by default and refuses
  requests whose Host is not loopback (or listed in `server.hosts`). It refuses cross-site
  requests, and pages carry a strict content security policy. Scripts are locked to the app's
  own origin plus the hashes of its inline theme snippets. Styles allow inline rules only
  because mermaid writes diagram styles at runtime. The app's own markup carries no style
  attribute. A page from another site or another local port reaching gitterrace is in scope.
- **Public mode.** `--public` (or `server.public = true`) is the supported way to serve
  beyond loopback, and a non-loopback `--addr` refuses to start without it. The promises are
  listed in [running it online](docs/online.md): committed history only, full commit ids only,
  `GET` and `HEAD` only, no live updates, actions, Send to or adding repositories, and no
  host paths. A Host that is not a loopback name, the bound address or listed in
  `server.hosts` gets `403`, which stops DNS rebinding. Error bodies, including `5xx`, never carry host paths. Any
  read beyond that list, any other method, or a Host outside `server.hosts` being answered
  is in scope.
- **Adding a repository.** `POST /_repos` lets `git terrace open` add a repository to a
  running server. It answers only loopback binds, refuses any request with a browser
  `Origin` or `Sec-Fetch-Site`, needs a per-process token, and accepts only the canonical
  top folder of a git repository. The token lives in a file with mode `0600` in a state
  folder with mode `0700`. On Windows those modes do not exist, so the token file's privacy
  rests on the permissions of your user profile folder. Reaching this endpoint from a
  page, another user or a public server is in scope.
- **Static export.** `git terrace export` writes plain files only, and only below its
  output folder. It refuses to write through a symlink, and it leaves out non-inert media
  under `raw/`. A leak through an export, such as a working tree file, a host path or a
  write outside the output folder, is in scope.
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
- **The cache.** Inside a repository gitterrace writes only a disposable cache under
  `.git/gitterrace/`, plus one `/<folder>/` line in `.git/info/exclude` when you export into
  the worktree. Every write is a temp file and a rename that refuses a symlink. Writing
  anywhere else, or a symlink that redirects a write, is in scope.

Out of scope:

- serving a private repository publicly without a login in front of it. gitterrace has no
  accounts, so public mode is read-only, not private. Put a reverse proxy with a login in
  front, as [running it online](docs/online.md) describes.
- programs you configured yourself in `actions.toml`, doing what you told them to
- vulnerabilities in dependencies, which belong upstream (tell us if gitterrace is affected)
- scanner output without a realistic path to exploitation

## Supported versions

Only the latest release gets security fixes.

## Credit

Reporters are credited in the release notes when the fix ships, unless they prefer to stay
anonymous. Say which in your report.
