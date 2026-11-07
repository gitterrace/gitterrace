# How numbers are counted

Every insights page, the API and MCP count a commit by one rule: `Scope::admits`
in `gitterrace-index/src/insights/scope.rs`. A commit counts when it is not a
merge, its author counts under the bot option, the selection (type, scope,
author, path) keeps it, and its local day lies in the period. Each page reads
one `Counted` per request, so two numbers on one page never disagree about which
commits they cover.

## Merges

Merges are left out of every insights number by default. A merge's diff against
its first parent repeats the lines its branch already brought, so counting it
would count those lines twice. The overview and the sidebar are the exception:
they show every commit, merges included, and the overview says how many of them
are merges.

The option `merges=1` in the URL (the "Merges" toggle on the options line,
`merges` in the API) counts merges as commits. It is off by default and goes
through the one membership rule, `Scope::admits`, so every section of every page
follows it together. A merge only ever adds to commit counts: its lines and
files are always zero, with the option on or off, and a path selection never
keeps a merge.

## Bots

A commit by a bot (an author whose name or e-mail marks a bot, such as
`dependabot[bot]`) counts only with `insights.include_bots` or `bots=1` in the
URL. The option is the same on every page and in every list, contributors and
the people of the types page included.

## People

Authors are people after the repository's `.mailmap`: two e-mails mapped to one
name count as one person, on every page. Ranked lists of people with equal
values order them by display name, then e-mail, never by the order the index
first met them.

## Days

A commit's day is its local day: the date its author saw, in the author's own
offset. A commit at 23:30 in Berlin on a Monday counts on Monday, whatever day
it was in UTC. The bars, the punch card, the weekday split and every total use
this same day.

One exception, named: the commit log groups commits by the day they landed, the
committer's local day, so its order and its day headings agree. A commit written
on Monday at 23:30 in Berlin and committed on Tuesday sits under Tuesday in the
log and counts on Monday in the bars. Every count still uses the author's day.

## Periods

A period is the last 7, 30, 90 or 365 local days, ending on the local day of the
repository's newest commit. A selection never moves that end: "last week" with
an author chip is the repository's last week, so it can be empty for that
author. `from` and `to` narrow a period to local days, both inclusive.

## All

`all` is every commit, whatever the clock says. A commit dated after the
server's time (a skewed clock on the machine that made it) still counts, at most
a day ahead of the clock that indexed it, and the same history always gives the
same totals.

## Code age and releases

Code age and release cadence are views over the whole history, whatever the
period. A file's age is its last counted change, so a merge or a commit the
selection leaves out never makes it younger. The commits between two releases
are the counted ones.

## Binary files

A binary file changes with 0 lines added or removed. It still counts as a
changed file and as a change on the hotspots page.

## Renames

Ownership and the per-file pages follow a renamed file across its earlier names,
as `git log --follow` does. Hotspots, the ownership totals over all time and the
"Files changed" count still count each name on its own, so a renamed file can
show up as two files there.
