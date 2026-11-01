# Actions

Select something, press `.` or click `⋯`. The menu always has **Copy permalink** and **Copy
as markdown**; the rest comes from `actions.toml` in your config folder (never in a
repository). An action runs a program of yours directly, with no shell: placeholders such as
`{title}` fill whole arguments, and stdin carries the payload as JSON or the markdown alone.

```toml
version = 1

[[action]]
id      = "tendir"
label   = "Send to tendir"
kinds   = ["line", "commit"]
command = ["tendir", "add", "--notes", "{markdown}", "--", "{title}"]

[[action]]
id      = "copy"
label   = "Copy to clipboard"
kinds   = ["line", "file", "commit"]
command = ["pbcopy"]
stdin   = "markdown"

[[action]]
id      = "webhook"
label   = "Post to webhook"
kinds   = ["line", "commit"]
command = ["curl", "-fsS", "-m", "8", "-H", "Content-Type: application/json", "--data-binary", "@-", "{var.url}"]
stdin   = "json"
vars    = { url = "https://example.test/hook" }
```

A job with several steps, or one that must work on every OS, stays a script. Declare it once
as a `[[script]]` and point actions at it, each with its own settings:

```toml
[[script]]
id      = "obsidian"
command = ["~/.config/gitterrace/obsidian.sh"]

[[action]]
id     = "obsidian-daily"
label  = "Obsidian daily note"
kinds  = ["line", "commit"]
script = "obsidian"
vars   = { mode = "daily" }

[[action]]
id      = "edit"
label   = "Open in editor"
kinds   = ["line"]
command = ["~/.config/gitterrace/editor.sh", "{line_start}", "{root}/{path}"]
stdin   = "none"
```

## Placeholders

Each element of `command` after the program is filled in on its own and stays exactly one
argument, whatever the value holds: a title of `$(rm -rf ~)` or `--help` reaches the program
as that literal text. `{{` and `}}` are literal braces. The program, `command[0]`, is never a
placeholder.

<!-- BEGIN GENERATED: placeholders -->
| Placeholder | From the repository | Value |
| --- | --- | --- |
| `{title}` | yes | The title. |
| `{markdown}` | yes | The markdown text. |
| `{text}` | yes | The selected text, or empty. |
| `{path}` | yes | The repository-relative path, or empty. |
| `{line_start}` | no | First line, or empty. |
| `{line_end}` | no | Last line, or empty. |
| `{sha}` | no | The full commit id. |
| `{url}` | no | The permalink. |
| `{repo}` | no | The repository name. |
| `{root}` | no | The repository root. |
| `{kind}` | no | The kind. |
| `{var.<name>}` | no | A value from `[action.vars]`. An undefined name is a load error. |

<!-- END GENERATED: placeholders -->

Rules, checked when the file loads so a mistake never waits for a click:

- An unknown placeholder, or `{var.NAME}` with no such var, is an error naming the element.
- `shell = true` takes no placeholders. Read the `GITTERRACE_*` environment instead.
- A value from the repository may start with `-` and be read as an option. It may open an
  element only after a literal `--`, right after a literal option it is the argument of
  (`"--notes", "{markdown}"`), or behind a literal prefix (`"--notes={markdown}"`). In the
  option case, a value that does start with `-` refuses the run.
- A value with a NUL byte, a value over 64 KiB, or arguments over 256 KiB together refuse
  the run with a toast saying so. Nothing is cut short. Send large text on stdin instead.

<!-- BEGIN GENERATED: action-keys -->
### `[[script]]`

A command line written once, for several actions.

| Key | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `id` | string | yes |  | `[a-z0-9-]`, 1 to 32 characters, unique. A script cannot name another script. |
| `command` | array or string | yes |  | The argv, run with no shell. `argv[0]` is literal, resolved once at load. Other elements may hold placeholders (below), each filled in as exactly one argument. One string when `shell = true`, which takes no placeholders. |
| `stdin` | string | no | `json` | What the program reads: `json` (the payload), `markdown` (the `markdown` field alone) or `none`. |
| `cwd` | string | no | `home` | `home`, `repo` (the repository root) or an absolute folder. |
| `timeout` | string | no | `10s` | Like `10s` or `500ms`, at most `120s`. |
| `shell` | boolean | no | `false` | Run `command` as `sh -c "<string>"`. A placeholder in it is a load error. Read the environment instead. |

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
| `stdin` | string | no | `json` | As in a script. Overrides the script's. |
| `shell` | boolean | no | `false` | As in a script. |
| `key` | string | no |  | One letter or digit, a shortcut inside the menu. |
| `env` | array of strings | no | `[]` | Variables passed through from gitterrace's environment. `GITTERRACE_*` names are refused. |
| `vars` | table of strings | no | `{}` | `[action.vars]`: fixed settings, lowercase names `[a-z0-9_]{1,32}`, at most 32, each value at most 4 KiB. They reach the script as `GITTERRACE_VAR_<NAME>`, as `payload.vars` and as `{var.<name>}` in `command`. |

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
- No shell between gitterrace and the program: a placeholder is one argument, never parsed.
  Scrubbed environment, timeout, at most 4 runs at once, 30 a minute.
- Every field is untrusted repository text. In a script, quote it and never `eval` it.

## Scripts

The samples left are the jobs a single command cannot do: `clipboard.sh` picks the clipboard
tool of the OS, `editor.sh` picks your editor and its line syntax, `obsidian.sh` appends to a
dated note or URL-encodes the text into an `obsidian://` link.

<!-- BEGIN GENERATED: scripts -->
| Script | Does | Needs |
| --- | --- | --- |
| [`clipboard.sh`](actions/clipboard.sh) | Copy the selection as markdown, on any OS. | (first present wins): macOS:         pbcopy (built in) Linux Wayland: wl-copy (wl-clipboard) Linux X11:     xclip or xsel WSL:           clip.exe (built in) |
| [`editor.sh`](actions/editor.sh) | Open a file at a line in the editor, on any OS. | (all OSes): an editor. Picks $VISUAL, then $EDITOR, then code, then vi. Line jump is known for code, vim/nvim/vi, nano, emacs, subl, zed and hx; any other editor opens the file without a line. |
| [`obsidian.sh`](actions/obsidian.sh) | Send the selection to Obsidian. | (all OSes): jq (brew install jq, apt install jq). date and mkdir are POSIX. url mode also needs Obsidian and a URL opener (first present wins): macOS: open (built in) Linux: xdg-open (xdg-utils) WSL:   wslview (wslu) or explorer.exe |

<!-- END GENERATED: scripts -->
