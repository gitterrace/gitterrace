# Send to and actions

Select something, press `.` or click `⋯`. The menu always has **Copy permalink**
and **Copy as markdown**. The rest comes from `actions.toml` in your config
folder (never in a repository), in two kinds: **Send to** for handing a
selection to another app, **actions** for running a program of yours.

## Send to

A `[[send]]` target is a URL template. The menu opens a small window with the
title and notes filled in. You edit them and press Send, and the browser opens
the destination's own add screen, where you confirm. gitterrace starts no
process and makes no request, so it stays read-only and holds no token.

```toml
version = 1

[[send]]
id    = "issue"
label = "New issue"
url   = "https://github.com/{var.repo}/issues/new?title={title}&body={notes}"
vars  = { repo = "owner/name" }

[[send]]
id    = "notes"
label = "New note"
url   = "obsidian://new?vault={var.vault}&file=Inbox%2F{title}&content={notes}"
vars  = { vault = "Notes" }
```

Page values are always percent-encoded and can never reach the scheme or the
host: the text before the first placeholder must hold the scheme and end the
host. `https`, `mailto` and app schemes are allowed, `http` only for loopback.
With no `[[send]]` entry, the window offers Copy as markdown.

### What a note carries

Text marked with the mouse snaps to whole words. Marked code sends its lines.
Marked prose in rendered Markdown (a README, a `.md` preview) sends the source
lines it was rendered from, byte for byte, and the permalink names them as
`#La-Lb`. A mark inside a table quotes the whole table. A mark shorter than
about 20 characters brings one line on each side as context. The title is the
first line you type above the context, else the file name and the first words of
the mark.

The last line of every note, and of Copy as markdown, is for machines:

```text
source: <repo>@<sha>:<path>#L<a>-L<b>
```

`<sha>` is the full commit id. A commit has no `:<path>` part, a whole file no
`#L` part, and a single line is still `#L<a>-L<a>`. In the path `%` is written
`%25` and `#` is written `%23`, so the only `#` on the line starts the range.
The sha is the first `@` followed by hex and then `:` or the line end, so a
repository name holding `@` still parses.

### The starter file

`git terrace init` writes the file below to `actions.toml` in your config
folder, unless one exists. It is an ordinary `actions.toml`. An entry with
`enabled = false` is checked but not offered: switch on the ones you use and
fill in their `vars`.

<!-- BEGIN GENERATED: starter -->
| Entry | Kind | On | What it does | Needs in `vars` | Commit lookup |
| --- | --- | --- | --- | --- | --- |
| `tendir` | Send to | yes | Opens tendir's new-task sheet, filled in. Needs the tendir app. |  | no |
| `mail` | Send to | yes | A mail draft in your mail app. |  | no |
| `github-issue` | Send to | no | A new issue on github.com. `repo` is `owner/name`. | `repo` | yes |
| `github-discussion` | Send to | no | A new discussion on github.com. `repo` is `owner/name`, `category` the category slug. | `repo`, `category` | no |
| `gitlab-issue` | Send to | no | A new issue on gitlab.com. `project` is `group/name`, or `group/subgroup/name`. | `project` | yes |
| `gitlab-issue-self-hosted` | Send to | no | A new issue on a self-hosted instance of that host. `base` is the instance URL, `project` is `group/name`. | `base`, `project` | yes |
| `gitlab-merge-request` | Send to | no | A new merge request on gitlab.com from a fixed branch. `branch` is the source branch. The Send window has no branch field, so change `branch` in this file before you send. | `project`, `branch` | no |
| `forgejo-issue` | Send to | no | A new issue on a self-hosted forge. `base` is the instance URL, `repo` is `owner/name`. | `base`, `repo` | yes |
| `copy` | action | no | Copies the selection as markdown on any system. Needs `clipboard.sh` from docs/actions in your config folder. |  | no |
| `edit` | action | no | Opens the file at the selected line in your editor. Needs `editor.sh` from docs/actions. |  | no |
| `tendir-add` | action | no | Adds a task with the tendir command line tool. |  | no |
| `webhook` | action | no | Posts the payload as JSON to a webhook of yours. Set `url` in `vars`. | `url` | no |
| `obsidian-new` | action | no | Writes a new note in an Obsidian vault. `vault` and `folder` pick where it lands. | `vault`, `folder` | no |

