# Using it

## Serving many repositories

Give several paths, or a folder that holds repositories:

```sh
git terrace ~/src/app ~/src/lib
git terrace ~/src
```

A folder is searched `multi.depth` levels down (2 by default), skipping hidden
folders and `multi.skip_dirs`. Each repository lives under `/r/<name>/`, and `/`
lists them all. `git terrace` in another repository joins the running server.
`git terrace remove` takes one off. `features.picker_enabled = false` hides the
switcher and the list.

The box above the list filters names, ignoring case, and counts the matches.
`/` jumps to it, Enter opens the first match, the arrows walk the matches,
Escape clears. The filter is kept in the address as `?q=`, and without
JavaScript the Filter button sends it to the server.

## The commit log and its graph

The log has three tabs: List, Graph and This ref. Graph draws every local
branch and the HEAD of each linked worktree, as `git log --graph --all` does.
Each tip's labels sit at the left, joined to its dot by a dashed line, and a
detached worktree is labeled "worktree" and its id. This ref narrows it to the
ref in the address. Lines past the column cap fold as "+N". A filtered log
has no graph. A public server leaves out worktree HEADs, and
branches of separate clones are never drawn (nothing is fetched).

When the server serves several clones of one project, the toolbar's "Related:"
links the log of each one sharing a root commit. A shallow clone without the
root commit is not related.

## Filtering commits

One field in the log's head filters the commits: `author:ann path:src !merge
login` lists Ann's non-merge commits under `src` whose message says "login".
Enter applies it, with or without JavaScript, and the address keeps it as
`?q=`. Each term shows as a chip whose cross removes that term alone.

Terms side by side must all match, `!` inverts one, and a comma means any of
its values: `type:fix,feat`. Quote a value with a space or comma. Words without
a keyword search the message. `|`, parentheses and `committer:` are not
supported; the field says so and keeps the text. The insights pages read the
same filter, except message words. Older `?path=`, `?author=`, `?type=`,
`?scope=` and `?period=` links still work. With the index ready no keyword
reads the history; before it, the log stops after the newest 2,000 commits.

<!-- BEGIN GENERATED: log-filter -->
| Syntax | Matches |
| --- | --- |
| `type:<type>[,<type>]` | Conventional type; `other` is none. |
| `scope:<scope>[,<scope>]` | Conventional scope; `none` is none. |
| `author:<who>[,<who>]` | E-mail in any case, or the exact name. |
| `path:<prefix>[,<prefix>]` | Touches a file under the prefix. |
| `after:<period\|YYYY-MM-DD>` | Within the period, or from the date. |
| `before:<period\|YYYY-MM-DD>` | Before the period, or the date. |
| `period:<1w\|1m\|3m\|1y\|all>` | Old form of `after:`; `all` is none. |
| `merge` | Merge commits. |
| `<word> or "<words>"` | The message has it, any case. |
| `!<term>` | The term does not match. |
| `<term> <term>` | All terms match; `a,b` is either. |

A period is `1w`, `1m`, `3m` or `1y`, counted back from the newest commit.

<!-- END GENERATED: log-filter -->

## The outline of a page

A rendered Markdown file with three or more headings gets an "On this page"
list (a rail on a wide screen, a menu on a narrow one) of its h2 and h3
headings, and h1 when there are several. A source file
lists its functions, types, classes and methods, each linked to its line, in
any language whose highlighting names its declarations (Java, C, C++, C#,
Kotlin, Ruby, PHP, Scala, Elixir, Lua, shell and about 80 more). In a large
file the list covers the part already highlighted.

## Languages

The languages page counts lines per language along the first-parent history. On
a large merge a renamed and edited file may count as a delete plus an add.
The net lines stay the same.

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

On a local server, Settings is the last entry of the Actions menu (`.`, then
`,`), at `/_settings`. It edits `config.toml` and `actions.toml`, as a form or
as text, and checks a value before it saves. A running server picks up the
change within a second. It is never on a public server or in an export.
`features.settings_enabled = false` removes it.

[Configuration](configuration.md) lists every key, and
[actions](actions.md) explains `actions.toml`.
