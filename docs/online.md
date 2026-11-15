# Running it online

gitterrace is built to run on your own machine, bound to `127.0.0.1`. To let
others read a repository, publish a static export or use public mode. Nothing
else binds to another address.

## Publish a static export

The safest option is no server at all. `git terrace export` writes files,
commits, diffs, blame, history and insights for the newest commits as plain
files for any static host. There is no process to attack, no working tree to
leak and no action to run.

```sh
git terrace export site ~/src/project --commits 300
```

Several repositories go into one site (`export site repo1 repo2`), and
`--project name=url` credits someone else's repository in every page's banner.
The export writes a `_headers` file with the content security policy and
`Cache-Control: no-cache` on the pages. A browser then revalidates them after a
new export. Scripts, styles and fonts carry the build's version
(`app.css?v=...`) and are kept a year. A new export names new versions, so a
fresh page never runs a stale script. Netlify and Cloudflare Pages apply the
file. Elsewhere, set the same headers on the host.

Every page ends with a credit line such as
`Source: name, MIT license, github.com/owner/name.`. The license comes
from the root `LICENSE`, `LICENCE` or `COPYING` (an uncommon text links to the
file). The link is the `origin` remote or `--project` URL without credentials,
only for public https and ssh hosts. An unknown part is left out. The
[live demo](https://demo.gitterrace.dev) is built this way.

An export always follows the public rule below, whatever `server.is_public`
says: committed history reachable from branches, tags, remote-tracking branches
and `HEAD`, and nothing of this machine. No working tree, stash, other ref,
action, Send to target, settings page or host path reaches it.

An export is a snapshot with no search, live updates or actions. For those, or
any revision on demand, use public mode.

## Serve in public mode

`--public` (or `server.is_public = true`) makes the server a read-only viewer of
committed history. Any `--addr` that is not loopback refuses to start without
it.

A public server shows files, commits, diffs, blame, history, compare, search and
insights at `HEAD`, any branch, tag or remote-tracking branch, or a full commit
id, plus `/go/...` permalinks. The JSON API under `/api/v1` is off unless
`features.api_enabled = true`.

It never shows or does:

- The working tree, uncommitted changes, stashes, linked worktrees, reflogs or
  any other ref.
- Abbreviated commit ids, so the object store cannot be enumerated.
- A full commit id that no public branch, tag, remote branch or HEAD reaches,
  so a stash or dangling commit stays private.
- Host folder paths, an in-progress rebase or merge.
- Live updates, actions, Send to, or adding repositories.
- Any method but `GET` and `HEAD`.

A request over `server.public_timeout_secs` (default 30) answers `503`. Slow
pages finish in the background and answer from the cache next time.

`[features]` can switch off more pages, such as `blame_enabled = false`: the
page answers 404 and its links are gone. The
ones that show this machine or change the server stay off whatever the file
says.

## Run with Docker

The root `Dockerfile` serves one repository, or a folder of repositories two
levels deep:

```sh
docker build -t gitterrace .
docker run --rm -p 127.0.0.1:8080:8080 -v "$PWD":/repos:ro \
  -v gitterrace-cache:/cache gitterrace
```

Open `http://localhost:8080`. The image runs in public mode. Keep `127.0.0.1:`
in `-p` unless the network may read every commit. The `/cache` volume keeps the
index across restarts. Mount a config folder on `/config` for other settings.
No prebuilt image is published. The build takes 5 to 10 minutes, on amd64 or
arm64.

On `docker stop` the server lets open responses finish and exits 0, well
inside Docker's 10 seconds. The healthcheck runs `git-terrace --version`. It
proves the binary starts, not that the port answers: the image has no shell or
HTTP client. A client that does not send its request headers in 10 seconds is
disconnected.

## Put it behind a reverse proxy

Put a TLS reverse proxy in front of gitterrace on loopback. The proxy owns
TLS, HSTS, connection limits, slow-client timeouts and, for a private
repository, the login.

```toml
[server]
addr = "127.0.0.1:4242"
is_public = true
hosts = ["git.example.com"]
```

`hosts` lists the exact names the proxy forwards in `Host`, without scheme or
port. Wildcards are refused. Any other name answers `403`, which stops DNS
rebinding. Forward the original `Host`: gitterrace reads no `X-Forwarded-*`.

## What not to do

- Do not serve a clone with unpublished work: every branch and tag is readable,
  and history includes deleted files.
- Do not expose a private repository without TLS and the proxy's login.
  gitterrace has no accounts.
- Do not bind `0.0.0.0` on a shared network for convenience: public mode makes
  it safe to read, not private.