The whole file, as `git terrace init` writes it:
[defaults/actions.toml](../crates/gitterrace-actions/defaults/actions.toml).

<!-- END GENERATED: starter -->

### Reference

<!-- BEGIN GENERATED: send -->

| Placeholder | Value |
| --- | --- |
| `{title}` | The note's first line, or the selection's name. |
| `{notes}` | The rest of the note, the quoted selection and the permalink. |
| `{permalink}` | The permalink, in `links.style`. |
| `{path}` | The repository-relative path, or empty. |
| `{lines}` | The line range, like `L3-L7`, or empty. |
| `{sha}` | The full commit id. |
| `{repo}` | The repository name. |
| `{var.<name>}` | A value from `[send.vars]`, inserted verbatim, so it may hold the base URL. |

Refused schemes: `javascript:`, `data:`, `file:`, `vbscript:`, `blob:`,
`about:`, `chrome:`, `view-source:`.

### `[[send]]`

One Send to target: a `url` template.

| Key | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `id` | string | yes |  | `[a-z0-9-]`, 1 to 32 characters, unique among sends. |
| `label` | string | yes |  | Text in the Send window, 1 to 80 characters. |
| `url` | string | yes |  | The URL template. The scheme and host are literal. Placeholders (below) fill the path, query or fragment, each percent-encoded. |
| `lookup` | string | no |  | A URL template that searches the destination for a commit. The commit page links to it as "Find this commit in ...". Nothing is fetched. Same scheme rules as `url`. Only `{sha}` (full id), `{short}` (first 7 characters) and `{var.<name>}` are allowed. |
| `enabled` | boolean | no | `true` | `false` checks the block but does not offer it. The starter file ships the ones that need your settings this way. |
| `vars` | table of strings | no | `{}` | `[send.vars]`: settings, inserted verbatim as `{var.<name>}`, one line each, at most 4 KiB. |

<!-- END GENERATED: send -->

## Actions

An action runs a program of yours directly, with no shell: placeholders such as
`{title}` fill whole arguments, and stdin carries the payload as JSON or the
markdown alone. Use one when the job is a write with no add screen, a script, or
a webhook.

```toml
[[action]]
id      = "webhook"
label   = "Post to webhook"
kinds   = ["line", "commit"]
command = ["curl", "-fsS", "-m", "8", "-H", "Content-Type: application/json", "--data-binary", "@-", "{var.url}"]
stdin   = "json"
vars    = { url = "https://example.test/hook" }
```

Obsidian's `obsidian://new` takes `vault`, `file` (a path in the vault, `/`
encoded as `%2F`), `content` and `append`, and opens the note it wrote. With
`append`, repeated sends add to one inbox note instead of a new note each time:

```toml
[[action]]
id      = "obsidian-new"
label   = "New Obsidian note"
kinds   = ["line", "commit"]
command = ["open", "obsidian://new?vault={var.vault:url}&file={var.folder:url}%2F{title:url}&content={markdown:url}"]
vars    = { vault = "Notes", folder = "Inbox" }

[[action]]
id      = "obsidian-inbox"
label   = "Obsidian inbox"
kinds   = ["line", "commit"]
command = ["open", "obsidian://new?vault={var.vault:url}&file={var.folder:url}%2Fgitterrace&append=true&content={markdown:url}"]
vars    = { vault = "Notes", folder = "Inbox" }
```

### Windows

