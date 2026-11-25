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
- The per-process owner secret, in a 0600 file under the state folder, and the
  cookie a launched browser derives from it. No page holds either.
- The user's other repositories and every file outside the served tree.

## Actors

- A web page in any browser tab on this machine: it can fire requests at
  loopback and try DNS rebinding.
- Another local process of the same user: it already runs as the user and gains
  nothing.
- Another OS user on the same machine: loopback is shared, so this actor can
  fetch every page and set any header. See the multi-user section.
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
| Running an action | loopback only | Same-origin, owner cookie, JSON only, `actions.toml` outside every served repository, and on Unix owned by you and not writable by others |
| Settings | loopback only, not in an export | Same-origin, owner cookie, only `config.toml` or `actions.toml`, never a path from the request |
| Adding a repository | loopback only | Token from a private file, any browser request refused |
| Launching a browser | loopback only | One-time ticket for the holder of the private file, spent once, gone in a minute |
| Every write | all | Never succeeds without the owner secret or the cookie made from it |
| MCP server | the process that started it | Read-only tools, the same feature switches as `/api/v1` |
| `open` and `gitterrace://` | the OS | Strict link parse, only known clones, no network for an unknown id |
| Static export | wherever it is hosted | No local pages, the content security policy in a `_headers` file the host must apply |
| Repository content | rendered pages | Escaped, inline script refused, actions never load from the repository |

Tests walk every route of the built server, so a new route is covered without a
new test. Each must be classified public or local, dark on a public server when
local, send `nosniff` and the policy, read nothing outside the tree, and refuse
a write without its token.

## Multi-user host

On a machine shared with other OS users, loopback is not private: any user can
connect to the port, read every page and send any header. So headers prove
nothing about who is asking, and a secret in a page is a secret given away.

- Pages carry no token. A page body holds neither the owner secret nor the
  cookie made from it (`no_served_page_contains_a_write_token`).
- The owner secret lives in `add-token-PORT` in the state folder, mode 0600 in a
  0700 folder, read without following a link and refused when another user owns
  or can read it.
- The challenge `GET /_repos?nonce=N` answers any local program, so its answer
  is made with a key of its own. The cookie name, the cookie value and the
  challenge answer each use an independent, domain-separated key, so no nonce
  yields the cookie (`no_challenge_answer_is_a_session_or_cookie_name`,
  `challenge_answers_open_no_write_and_the_launch_flow_still_does`).
- `git terrace open` and `git terrace launch` read that file and ask `POST /_launch`
  for a ticket. Process arguments are readable by every user (`ps` on macOS,
  `/proc/PID/cmdline` on Linux), so the link is never an argument: the opener
  gets a `file://` page, mode 0600 in the state folder, that forwards to it
  (`the_opener_argument_never_holds_the_ticket`). A browser opens
  `/_launch?t=TICKET`, which spends the ticket and sets an HttpOnly,
  SameSite=Strict cookie holding a hash of the secret. The secret never sits in
  the browser. `git terrace launch --print` prints the link, which works once
  and is on the terminal, so it is the user's to guard.
- Every write, and every read of the settings files, needs that cookie on top of
  the same-origin checks, which stop other sites but are forgeable by a local
  client (`a_forging_loopback_client_without_owner_proof_writes_nothing`).
  Without it, writing is refused: another user can run no action and save no
  setting.
- Not covered: reading. Committed history, the working tree pages and the
  diffs of a served repository are readable by any local user who can reach the
  port, as on any loopback server. Run it on a machine you do not share, or bind
  it where the OS limits who connects, when that matters.
- Without a state folder the server has no secret a person can read, so no
  browser can be launched and writes stay refused.

## Accepted risks

- The working tree reader checks a file and then reads it: a local process could
  swap in a symlink in between, and it already runs as the user. The open itself
  refuses a link (`O_NOFOLLOW` on Unix, the reparse point opened as itself on
  Windows), so the swap fails rather than reads elsewhere.
- Windows checks no ACLs. `actions.toml` is accepted without an owner check, and
  the temp file a settings save writes takes the ACL its folder passes down.
  Both rely on `%APPDATA%` being private to one user, its default. A shared
  account or a folder other users can write to is not supported.
- Styles allow inline rules on a response that renders a mermaid diagram, and
  on no other page. Mermaid writes a `<style>` and style attributes into its
  SVG, and neither takes a nonce or a hash. The page policy is strict by
  default (`style-src 'self'`). A page that renders a diagram carries a
  response extension the guard reads, and only that response gets
  `'unsafe-inline'`. An absent extension means strict. A style cannot run
  code, but it can restyle the page or leak through selectors, so the scope
  stays as narrow as the diagram. A static export writes the strict policy for
  every path and one rule per page with a diagram that swaps in the loose one.
  A host applies rules in file order, so the swap detaches the header and sets
  it again. A path a host rule cannot name, or a page past the rule limit,
  keeps the strict policy and its diagram stays unstyled. The crawl in
  `crawl_tests.rs` fails on an inline style or a loose policy anywhere else,
  and the `csp-style` browser check loads a diagram page and plain pages.
