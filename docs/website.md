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
`make deploy`. After pushing the tag, `make demo` refreshes demo.gitterrace.dev.

## Demo

demo.gitterrace.dev is `git terrace export` of this repository at its newest tag: the real app's
pages, frozen, to click around without installing anything. `make demo` builds it from a fresh
clone of the tag (no working-tree changes, local branches or stashes leak in) into
`target/demo`, then deploys the `gitterrace-demo` Worker from `demo/wrangler.toml`.

- **One tag, one demo.** The demo shows a release, not main, so it matches what `brew install`
  gives. Run `make demo` after each tag; it is a local step like `make deploy`, no workflow.
- **It says what it is.** Every exported page carries a "Static snapshot at <tag>" banner with
  a link to install, so nobody mistakes it for a live view. Search, live updates, actions and
  the API do not exist in a static copy.
- **Not indexed.** `demo/_headers` sends `X-Robots-Tag: noindex`: the site is the page to
  find, the demo is where it links.
- **Every insights page is in it.** Export seeds its crawl from the server's prewarmed pages, and
  a test fails when one of them is missing from an export.

One-time setup: add the custom domain `demo.gitterrace.dev` by deploying once (`make demo`);
the route in `demo/wrangler.toml` creates it.

## Before the release

gitterrace.dev is blocked at the edge until the release, so nobody reads install lines that
fail yet. `make stage` deploys behind that block: it runs every check `make deploy` runs except
the release check. Both refuse uncommitted `site/` or `wrangler.toml`, so what ships is a
commit. Never run `make stage` with the block lifted.
