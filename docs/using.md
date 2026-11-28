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
`[[send]]` target, **Copy as markdown** puts the note on the clipboard. If the
clipboard refuses, the window stays open with your draft and shows the error.
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

Good to know: a folder is searched `multi.depth` levels down. It
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

A commit and a compare page share a **Scope** switch in the page head: **This
commit**, **Branch vs default branch** and **Working tree**. An entry shows
only when its page is served. The working tree is left out on a public server
or when its feature is off, and the branch entry when HEAD is detached or
compare is off.

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
index is ready, the log shows only the newest commits
(`limits.unindexed_commits`).

## Jump within a file

1. Open a Markdown file with three or more headings, or a source file.
2. Use **On this page** to jump to a heading, function, type or method.

Good to know: source outlines work in every
[language](architecture.md#syntax-highlighting) the code view highlights. In a
large file the list covers the part already highlighted.

## File rail

On a commit or working tree page at least 1100 pixels wide, a rail of the
changed files stays beside the diff while you scroll. The file in view is
highlighted, a file you ticked **Viewed** carries a check, and a click jumps to
that file. Narrower windows keep the changed-files card instead. The links are
plain anchors, so they work without scripts.

## Checkpoints

A checkpoint records the uncommitted work as it is now, so a later review can
show only what changed since.

1. Open **Working tree** and click **Checkpoint**. A **Since checkpoint** tab
   appears.
2. Edit more files, then open the tab. It lists only the files whose content
   differs from the checkpoint. The files it already holds are folded into a
   count you can open.
3. Tick **Viewed** on a file as you read it, or click **Mark all viewed**. A
   mark holds while the file's patch is the same: edit the file again and it
   reads **Changed since viewed**.

With two or more checkpoints the checkpoint page has a **Since** and an
**Until** list. Pick a later checkpoint as **Until** to see what changed
between the two, without the working tree as it is now.

The Checkpoint button and the viewed marks write only the cache folder
`.git/gitterrace`. Your repository and working tree are never changed. Marks
survive a reload and show in a second tab. Marks an older version kept in the
browser move to the cache folder the first time you open the page.
Without a writable cache folder the buttons are replaced by a note and viewed
marks stay in the browser.

`GET /api/v1/worktree?since=<id>` (and the
`worktree_diff` tool) lists the files whose content differs from it. `since`
also takes a duration from `1m` to `7d`, such as `15m`: the newest checkpoint
taken at or before that long ago, or a 404 `checkpoint_not_found` that names
the age of the oldest one kept. Add `until=<id>` to compare two checkpoints
instead of the checkpoint and the working tree. A file
whose copy was not kept (binary, over `review.checkpoint_file_kb`, or dropped
by `review.checkpoint_mb`) is diffed against HEAD and named in `content_gone`.
Set `review.has_auto_checkpoint` to take one before Mark all viewed.
Checkpoints live in `.git/gitterrace/checkpoints`, hold only changed text
files that git does not ignore, and are not rebuildable: deleting the folder
forgets them. The buttons are on local servers only, a public server leaves
them out.

## Review order

The Working tree and Compare pages open with a **Review order** panel: the
changed files, riskiest first. Each row links to its diff and has a **Why**
list with the terms of its risk ([listed for agents](mcp.md#review-a-change))
and the plain facts behind them. Lockfiles, generated, binary and oversized
files are under **Not scored**. Press `]` and `[` to walk the diff files in
this order.
With more than one folder the panel groups its rows into **chapters**, each
with a heading in the same list. Files group by folder (a package under
`crates/`, `packages/` and the like, else the top folder), the highest risk
first. Chapters of only tests or only
docs come after the code ones and the unscored files close as one skim chapter.
The grouping is only an order, it never changes a file's risk. A **Why** list
also names risk signals, each with its evidence: a test removed, a source file
changed while the test next to it was not, a manifest or CI file changed, and a
large deletion. Signals sit beside the risk and are not added to it.
The panel is the same plan that `GET /api/v1/review_plan` and the `review_plan`
tool answer, and it needs the insights index.

## Review basket

The basket collects findings: a line, file, commit or hunk you flagged, each
with a note of your own. It lives in `.git/gitterrace/findings`, is capped by
`review.findings_entries` (oldest dropped) and is not rebuildable: deleting
the folder forgets it. `GET /api/v1/findings` and the
`list_findings` tool list it, oldest first, and say where the code of each
finding is now: `untouched`, `lines changed`, `lines edited`, `file changed` or `file gone`. A
status says what happened to the code, never whether the finding was dealt
with. Lines are looked up as the same text, so moved lines stay `untouched`
with their new range. Lines that were edited are followed through the diff and
reported as `lines edited` at the span they became, or `lines changed` when they
were all deleted. A consumer that matches the statuses must accept the new one.
When line anchoring is impossible (a binary file, a range
past the end) the answer is about the whole file. Both are refused on a public
server, and neither writes: adding and removing is a web action. Pass
`findings` to the `review_change` prompt to include the basket in the recipe.
An action of kind `review` hands the whole basket to a script as one payload.

In the browser the basket is the button in the top bar, with the number of
entries on it. Select lines (or a file, hunk or commit), press `.` and choose
**Add to review** to write a note. The button opens the panel: each entry shows
where its code is now as a chip (`untouched`, `lines changed`, `lines edited`,
`file changed` or `file gone`), with a Remove button, plus **Clear**. **Send all** runs one of
your actions of kind `review` with every entry in a single payload. The page
names the entries by id and the server reads the notes from its own basket, so
a page cannot send words of its own. Notes are plain text and shown as text.
In the file view a dot in the gutter marks the lines of an `untouched`
entry, where they are now even if they moved, and shows the note on hover.
Click it to open the panel. Only a file at HEAD or in the working tree is
marked, and an older revision shows just the entries made at it. Lines that
changed get no marker: the panel still lists them.
The button, the notes and Send all are on local servers only, a public server
leaves them out.

## Mark files as viewed

Each file of a commit, compare or working tree diff has a **Viewed** box, and
a line above the diff counts the marked files. The marks live in this
browser's localStorage, per page address, and never leave the machine. A file
marked viewed whose diff changed since (a live refresh of the working tree, say)
is unchecked and shows a **Changed since viewed** badge until you mark it again.

## Diffs open in full

A diff opens every file in full, except one over `display.diff_open_lines`
changed lines and the files past the page's budget. The bar above the diff then
says how many files are open and how many are folded, with an **Open all**
button that loads them all. A generated file stays folded until asked.

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

- **Hand it a finding.** Mark lines, press `.` and pick an **Ask** entry,
  one per [coding agent](actions.md#hand-a-finding-to-an-agent). The agent
  opens in a terminal in the repository with your note as its first prompt.
  [Hand a finding to an agent](actions.md#hand-a-finding-to-an-agent) switches
  them on.
- **Let it read the repository.** `git terrace mcp` is a read-only MCP server
  for your agent: history, blame, insights, a comparison of two revisions and
  your uncommitted changes. Register it once in your agent's MCP settings, with
  the command `git terrace mcp --served`. The
  [tools and prompts](api.md#mcp-stdio) are listed in the API docs, a permalink
  an action hands over is read back with `resolve_permalink`, and
  [Agents and MCP](mcp.md) is a skill to copy for review, history and hotspots.

A commit whose message ends with an `Agent-Session: <id>` trailer shows the
session and a `claude --resume <id>` command on its page. A `Co-authored-by:`
trailer that names a coding agent shows too. Only the message is read, so this
needs no setting and no file outside the repository. Matching a commit to the
agent's own session files is not done.

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

## See a file's history at a glance

A file's page, its history page and every folder listing fetch a small card
from the insights. The card for a file shows how often it changed and its
rank, who owns it, which files change with it and how old its lines are.
Files in the top tenth of the hotspots carry a Hot badge, also on the rows of
a folder. The badge and the card link to the hotspots page for that path.
While the index is still being built, or without insights, no card shows.

## Find knowledge that is at risk

The Ownership page ends with **Knowledge risk**. It lists the folders one level
down with their top owner, bus factor and when that owner last committed. A
folder is **At risk** when one person holds half of it and has not committed in
the 12 months before the newest commit. Click a folder to list what is in it,
and **Up one level** to go back. **Files without an active author** lists the
largest files whose top owner, and everyone with a tenth of its changes, has
been quiet for a year.

Click an owner to open their page, `/stats/ownership/{author}`: the files they
own most, how much of the code that is, when they last committed, and the
folders where they are the top owner. Ownership is by lines changed, as on the
Ownership table, not by who wrote the lines that are left. An unknown name is a
404. `GET /api/v1/insights/knowledge_risk` and the `insight` tool answer the
same, with `path` to drill, `author` for one person and `limit`.

## Change a setting

1. Press `.`, then `,` to open Settings on a local server.
2. Edit `config.toml` or `actions.toml`, as a form or as text.
3. Save. The value is checked first, and a running server picks it up within a
   second.

Good to know: Settings is never on a public server or in an export.
The actions menu (`.`) ends with the running version, small and muted.
Search works on the Settings page too: it looks through the first
repository served. Search, Settings, the diff palette, wrapping, tab width and
font size each have a key: [Configuration](configuration.md) lists them.

## Keys

Press `?` on any page for the full list. The common ones:

<!-- BEGIN GENERATED: keys -->
| Key | Does |
| --- | --- |
| `t` | Find a file by name |
| `/` | Search code and commits |
| `y` | Copy a permalink to this revision |
| `g` then `c` | Go to the code |
| `g` then `i` | Go to insights |
| `g` then `b` | Go to branches and tags |
| `g` then `l` | Go to the log |
| `g` then `s` | Go to search |
| `b` | Switch branch or tag |
| `.` | The action menu for the selection |
| `j`, `k` | Move through a list, `Enter` opens |
| `]`, `[` | Jump to the next or previous changed file in a diff |

<!-- END GENERATED: keys -->
