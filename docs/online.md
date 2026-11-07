# Running it online

gitterrace is built to run on your own machine, bound to `127.0.0.1`. That stays the
recommendation. When you want others to read a repository, publish a static export or use public
mode. Nothing else binds to another address.

## A static export

The safest way to put a repository online is no server at all. `git terrace export` writes the
site as plain files: files, commits, diffs, blame, history and insights for the newest commits,
and nothing else. Any static host or CDN can serve the folder. There is no process to attack,
no working tree to leak and no action to run.

```sh
git terrace export site ~/src/project --commits 300 --no-actions --no-watch
```

It is a single command, so it scripts well: run it from CI or a cron job on each release and
upload the folder. Several repositories go into one site (`export site repo1 repo2`), and
`--project name=url` credits a repository that is someone else's work in every page's banner.

Every page ends with one credit line: `Source: name, MIT license, https://github.com/owner/name`.
The license is recognized from the root `LICENSE` or `COPYING` file (a text that is not a common
license links to the file instead). The link is the `origin` remote, or the `--project` URL, with
any credentials removed, and only for public https and ssh hosts. Either part is left out when
unknown.

The [live demo](https://demo.gitterrace.dev) is built this way. Its build script clones each
repository at a release tag, exports them all into one folder with `--commits 300 --no-actions`,
adds a header file that limits scripts and styles to the same origin, and deploys the folder.
Bumping a tag and rerunning the script publishes the new release.

An export is a snapshot. It shows what was committed when it ran and is updated only by running
it again. It has no search, live updates or actions, since those need a server. When you need
those, or any revision on demand, use public mode.

## Public mode

`--public` (or `server.is_public = true` in `config.toml`) turns the server into a read-only
viewer of committed history. Any `--addr` that is not loopback refuses to start without it.

What a public server shows:

- Files, commits, diffs, blame, history, compare, search and insights at `HEAD`, any branch,
  tag or remote-tracking branch, or a full commit id.
- The JSON API under `/api/v1` with the same revisions, only when `features.api_enabled = true`
  says so. It is off on a public server by default.
- `/go/...` permalinks.

What it never shows or does:

- The working tree, uncommitted changes, stashes, linked worktrees, reflogs or any other ref.
- Abbreviated commit ids: only full ids resolve, so the object store cannot be enumerated.
- Paths of folders on the host, an in-progress rebase or merge.
- Live updates (no `/events`), actions, Send to targets, and adding repositories.
- Any method but `GET` and `HEAD`.

Every request that takes longer than `server.public_timeout_secs` (default 30) answers `503`.
Pages that are slow on a large history (blame, insights) finish in the background and answer
from the cache next time.

Any feature in `[features]` can be switched off for a public server as well, for example
`blame_enabled = false` or `search_enabled = false`. A switched-off page answers 404 and its links
are gone. The ones that show this machine stay off whatever the file says.

## Behind a reverse proxy

Run gitterrace on loopback and put a TLS reverse proxy in front of it. The proxy owns TLS, HSTS,
connection limits, slow-client timeouts and, for a private repository, the login.

```toml
[server]
addr = "127.0.0.1:4242"
is_public = true
hosts = ["git.example.com"]
```

`hosts` lists the exact names the proxy forwards in the `Host` header, without scheme or port.
Wildcards are refused. Any other name answers `403`, which stops DNS rebinding. Forward the
original `Host`, since gitterrace reads no `X-Forwarded-*` header.

## What not to do

- Do not serve a clone with work you have not published: everything committed on any branch or
  tag is readable, and history includes deleted files.
- Do not expose it without TLS or without the proxy's login when the repository is private.
  gitterrace has no accounts.
- Do not bind `0.0.0.0` on a shared network for convenience: public mode makes it safe to
  read, not private.
