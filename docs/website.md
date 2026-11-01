# Website

gitterrace.dev is the one page in `site/`, served by a Cloudflare Worker with static assets.
gitterrace.com and both `www` hosts redirect to it.

## Decisions

- **Generated, not authored.** `cargo xtask generate` writes every file in `site/` from the
  repo's own sources (Cargo.toml, clap, the registries, `docs/why.md`, the brand SVGs, the
  theme). `make docs-check` fails when the page is stale, so the page cannot drift from the
  binary it describes. Fix a fact at its source, then `make docs-gen`.
- **A Worker with static assets, not Pages.** Deploy with `wrangler deploy`. `wrangler pages
  deploy` cannot see a Worker and reports "Project not found", which reads as a missing
  project. There is no Worker script: Cloudflare serves the files and honours `_headers` and
  `404.html`.
- **No script on the page.** The CSP says `script-src 'none'`. The page needs none, and a
  script added later fails visibly instead of shipping.
- **No install line before the release.** `make site`, which every deploy runs first, refuses
  while the repository, the Homebrew tap or the `v<version>` tag does not answer. The page
  must never tell anyone to run a command that fails.
- **One canonical host.** Every page canonicals to `homepage` in
  `crates/gitterrace/Cargo.toml`. The other hosts redirect with a 301, never serve a copy.

## One-time setup

The steps below are dashboard actions. Scripting them needs a Zone:Edit token, a larger
credential than the task deserves, so `make domains` measures the result instead.

1. Add gitterrace.dev and gitterrace.com to Cloudflare and point the registrar's nameservers
   at it.
2. Create the `gitterrace-web` Worker: Workers and Pages, Create, "upload static files". Then
   run `make deploy` once.
3. On the Worker, Settings, Domains and Routes: add the custom domain `gitterrace.dev`.
4. Redirects: for `www.gitterrace.dev`, `gitterrace.com` and `www.gitterrace.com`, add a
   proxied DNS record (`AAAA 100::`) and a Redirect Rule to `https://gitterrace.dev` with
   status 301, preserving the path.
5. Log in once on the machine that deploys: `npx wrangler@4 login`. There is no deploy
   workflow: the site ships from a maintainer's machine with `make deploy`, so no Cloudflare
   token lives in the repository's secrets.
6. `make domains` until every line is `[+]`. Run it again after any DNS change.

## Each release

Bump `SECURITY_TXT_EXPIRES` in `xtask/src/site.rs`, `make docs-gen`, commit, then
`make deploy`. After pushing the tag, refresh demo.gitterrace.dev from
[gitterrace/demo](https://github.com/gitterrace/demo) with `make deploy` there.

## Demo

demo.gitterrace.dev is `git terrace export` of well-known public repositories, each at a release
tag: the real app's pages, frozen, to click around without installing anything. It exports
like `git terrace` serves a folder of repositories: an index at `/`, each repository under
`/r/<name>/`. It lives in its own repository, [gitterrace/demo](https://github.com/gitterrace/demo):
there `make build` clones the repositories in `repos.txt` and exports them, and `make deploy`
ships the `gitterrace-demo` Worker. `[workspace.metadata.demo]` in this `Cargo.toml` names the
URL and the repositories for the README and site link; keep it in step with `repos.txt`.

- **Someone else's projects, not ours.** This repository's history is ten layer commits on one
  day, which makes a thin demo. jq and ripgrep have years of history and hundreds of
  contributors, and contrast with each other: jq is kept by a team that changed over the
  years, ripgrep mostly by one person. Both are MIT licensed.
- **Each one credited.** Every page names and links its project in the snapshot banner
  ("Static snapshot of jq at jq-1.8.2, shown in gitterrace") through `export --project`.
- **Pinned to a release.** Each repository is checked out on a branch named after its tag, so
  pages show a branch, not a detached HEAD. To refresh, bump a tag in the demo's `repos.txt` and
  run `make deploy` there, and also after each gitterrace release, so the demo shows the newest app. A
  local step like `make deploy`, no workflow.
- **It says what it is.** Search, live updates, actions and the API need a server; the export
  hides them and the banner links to install.
- **Not indexed.** The demo's `_headers` sends `X-Robots-Tag: noindex`: the site is the page to
  find, the demo is where it links.
- **Every insights page is in it.** Export seeds its crawl from the server's prewarmed pages, and
  a test fails when one is missing, for one repository and for several.

One-time setup: deploy once from gitterrace/demo; the route in its `wrangler.toml` creates the
custom domain `demo.gitterrace.dev`.

## Before the release

gitterrace.dev is blocked at the edge until the release, so nobody reads install lines that
fail yet. `make stage` deploys behind that block: it runs every check `make deploy` runs except
the release check. Both refuse uncommitted `site/` or `wrangler.toml`, so what ships is a
commit. Never run `make stage` with the block lifted.
