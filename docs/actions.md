# Actions

Select lines, a hunk, a blame line, a search hit, a hotspot row, a commit or a page, then
press `.` or click `⋯`. The menu always has **Copy permalink** (`/go/<repo-id>/<sha>/<path>`,
stable across clones and renames) and **Copy as markdown**. Anything else comes from your
`actions.toml`, which runs a program of yours with the selection as JSON on stdin.

## Configuration

`actions.toml` lives in gitterrace's config folder (`~/.config/gitterrace`, `$XDG_CONFIG_HOME`,
or `--config DIR`), never in a repository.

```toml
version = 1

[[script]]
id      = "obsidian"
command = ["~/.config/gitterrace/obsidian.sh"]

[[action]]
id     = "obsidian-daily"
label  = "Obsidian daily note"
kinds  = ["line", "hunk", "commit"]
script = "obsidian"
[action.vars]
mode  = "daily"
vault = "~/Notes"
```

<!-- BEGIN GENERATED: action-keys -->
### File

At the top of `actions.toml`.

| Key | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `version` | integer | yes |  | Must be `1`. |
| `script` | array of tables | no |  | `[[script]]` blocks: a command line written once for several actions. |
| `action` | array of tables | no |  | `[[action]]` blocks: what the menu offers. |

### `[[script]]`

A command line written once, for several actions.

| Key | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `id` | string | yes |  | `[a-z0-9-]`, 1 to 32 characters, unique. A script cannot name another script. |
| `command` | array or string | yes |  | The argv, no templating; `argv[0]` is resolved once at load. One string when `shell = true`. |
| `cwd` | string | no | `home` | `home`, `repo` (the repository root) or an absolute folder. |
| `timeout` | string | no | `10s` | Like `10s` or `500ms`, at most `120s`. |
| `shell` | boolean | no | `false` | Run `command` as `sh -c "<string>"`. Nothing is substituted into it. |

### `[[action]]`

One menu entry. It runs either its own `command` or a `script`.

| Key | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `id` | string | yes |  | `[a-z0-9-]`, 1 to 32 characters, unique. |
| `label` | string | yes |  | Menu text, 1 to 80 characters. |
| `kinds` | array of strings | yes |  | Where the action is offered, from the kinds below. An unknown kind is an error. |
| `command` | array or string | no |  | As in a script. Set exactly one of `command` and `script`. |
| `script` | string | no |  | The `id` of a `[[script]]`. `shell` is not allowed beside it. |
| `cwd` | string | no | `home` | As in a script. Overrides the script's. |
| `timeout` | string | no | `10s` | As in a script. Overrides the script's. |
| `output` | string | no | `toast` | `toast` shows the first output line, `ignore` shows failures only. |
| `shell` | boolean | no | `false` | As in a script. |
| `key` | string | no |  | One letter or digit, a shortcut inside the menu. |
| `env` | array of strings | no | `[]` | Variables passed through from gitterrace's environment. `GITTERRACE_*` names are refused. |
| `vars` | table of strings | no | `{}` | `[action.vars]`: fixed settings, lowercase names `[a-z0-9_]{1,32}`, at most 32, each value at most 4 KiB. They reach the script as `GITTERRACE_VAR_<NAME>` and as `payload.vars`, and are never templated into `argv`. |

<!-- END GENERATED: action-keys -->

An unknown key or field is an error, so typos surface. With `shell = true` nothing is
substituted into the string: refer to the selection through quoted variables,
`"$GITTERRACE_PATH"`.

The file is refused as a whole (logged, the server runs without actions) unless it is a
regular file owned by you and not writable by group or others, in a folder with the same
properties, and outside every served repository. Edits are picked up within a second; an
invalid edit keeps the previous set. Going from no actions to some needs a restart.

Actions are off when gitterrace listens on anything but loopback, and `--no-actions` turns
them off for one run.

### Kinds

<!-- BEGIN GENERATED: kinds -->
| Kind | Selection |
| --- | --- |
| `line` | A line or a range of lines in a file. |
| `file` | A whole file. |
| `commit` | A commit. |
| `hunk` | One hunk of a diff. |
| `blame` | One line of a blame view. |
| `search-hit` | A code or commit search hit. |
| `insight` | A row of an insights page, such as a hotspot. |
| `page` | The page being shown. |

<!-- END GENERATED: kinds -->

## Payload (version 1)

Absent fields are never `null`. New fields may appear under `v: 1`; a removed or retyped one
bumps the version. The server reads every field from git, so a page cannot hand your program
text the repository does not hold.

