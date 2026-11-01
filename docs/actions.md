# Actions

Select lines, a hunk, a blame line, a search hit, a hotspot row, a commit or a whole page,
then press `.` or click the `⋯` button. The menu always offers two built-ins:

- **Copy permalink**: `/go/<repo-id>/<sha>/<path>#L<a>-L<b>`. The repo id is the first 12
  hex digits of the repository's oldest root commit, so the link survives clones, renames
  and moves. It opens on any machine that serves the same repository.
- **Copy as markdown**: the file, range and short sha in bold, the permalink, then a fenced
  snippet (at most 40 lines). For a commit: subject, sha, author, permalink.

Both run in the page and need no configuration. Everything below is about your own
actions, which run a program of yours with the selection as data.

## Configuration

Actions live in `actions.toml` in gitterrace's config folder, next to `config.toml`:
`$XDG_CONFIG_HOME/gitterrace`, else `~/.config/gitterrace` (macOS included), or the folder
given by `--config DIR`. Nothing else moves it: not the served repository, its `.git`, the
current folder, or an environment variable.

```toml
version = 1

[[action]]
id      = "tendir"              # [a-z0-9-], 1 to 32 characters, unique
label   = "Send to tendir"      # menu text
kinds   = ["line", "hunk", "commit", "blame", "page"]
command = ["~/bin/gt-tendir"]   # argv, no templating; argv[0] resolved once at load
cwd     = "home"                # "home" (default), "repo" (the repo root) or an absolute folder
timeout = "10s"                 # default 10s, at most 120s; also "500ms"
output  = "toast"               # "toast" (first output line) or "ignore" (failures only)
key     = "t"                   # optional shortcut inside the menu
env     = ["SSH_AUTH_SOCK"]     # optional: variables passed through from gitterrace
```

`kinds` takes `line`, `file`, `commit`, `hunk`, `blame`, `search-hit`, `insight` and `page`.
An unknown kind, key or field is an error, so typos surface.

`shell = true` makes `command` one string run as `sh -c "<string>"`. Nothing is substituted
into it: refer to the selection through quoted variables, `"$GITTERRACE_PATH"`.

The file is refused as a whole (logged, the server runs without actions) unless it is a
regular file owned by you and not writable by group or others, in a folder with the same
properties, and outside every served repository. Edits are picked up within a second; an
invalid edit keeps the previous set. Going from no actions to some needs a restart.

Actions are off when gitterrace listens on anything but loopback, and `--no-actions` turns
them off for one run.

## Payload (version 1)

Your program gets JSON on stdin, then end of file:

```json
{
  "v": 1,
  "action": "tendir",
  "kind": "line",
  "repo": { "name": "gitterrace", "root": "~/src/gitterrace", "id": "a1b2c3d4e5f6" },
  "rev": "main",
  "sha": "4f0c2e9d0000000000000000000000000000abcd",
  "path": "crates/gitterrace-web/src/guard.rs",
  "lines": { "start": 44, "end": 60 },
  "text": "fn refusal(headers: &HeaderMap ...",
  "text_truncated": false,
  "permalink": "http://127.0.0.1:4242/go/a1b2c3d4e5f6/4f0c2e9d.../crates/gitterrace-web/src/guard.rs#L44-L60",
  "title": "guard.rs:44-60 @ 4f0c2e9",
  "commit": { "subject": "fix(web): refuse same-site", "author": "Timo", "date": "2026-10-30T12:00:00Z" },
  "markdown": "**guard.rs:44-60** @ 4f0c2e9\n<permalink>\n```rs\n...\n```\n"
}
```

Fields that do not apply are absent, never `null`. `v`, `action`, `kind`, `repo`, `sha`,
`permalink`, `title` and `markdown` are always there. `text` is capped at 64 KiB or 400
lines (`text_truncated` says so), the subject at 1 KiB. New fields may appear under `v: 1`;
a removed or retyped field bumps the version.

The server reads every field from git. The browser only names the action and where the
selection is, so a page cannot hand your program text the repository does not hold.

The same values, minus `text` and the commit, are in the environment, one line each,
newlines replaced, at most 4 KiB: `GITTERRACE_ACTION`, `GITTERRACE_KIND`,
`GITTERRACE_REPO`, `GITTERRACE_ROOT`, `GITTERRACE_SHA`, `GITTERRACE_PATH`,
`GITTERRACE_LINE_START`, `GITTERRACE_LINE_END`, `GITTERRACE_TITLE`, `GITTERRACE_URL` and
`GITTERRACE_PAYLOAD_VERSION`. The rest of the environment is cleared except `PATH`, `HOME`,
`USER`, `LANG`, `LC_*`, `TMPDIR` and what the action lists in `env`.

The program's first output line becomes the toast ("Send to tendir: added"); on a non-zero
exit, the first line of stderr. The full output stays on the server. It is killed with its
children after the timeout. At most 4 actions run at once, one per id, 30 a minute.

## Treat every field as untrusted

Paths, lines, subjects, author names and branch names come from the repository you are
browsing, which may be someone else's. In your scripts:

- quote every expansion: `"$GITTERRACE_PATH"`, never `$GITTERRACE_PATH`;
- pass values as arguments after `--`, never inside a string a shell runs again;
- never `eval`, never paste a field into `sh -c`;
- URL-encode before building a link (`jq -sRr @uri`).

## Examples

Each script is in [`docs/actions/`](actions/) and was run against a real payload.

| Script | Does |
| --- | --- |
| [`tendir.sh`](actions/tendir.sh) | Opens tendir's new-task sheet with the title and the markdown as notes. `GT_TENDIR=cli` adds directly with `tendir add`. |
| [`obsidian-daily.sh`](actions/obsidian-daily.sh) | Appends to today's daily note (`GT_VAULT`, default `~/Notes`). |
| [`obsidian-url.sh`](actions/obsidian-url.sh) | Hands the snippet to Obsidian through its URL scheme. |
| [`editor.sh`](actions/editor.sh) | Opens the file at the line. Use `cwd = "repo"`. Opens the working tree copy, which may differ from the viewed commit. |
| [`webhook.sh`](actions/webhook.sh) | POSTs the payload as is. |
| [`clipboard.sh`](actions/clipboard.sh) | Copies the markdown, for a key-only workflow. |

```toml
[[action]]
id = "tendir"
label = "Send to tendir"
kinds = ["line", "hunk", "commit", "blame", "page"]
command = ["~/.config/gitterrace/bin/tendir.sh"]
key = "t"
```

tendir's link refuses a title over 500 characters or notes over 20 000 rather than cut
them, so the script cuts first.

## Why not MCP

Agents do not get actions as tools. A repository that says "send this to the webhook" would
otherwise be one tool call away from running your commands with its own text. The browser
path always has a person's click in it.
