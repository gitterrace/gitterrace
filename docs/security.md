# Security model

What gitterrace protects, from whom, and what holds each door shut. To report a
vulnerability, see [SECURITY.md](../SECURITY.md).

## What it writes

gitterrace never writes your repository or your working tree: no object, ref,
config or working file changes. It keeps one folder of its own,
`.git/gitterrace/`, for the index and, from 0.2.1, your checkpoints, viewed
marks and review notes. Delete the folder at any time: the index rebuilds, and
the marks and notes are forgotten. A checkpoint holds copies of changed text
files from your working tree (never ignored or binary files), so treat that
folder like the repository.

## Assets

- Working tree files, including ignored ones such as `.env`. A checkpoint copies
  changed text files that are not ignored into `.git/gitterrace/`, so that
  folder is as sensitive as the working tree.
- Git objects and refs, plus `.git/config` with any credentials in remote URLs.
- `config.toml`, `actions.toml` and the repository registry: editing
  `actions.toml` is code execution by design.
- Per-process tokens: the actions token, the settings token and the add token in
  its file.
- The user's other repositories and every file outside the served tree.

## Actors

- A web page in any browser tab on this machine: it can fire requests at
  loopback and try DNS rebinding.
- Another local process: it already runs as the user and gains nothing.
- A peer on the network, when the server binds beyond loopback or runs with
  `--public`.
- A hostile repository: file names, commit text, markdown, SVG, `.mailmap`, huge
  blobs, symlinks, a committed `actions.toml`.

## Doors and what holds them

| Door | Exposure | Holds it |
| --- | --- | --- |
| Every request | all | Host allowlist, strict content security policy, cross-site requests refused on a local server |
| Pages | local and public | Committed objects only when public, every read pinned to a commit |
| Local-only pages | local only | Answered 404 on a public server |
| File paths in a URL | both | Looked up by name in the tree, the working tree reader never follows a symlink out |
| `/raw` | both | SVG and text sandboxed, PDF as a download, size cap |
| Running an action | loopback only | Same-origin token, JSON only, `actions.toml` outside every served repository, and on Unix owned by you and not writable by others |
| Settings | loopback only, not in an export | Same-origin token, only `config.toml` or `actions.toml`, never a path from the request |
| Adding a repository | loopback only | Token from a private file, any browser request refused |
| Every write | all | Never succeeds without its token |
| MCP server | the process that started it | Read-only tools, the same feature switches as `/api/v1` |
| `open` and `gitterrace://` | the OS | Strict link parse, only known clones, no network for an unknown id |
| Static export | wherever it is hosted | No local pages, the content security policy in a `_headers` file the host must apply |
| Repository content | rendered pages | Escaped, inline script refused, actions never load from the repository |

Tests walk every route of the built server, so a new route is covered without a
new test. Each must be classified public or local, dark on a public server when
local, send `nosniff` and the policy, read nothing outside the tree, and refuse
a write without its token.

## Accepted risks

- The working tree reader checks a file and then reads it: a local process could
  swap in a symlink in between, and it already runs as the user. The open itself
  refuses a link (`O_NOFOLLOW` on Unix, the reparse point opened as itself on
  Windows), so the swap fails rather than reads elsewhere.
- Windows checks no ACLs. `actions.toml` is accepted without an owner check, and
  the temp file a settings save writes takes the ACL its folder passes down.
  Both rely on `%APPDATA%` being private to one user, its default. A shared
  account or a folder other users can write to is not supported.
- Styles allow inline rules for mermaid diagrams. A style cannot run code.
