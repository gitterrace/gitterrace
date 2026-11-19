# Using it

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
joins the running server, and `git terrace remove` takes one off.

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

## Share a link to a line

1. Click a line number. Shift-click another to select a range.
2. Copy the address, or press `y` for a permalink pinned to the commit.

## Read the insights

Open insights with `g i`. The languages page counts lines along the
first-parent history, so a renamed and edited file may count as a delete plus
an add.

## Change a setting

1. Press `.`, then `,` to open Settings on a local server.
2. Edit `config.toml` or `actions.toml`, as a form or as text.
3. Save. The value is checked first, and a running server picks it up within a
   second.

Good to know: Settings is never on a public server or in an export.
`features.settings_enabled = false` removes it. [Configuration](configuration.md)
lists every key.

## Keys

Press `?` on any page for the full list. The common ones:

| Key | Does |
| --- | --- |
| `/` | Search code and commits |
| `t` | Find a file by name |
| `b` | Switch branch or tag |
| `y` | Copy a permalink to this revision |
| `j`, `k` | Move through a list, `Enter` opens |
| `g` then `c`, `l`, `b`, `i`, `s` | Code, log, branches, insights, search |
| `.` | The action menu for the selection |
