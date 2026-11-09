# Security model

What gitterrace protects, from whom, and what holds each door shut. To report a
vulnerability, see [SECURITY.md](../SECURITY.md).

## Assets

- Working tree files, including ignored ones such as `.env`.
- Git objects and refs, plus `.git/config` with any credentials in remote URLs.
- `config.toml`, `actions.toml` and the repository registry: editing
  `actions.toml` is code execution by design.
- Per-process tokens: the actions token, the settings token and the add token in
  its file.
- The user's other repositories and every file outside the served tree.

## Actors

- A web page in any browser tab on this machine: it can fire requests at
  loopback and try DNS rebinding.
- Another local process: it runs as the user already, so it gains nothing it
  lacks.
- A peer on the network, when the server binds beyond loopback or runs with
  `--public`.
- A hostile repository: file names, commit text, markdown, SVG, `.mailmap`, huge
  blobs, symlinks, a committed `actions.toml`.

## Doors and what holds them

| Door | Exposure | Holds it |
| --- | --- | --- |
| Every request | all | Host allowlist, cross-site requests refused, strict content security policy |
| Pages | local and public | Committed objects only when public, every read pinned to a commit |
| Local-only pages | local only | Answered 404 on a public server |
| File paths in a URL | both | Looked up by name in the tree, the working tree reader never follows a symlink out |
| `/raw` | both | SVG and text sandboxed, PDF as a download, size cap |
| Running an action | loopback only | Same-origin token, JSON only, `actions.toml` must be yours and not writable by others |
| Settings | loopback only, not in an export | Same-origin token, only `config.toml` or `actions.toml`, never a path from the request |
| Adding a repository | loopback only | Token from a private file, any browser request refused |
| Every write | all | Never succeeds without its token |
| MCP server | the process that started it | Read-only tools, the same feature switches as `/api/v1` |
| `open` and `gitterrace://` | the OS | Strict link parse, only known clones, no network for an unknown id |
| Static export | wherever it is hosted | The content security policy ships with it, no local pages |
| Repository content | rendered pages | Escaped, inline script refused, actions never load from the repository |

Every route is checked against these doors by tests that walk all of them, so a
new route is covered without a new test.

## Accepted risks

- The working tree reader checks a file and then reads it: a local process could
  swap in a symlink in between, and it already runs as the user.
- Styles allow inline rules for mermaid diagrams. A style cannot run code.