gitterrace builds and is smoke-tested on native Windows (CI) and under Wine,
including a timeout that kills its child. Registry, ACL and process group
behavior are not exercised by Wine. Natively, a timeout ends the script with
`taskkill /T` (then `/F`) instead of a process group signal, `actions.toml` is
read from `%APPDATA%\gitterrace` without an owner or mode check (Windows has no
mode bits, and ACLs are not read), and `command[0]` must be an `.exe`, `.cmd`,
`.bat` or `.com`. The sample `.sh` scripts need a POSIX shell, so call a Windows
tool directly, for example `command = ["clip.exe"]` with `stdin = "markdown"`.

When the same action must work on every system, a script picks the tool. The
copy action with [`clipboard.sh`](actions/clipboard.sh) runs unchanged on macOS,
Linux (Wayland or X11) and WSL:

```toml
[[action]]
id      = "copy"
label   = "Copy to clipboard"
kinds   = ["line", "file", "commit"]
command = ["~/.config/gitterrace/clipboard.sh"]
stdin   = "markdown"
```

A job with several steps stays a script too. Declare it once as a `[[script]]`
and point actions at it, each with its own settings:

```toml
[[script]]
id      = "obsidian"
command = ["~/.config/gitterrace/obsidian.sh"]

[[action]]
id     = "obsidian-daily"
label  = "Obsidian daily note"
kinds  = ["line", "commit"]
script = "obsidian"
vars   = { folder = "Daily" }

[[action]]
id      = "edit"
label   = "Open in editor"
kinds   = ["line"]
command = ["~/.config/gitterrace/editor.sh", "{line_start}", "{root}/{path}"]
stdin   = "none"
```

## Placeholders

Each element of `command` after the program is filled in on its own and stays
exactly one argument, whatever the value holds: a title of `$(rm -rf ~)` or
`--help` reaches the program as that literal text. `{{` and `}}` are literal
braces. The program, `command[0]`, is never a placeholder.

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

| Modifier | Effect |
| --- | --- |
| `{name:url}` | Percent-encodes the value for a URL query value: letters, digits and `-._~` stay, every other byte (`&`, `=`, `?`, `#`, `/`, `+`, space, newline, non-ASCII) becomes `%XX`, so the value cannot leave its parameter. The caps count the encoded length. |

<!-- END GENERATED: placeholders -->

Rules, checked when the file loads so a mistake never waits for a click:

- An unknown placeholder, or `{var.NAME}` with no such var, is an error naming
  the element.
- An unknown modifier (`{title:bogus}`) is an error naming the element.
- `shell = true` takes no placeholders. Read the `GITTERRACE_*` environment
  instead.
- A value from the repository may start with `-` and be read as an option. It
  may open an element only after a literal `--`, right after a literal option it
  is the argument of (`"--notes", "{markdown}"`), or behind a literal prefix
  (`"--notes={markdown}"`). In the option case, a value that does start with `-`
  refuses the run.
- A value with a NUL byte, a value over 64 KiB, or arguments over 256 KiB
  together refuse the run with a toast saying so. Nothing is cut short. Send
  large text on stdin instead.

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
| `enabled` | boolean | no | `true` | `false` checks the block but does not offer it, and its program may be missing. The starter file ships the examples this way. |
| `kinds` | array of strings | yes |  | Where the action is offered, from the kinds below. An unknown kind is an error. |
| `command` | array or string | no |  | As in a script. Set exactly one of `command` and `script`. |
| `script` | string | no |  | The `id` of a `[[script]]`. `shell` is not allowed beside it. |
| `cwd` | string | no | `home` | As in a script. Overrides the script's. |
| `timeout` | string | no | `10s` | As in a script. Overrides the script's. |
| `output` | string | no | `toast` | `toast` shows the first output line, `ignore` shows failures only. |
| `stdin` | string | no | `json` | As in a script. Overrides the script's. |
| `shell` | boolean | no | `false` | As in a script. |
| `key` | string | no |  | One letter or digit: a shortcut inside the menu, and on the page press `a` then the key (`a n`). Not a menu key (`p`, `m`, `s`) or a page key (`g`, `t`, `y`, `b`, `j`, `k`), and unique among actions. Either is a load error. |
| `env` | array of strings | no | `[]` | Variables passed through from gitterrace's environment. `GITTERRACE_*` names are refused. |
| `vars` | table of strings | no | `{}` | `[action.vars]`: fixed settings, lowercase names `[a-z0-9_]{1,32}`, at most 32, each value at most 4 KiB. They reach the script as `GITTERRACE_VAR_<NAME>`, as `payload.vars` and as `{var.<name>}` in `command`. |

