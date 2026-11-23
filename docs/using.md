# Using it

The everyday tasks, one section each: review before a push, browse and compare,
find commits, share a line, work with an agent and change a setting.

## Review before you push

1. Run `git terrace` in the repository and open the address it prints.
2. Click **Working tree** in the header, or open `/worktree`. It shows each
   uncommitted file and its diff. The **Staged**, **Unstaged** and **All**
   tabs show what the next commit holds, what it leaves out, or both.
3. Select the lines a finding is about and press `.`.
4. Pick a **Send to** target, add a note and press Send.

Good to know: the chip shows only while the tree has changes. Without a
`[[send]]` target, **Copy as markdown** puts the note on the clipboard.
Mail draft is there from the start. `git terrace init` writes `actions.toml`
with more targets (issues, merge requests, your editor, a webhook). Switch one
on with `enabled = true`, and [Send to and actions](actions.md) covers the rest.

## Browse several repositories

One server holds all your repositories, each under `/r/<name>/`.

1. Pass several paths, or a folder that holds repositories:

   ```sh
   git terrace ~/src/app ~/src/lib
   git terrace ~/src
   ```

2. Open `/` for the list.
3. Press `/` to filter the list, then Enter to open the first match.

Good to know: a folder is searched `multi.depth` levels down (2 by default). It
skips hidden folders and `multi.skip_dirs`. `git terrace` in another repository
joins the running server, and `git terrace remove` takes one off. To keep one
server running from login (optional), see [Run it as a service](service.md).

## See branches and worktrees together

1. Open the log (`g l`).
2. Pick the **Graph** tab. It draws every local branch and the HEAD of each
   linked worktree, like `git log --graph --all`.
3. Pick **This ref** to keep only the ref in the address.

Good to know: a filtered log has no graph. A public server leaves out worktree
HEADs. When several clones of one project are served, **Related:** links the
log of each clone that shares a root commit.

## Compare a branch

1. Open a branch's log. **Since it left the default branch** opens the compare
   page from the commit the branch forked at, like a pull request.
2. On the compare page, **since fork** switches between that view (`A...B`)
   and every commit in B that A lacks (`A..B`).

From a terminal, `git terrace --ref main...feature` opens the same page.
`--print` prints the address instead.

## Find commits

1. Type terms into the field above the log, for example
   `author:ann path:src !merge login`.
2. Press Enter. The address keeps the filter as `?q=`.

Terms side by side must all match. `!` inverts one, and a comma means any of
its values. Quote a value that holds a space or comma. The insights pages read
the same filter, except message words.

<!-- BEGIN GENERATED: log-filter -->
| Syntax | Matches |
| --- | --- |
| `type:<type>[,<type>]` | Conventional type, `other` for none. |
| `scope:<scope>[,<scope>]` | Conventional scope, `none` for none. |
| `author:<who>[,<who>]` | E-mail in any case, or the exact name. |
| `path:<prefix>[,<prefix>]` | Touches a file under the prefix. |
| `after:<period\|YYYY-MM-DD>` | Within the period, or from the date. |
| `before:<period\|YYYY-MM-DD>` | Before the period, or the date. |
| `merge` | Merge commits. |
| `<word> or "<words>"` | The message has it, any case. |
| `!<term>` | The term does not match. |
| `<term> <term>` | All terms match, `a,b` is either. |

A period is `1w`, `1m`, `3m` or `1y`, counted back from the newest commit.

<!-- END GENERATED: log-filter -->

Good to know: `|`, parentheses and `committer:` are not supported. Until the
index is ready, the log stops after the newest 2,000 commits.

## Jump within a file

1. Open a Markdown file with three or more headings, or a source file.
2. Use **On this page** to jump to a heading, function, type or method.

Good to know: source outlines work in about 90 languages. In a large file the
list covers the part already highlighted.

## Mark files as viewed

Each file of a commit, compare or working tree diff has a **Viewed** box, and
a line above the diff counts the marked files. The marks live in this
browser's localStorage, per page address, and never leave the machine. A file
marked viewed whose diff changed since (a live refresh of the working tree, say)
is unchecked and shows a **Changed since viewed** badge until you mark it again.

## Find in the whole diff

The box above a commit's or the working tree's diff searches every changed
file on the server, folded and not yet loaded ones too, which the browser's own
find cannot see. It lists the files that hold the text with a few lines each,
and a click opens the file. Case is ignored.

## Share a link to a line

1. Click a line number. Shift-click another to select a range.
2. Copy the address, or press `y` for a permalink pinned to the commit.

## Work with a coding agent

Two ways, and you can use both:

- **Hand it a finding.** Mark lines, press `.` and pick **Ask Claude Code**,
  **Ask Codex**, **Ask Kiro** or **Ask Gemini**. The agent opens in a terminal
  in the repository with your note as its first prompt.
  [Hand a finding to an agent](actions.md#hand-a-finding-to-an-agent) switches
  them on.
- **Let it read the repository.** `git terrace mcp` is a read-only MCP server
  for your agent: history, blame, insights, a comparison of two revisions and
  your uncommitted changes. Register it once in your agent's MCP settings, with
  the command `git terrace mcp --served`. The
  [tools and prompts](api.md#mcp-stdio) are listed in the API docs, and
  [Agents and MCP](mcp.md) is a skill to copy for review, history and hotspots.

## Read the insights

Open insights with `g i`. The languages page counts lines along the
first-parent history, so a renamed and edited file may count as a delete plus
an add.

A range narrows the whole page, charts included. Pick a period, type
`after:` and `before:` dates, or click a bar on the commits chart. Every chart
then draws only those days, with its axis and buckets fitted to them. On the
languages page, drag across the chart to select a stretch of commits: the page
reloads with the chart drawn on just that stretch. The date axis starts at the
first selected commit and ends at the last.

## Change a setting

1. Press `.`, then `,` to open Settings on a local server.
2. Edit `config.toml` or `actions.toml`, as a form or as text.
3. Save. The value is checked first, and a running server picks it up within a
   second.

Good to know: Settings is never on a public server or in an export.
The actions menu (`.`) ends with the running version, small and muted.
Search works on the Settings page too: it looks through the first
repository served and stays off with `features.search_enabled = false`.
`features.settings_enabled = false` removes it. `display.diff_palette = "colorblind"` marks diff additions blue and
removals orange instead of green and red.
`display.has_diff_wrap`, `display.diff_tab_chars` and `display.diff_font_px` wrap diff
lines, set the tab width and the code font size in every diff.
[Configuration](configuration.md) lists every key.

## Keys

Press `?` on any page for the full list. The common ones:

| Key | Does |
| --- | --- |
| `/` | Search code and commits |
| `t` | Find a file by name |
| `b` | Switch branch or tag |
| `y` | Copy a permalink to this revision |
| `j`, `k` | Move through a list, `Enter` opens |
| `]`, `[` | Jump to the next or previous changed file in a diff |
| `g` then `c`, `l`, `b`, `i`, `s` | Code, log, branches, insights, search |
| `.` | The action menu for the selection |
