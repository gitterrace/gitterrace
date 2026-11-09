# Using it

What the pages offer once `git terrace` runs, and how to reach each part.

## Serving many repositories

One server shows many repositories: one address, a switcher between them, and
links that stay valid as others join.

Give several paths, or a folder that holds repositories:

```sh
git terrace ~/src/app ~/src/lib
git terrace ~/src
```

A folder is searched `multi.depth` levels down (2 by default). Hidden folders
and the names in `multi.skip_dirs` are skipped. `/` then lists every served
repository with its last commit and recent activity, and each one lives under
`/r/<name>/`.

The box above that list filters it by repository name. It keeps the names that
contain what you type, ignoring case, and shows how many of the total match.
Press `/` anywhere on the page to jump to the box. Enter opens the first match,
the arrow keys walk the matches and Escape clears the box. The filter is kept
in the address as `?q=`, so a filtered list can be bookmarked, and it works
without JavaScript too: the Filter button sends the same query to the server.

Run `git terrace` in another repository while a server runs and it joins that
server instead of opening a second port. The switcher in the page header lists
them all. `git terrace remove` takes one off again.
`features.picker_enabled = false` hides the switcher and the list.

## The commit log and its graph

The log of a branch has a List and a Graph switch in its head. Graph draws the
branches and merges beside each commit, one row per commit. A log filtered by
path or author has no graph, since its commits are not one line of history.

Graph draws every branch line at once, as `git log --graph --all` does: the
browsed ref, each local branch and the HEAD of each linked worktree, newest
first by commit date, each line in its own color from the theme. Each tip's
labels sit at the left of the graph, joined to the tip's dot by a dashed line in
its column's color. A detached worktree is labelled "worktree" and its id. This
ref narrows the graph to the history of the ref in the address, as before. Lines
past the column cap fold as "+N", and paging works the same in both. Worktree
HEADs are working state of this machine, so a public server leaves them out, as
it does the worktree pages. Branches of separate clones are not drawn:
gitterrace never fetches into a browsed repo.

When the server serves several clones or forks of one project, the log's
toolbar says "Related:" and links the log of each other served repository that
shares a root commit with this one. Only repositories the server already lists
take part, by their served name. A shallow clone that lacks the root commit is
not related until it is deepened.

## The outline of a page

A rendered Markdown file with three or more headings gets an "On this page"
list: a rail beside the text on a wide screen, a menu on a narrow one. It lists
the h2 and h3 headings, and h1 only when the file has several. Type in its
field to filter the headings.

A source file in Rust, Go, Python, JavaScript or TypeScript gets the same list
of its symbols: functions, types, classes and methods, each linked to its line.

## Languages

The languages page counts lines per language along the first-parent history. On
a large merge (more than 30 by 30 added and deleted files left to pair, the same
cut as git's `diff.renameLimit`), a renamed file that was also edited counts as a
delete plus an add in the added and removed lines. The net lines stay the same.

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

## Settings

On a local server the Actions menu (`.`, or the Actions button of a page) ends
with Settings, and `,` picks it in the menu. The page is at `/_settings`. It
edits `config.toml` and `actions.toml`, as a form or as plain text, and checks a
value before it saves. A running server picks up the change within a second.
The page is never on a public server or in an export.
`features.settings_enabled = false` removes it.

[Configuration](configuration.md) lists every key, and
[actions](actions.md) explains `actions.toml`.