<!-- END GENERATED: action-keys -->

Kinds:

<!-- BEGIN GENERATED: kinds -->
`line` (a line or a range of lines in a file), `file` (a whole file), `commit`
(a commit), `hunk` (one hunk of a diff), `blame` (one line of a blame view),
`search-hit` (a code or commit search hit), `insight` (a row of an insights
page, such as a hotspot), `page` (the page being shown).

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
| `permalink` | string | yes | `gitterrace://<repo-id>/<sha>/<path>#L<a>-L<b>`, or the `/go` URL with `links.style = "go"`. Survives clones and renames. |
| `title` | string | yes | A short label such as `guard.rs:44-60 @ 4f0c2e9`. |
| `commit` | object | no | `subject`, `author` and `date` (RFC 3339) of `sha`. |
| `markdown` | string | yes | The same text the Copy as markdown built-in produces. |
| `vars` | object | yes | The action's `[action.vars]`, possibly empty. Set from `actions.toml` only. |

<!-- END GENERATED: payload -->

Environment (same values, plus `GITTERRACE_VAR_<NAME>`):

<!-- BEGIN GENERATED: env -->
`GITTERRACE_PAYLOAD_VERSION`, `GITTERRACE_ACTION`, `GITTERRACE_KIND`,
`GITTERRACE_REPO`, `GITTERRACE_ROOT`, `GITTERRACE_SHA`, `GITTERRACE_TITLE`,
`GITTERRACE_URL`, `GITTERRACE_PATH`, `GITTERRACE_LINE_START`,
`GITTERRACE_LINE_END`.

<!-- END GENERATED: env -->

## Security

- The file must be yours, private, outside every served repository. Off beyond
  loopback, with `--no-actions`, and never offered to MCP.
- No shell between gitterrace and the program: a placeholder is one argument,
  never parsed. Scrubbed environment, timeout, at most 4 runs at once, 30 a
  minute.
- A `timeout` is at most 120 s. That ceiling is fixed and no setting raises it.
  Each stream of output keeps its first `limits.action_output_kb` (default 64
  KiB) and drops the rest.
- Every field is untrusted repository text. In a script, quote it and never
  `eval` it.

## Scripts

The samples left are the jobs a single command cannot do: `clipboard.sh` picks
the clipboard tool of the OS, `editor.sh` picks your editor and its line syntax,
`obsidian.sh` appends to today's daily note, a write with no Obsidian screen of
its own.

<!-- BEGIN GENERATED: scripts -->
| Script | Does | Needs |
| --- | --- | --- |
| [`clipboard.sh`](actions/clipboard.sh) | Copy the selection as markdown, on any OS. | (first present wins): macOS:         pbcopy (built in) Linux Wayland: wl-copy (wl-clipboard) Linux X11:     xclip or xsel WSL:           clip.exe (built in) Windows:       a POSIX sh is needed. On native Windows use command = ["clip.exe"] instead |
| [`editor.sh`](actions/editor.sh) | Open a file at a line in the editor, on any OS. | (all OSes): an editor. Picks $VISUAL, then $EDITOR, then the first of code, zed, zeditor found, then vi. Line jump is known for code, vim/nvim/vi, nano, emacs, subl, zed/zeditor and hx. Any other editor opens the file without a line. |
| [`obsidian.sh`](actions/obsidian.sh) | Append the selection to today's Obsidian daily note. | (all OSes): jq (brew install jq, apt install jq). date and mkdir are POSIX. |

<!-- END GENERATED: scripts -->
