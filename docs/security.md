# Security model

What git terrace protects, from whom, at which door, and which test holds each
door shut.

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

| Door                                                   | Exposure                     | Holds it                                                                                        | Test                                                   |
| ------------------------------------------------------ | ---------------------------- | ----------------------------------------------------------------------------------------------- | ------------------------------------------------------ |
| Every request                                          | all                          | Host allowlist, `Sec-Fetch-Site` refusal, nosniff, CORP, CSP (`guard.rs`)                       | `guard.rs` tests, `security_class_tests`               |
| Page routes (`PUBLIC`)                                 | local and public             | Committed objects only when public (`restrict_to_committed`), time bound                        | `public.rs` tests                                      |
| Local routes (`LOCAL`)                                 | local only                   | `only_public` 404s anything not listed in `PUBLIC`                                              | `every_local_route_is_dark_on_a_public_server`         |
| Path tails (`blob`, `raw`, `history`, `worktree/diff`) | both                         | Tree lookup by name, worktree reader canonicalizes the parent and never follows a final symlink | `no_path_route_reads_outside_the_tree`                 |
| `/raw`                                                 | both                         | `sandbox` CSP for SVG and text, PDF as attachment, size cap                                     | `wiring_tests`, `security_class_tests`                 |
| `POST /_actions/run`                                   | loopback only                | `same_origin_write`, JSON only, owner and mode checks on `actions.toml`                         | `run_route_security_table`                             |
| `/_settings/*`                                         | loopback only, not in export | `same_origin_write` on read and write, enum file choice, stamp                                  | `settings/tests.rs`                                    |
| `/_repos`                                              | loopback only                | Add token from a 0600 file, any browser header refused                                          | `add/tests.rs`                                         |
| Every non-GET method                                   | all                          | Tokenless writes never succeed                                                                  | `no_write_succeeds_without_the_token`                  |
| MCP server                                             | stdio, the spawning process  | Read-only tools, same feature switches as `/api/v1`                                             | `mcp_tools_follow_the_switches_of_the_web_api`         |
| `open` and `gitterrace://`                             | the OS                       | Strict link parse, lookup only among known clones, no network for an unknown id                 | `open` tests                                           |
| Static export                                          | wherever it is hosted        | `_headers` carries the CSP, no local routes crawled                                             | `export/tests.rs`                                      |
| Hostile content                                        | rendered pages               | Askama escaping, the CSP refuses inline script, actions never load from the repository          | `hostile_repo_content_is_escaped_where_the_menu_lives` |

## Class guards

The tests in `security_class_tests.rs` read every route pattern from the built
routers, the server's and one repository's, so a route registered in any file is
walked. Each must be public or in `LOCAL`, and in the sitemap. Every route is
then checked for public darkness under every method, headers on every method it
takes with each token a page holds, path escapes in its tail and in `?path=`,
`?file=` and `?doc=`, and tokenless writes, without a new test.

## Accepted risks

- The worktree reader checks metadata then reads: a local process could swap in
  a symlink in between, and it already runs as the user.
- Styles allow `unsafe-inline` for mermaid. A style cannot run code.
