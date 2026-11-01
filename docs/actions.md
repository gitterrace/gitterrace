# Actions

Select something, press `.` or click `⋯`. The menu always has **Copy permalink** and **Copy
as markdown**; the rest comes from `actions.toml` in your config folder (never in a
repository), which runs a program of yours with the selection as JSON on stdin.

```toml
version = 1

[[script]]
id      = "obsidian"
command = ["~/.config/gitterrace/obsidian.sh"]

[[action]]
id     = "obsidian-daily"
label  = "Obsidian daily note"
kinds  = ["line", "commit"]
script = "obsidian"
vars   = { mode = "daily" }
```

<!-- BEGIN GENERATED: action-keys -->
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

Kinds:

<!-- BEGIN GENERATED: kinds -->
`line` (a line or a range of lines in a file), `file` (a whole file), `commit` (a commit), `hunk` (one hunk of a diff), `blame` (one line of a blame view), `search-hit` (a code or commit search hit), `insight` (a row of an insights page, such as a hotspot), `page` (the page being shown).

<!-- END GENERATED: kinds -->

## Payload (version 1)

Absent fields are never `null`. A removed or retyped field bumps the version.

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

Environment (same values, plus `GITTERRACE_VAR_<NAME>`):

<!-- BEGIN GENERATED: env -->
`GITTERRACE_PAYLOAD_VERSION`, `GITTERRACE_ACTION`, `GITTERRACE_KIND`, `GITTERRACE_REPO`, `GITTERRACE_ROOT`, `GITTERRACE_SHA`, `GITTERRACE_TITLE`, `GITTERRACE_URL`, `GITTERRACE_PATH`, `GITTERRACE_LINE_START`, `GITTERRACE_LINE_END`.

<!-- END GENERATED: env -->

## Security

- The file must be yours, private, outside every served repository. Off beyond loopback, with
  `--no-actions`, and never offered to MCP.
- No templating: `command` is an argv, vars are fixed text. Scrubbed environment, timeout, at
  most 4 runs at once, 30 a minute.
- Every field is untrusted repository text: quote it, never `eval` it.

## Examples

<!-- BEGIN GENERATED: scripts -->
| Script | Does | Needs |
| --- | --- | --- |
| [`clipboard.sh`](actions/clipboard.sh) | Copy the selection as markdown. | (first present wins): macOS:         pbcopy (built in) Linux Wayland: wl-copy (wl-clipboard) Linux X11:     xclip or xsel WSL:           clip.exe (built in) Always needs jq (macOS: brew install jq; Debian/Ubuntu: apt install jq). |
| [`editor.sh`](actions/editor.sh) | Open the file at the selected line in the editor. | (all OSes): an editor. Picks $VISUAL, then $EDITOR, then code, then vi. Line jump is known for code, vim/nvim/vi, nano, emacs, subl, zed and hx; any other editor opens the file without a line. |
| [`obsidian.sh`](actions/obsidian.sh) | Send the selection to Obsidian. | (all OSes): jq (brew install jq, apt install jq). date and mkdir are POSIX. url mode also needs Obsidian and a URL opener (first present wins): macOS: open (built in) Linux: xdg-open (xdg-utils) WSL:   wslview (wslu) or explorer.exe |
| [`tendir.sh`](actions/tendir.sh) | Send to tendir. | : jq, plus macOS: open (built in) and the tendir app Linux: xdg-open (xdg-utils) with a tendir:// handler, or the tendir CLI WSL:   wslview (wslu) or explorer.exe, or the tendir CLI |
| [`webhook.sh`](actions/webhook.sh) | POST the payload as-is. | (all OSes): curl (brew install curl, apt install curl). |

<!-- END GENERATED: scripts -->