<!-- BEGIN GENERATED: payload -->
| Field | Type | Always there | Description |
| --- | --- | --- | --- |
| `v` | integer | yes | Payload version, `1`. |
| `action` | string | yes | The action's `id`. |
| `kind` | string | yes | What was selected, one of the kinds below. |
| `repo` | object | yes | `name`, `root` (absolute path) and `id` (stable repository id, absent for a repository without commits). |
| `rev` | string | yes | The revision the page was showing, as named in the URL. |
| `sha` | string | yes | The full commit id `rev` resolves to. |
| `path` | string | no | Repository-relative path, when the selection is in a file. |
| `lines` | object | no | `start` and `end`, 1-based and inclusive. |
| `text` | string | no | The selected text, read from git. Capped at 64 KiB or 400 lines. |
| `text_truncated` | boolean | no | Set with `text`: true when a cap cut it. |
| `query` | string | no | The search query, for a search hit. |
| `permalink` | string | yes | A `/go/<repo-id>/<sha>/<path>#L<a>-L<b>` link that survives clones and renames. |
| `title` | string | yes | A short label such as `guard.rs:44-60 @ 4f0c2e9`. |
| `commit` | object | no | `subject`, `author` and `date` (RFC 3339) of `sha`. |
| `markdown` | string | yes | The same text the Copy as markdown built-in produces. |
| `vars` | object | yes | The action's `[action.vars]`, possibly empty. Set from `actions.toml` only. |

<!-- END GENERATED: payload -->

### Environment

Each payload value is also in the environment (one line, at most 4 KiB), plus
`GITTERRACE_VAR_<NAME>` per var. Everything else is cleared except `PATH`, `HOME`, `USER`,
`LANG`, `LC_*`, `TMPDIR` and what the action lists in `env`.

<!-- BEGIN GENERATED: env -->
| Variable | Value |
| --- | --- |
| `GITTERRACE_PAYLOAD_VERSION` | The payload version. |
| `GITTERRACE_ACTION` | The action's `id`. |
| `GITTERRACE_KIND` | The kind. |
| `GITTERRACE_REPO` | The repository name. |
| `GITTERRACE_ROOT` | The repository root. |
| `GITTERRACE_SHA` | The commit id. |
| `GITTERRACE_TITLE` | The title. |
| `GITTERRACE_URL` | The permalink. |
| `GITTERRACE_PATH` | The path, when there is one. |
| `GITTERRACE_LINE_START` | First line, when there is a range. |
| `GITTERRACE_LINE_END` | Last line, when there is a range. |

<!-- END GENERATED: env -->

## Security model

- The file is refused unless it is a regular file owned by you, not group or world writable,
  in a folder with the same properties, outside every served repository. Edits reload within
  a second; an invalid edit keeps the previous set.
- Off when listening beyond loopback, with `--no-actions`, and never offered to MCP: a
  repository that says "send this to the webhook" would otherwise be one tool call away from
  running your commands with its own text. The browser path always has a person's click.
- `command` is an argv with no templating. Vars are fixed text from your file.
- The program gets a scrubbed environment, a timeout (killed with its children), at most 4
  running at once, one per id, 30 a minute. The first output line becomes the toast.
- Treat every field as untrusted: paths, subjects and branch names come from the repository
  you are browsing. Quote every expansion, never `eval`, URL-encode before building a link.

## Examples

Scripts in [`docs/actions/`](actions/), exercised per OS by `make actions`. Settings come
from `[action.vars]`; each script's header lists them. Windows: run gitterrace under WSL;
native Windows is not supported (the scripts detect WSL).

<!-- BEGIN GENERATED: scripts -->
| Script | Does | Needs |
| --- | --- | --- |
| [`clipboard.sh`](actions/clipboard.sh) | Copy the selection as markdown. | (first present wins): macOS:         pbcopy (built in) Linux Wayland: wl-copy (wl-clipboard) Linux X11:     xclip or xsel WSL:           clip.exe (built in) Always needs jq (macOS: brew install jq; Debian/Ubuntu: apt install jq). |
| [`editor.sh`](actions/editor.sh) | Open the file at the selected line in the editor. | (all OSes): an editor. Picks $VISUAL, then $EDITOR, then code, then vi. Line jump is known for code, vim/nvim/vi, nano, emacs, subl, zed and hx; any other editor opens the file without a line. |
| [`obsidian.sh`](actions/obsidian.sh) | Send the selection to Obsidian. | (all OSes): jq (brew install jq, apt install jq). date and mkdir are POSIX. url mode also needs Obsidian and a URL opener (first present wins): macOS: open (built in) Linux: xdg-open (xdg-utils) WSL:   wslview (wslu) or explorer.exe |
| [`tendir.sh`](actions/tendir.sh) | Send to tendir. | : jq, plus macOS: open (built in) and the tendir app Linux: xdg-open (xdg-utils) with a tendir:// handler, or the tendir CLI WSL:   wslview (wslu) or explorer.exe, or the tendir CLI |
| [`webhook.sh`](actions/webhook.sh) | POST the payload as-is. | (all OSes): curl (brew install curl, apt install curl). |

<!-- END GENERATED: scripts -->
