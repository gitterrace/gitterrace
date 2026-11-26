#!/usr/bin/env bash
# Sourced by selftest.sh.
echo "scope -- the guards apply inside the org tree and nowhere else"
# The most important cases in the file. The engine runs in EVERY session on the
# machine, so a guard that fires outside the org blocks unrelated work.
t ALLOW scope /tmp bash '{"command":"git add ."}' "staging outside the org"
t ALLOW scope "$HOME" bash '{"command":"sed -i s/a/b/ f"}' "sed -i outside the org"
t ALLOW scope /tmp write '{"file_path":"/tmp/docs/generated/api.md"}' "generated path outside the org" # lint-path-ok
# The org ROOT itself must be in scope. One harness resets cwd there between
# calls, so treating it as foreign silently disables every guard for that
# harness.
t BLOCK scope "$ORG" bash '{"command":"git add ."}' "staging AT THE ORG ROOT is in scope"
t BLOCK scope "$ORG" bash '{"command":"rm -rf /etc/zz-probe"}' "containment AT THE ORG ROOT is in scope"

echo
echo "containment -- destructive verbs stay inside the org tree"
t BLOCK containment "$REPO" bash '{"command":"rm -rf '"$HOME"'/Documents/zz-probe"}' "rm outside the org"
t BLOCK containment "$REPO" bash '{"command":"/bin/rm -rf '"$HOME"'/Documents/zz"}' "absolute-path verb"
# $HOME/${HOME} in the fixture JSON below are LITERAL, the guard itself must
# expand them (see 05-containment.sh).
# shellcheck disable=SC2016
t BLOCK containment "$REPO" bash '{"command":"rm -rf \"$HOME/Documents/zz\""}' "quoted \$HOME"
# shellcheck disable=SC2016
t BLOCK containment "$REPO" bash '{"command":"rm -rf ${HOME}/Documents/zz"}' "braced \${HOME}"
t BLOCK containment "$REPO" bash '{"command":"rm -rf ~/Documents/zz"}' "tilde"
t BLOCK containment "$REPO" bash '{"command":"sudo rm -rf /etc/zz"}' "sudo prefix"
t BLOCK containment "$REPO" bash '{"command":"env FOO=1 rm -rf /etc/zz"}' "env prefix"
t BLOCK containment "$REPO" bash '{"command":"mv /etc/hosts /tmp/x"}' "mv out of the org"
t BLOCK containment "$REPO" bash '{"command":"dd of=/etc/hosts"}' "dd of= names its destination as an operand"
t BLOCK containment "$REPO" bash '{"command":"dd if=/dev/zero of=/etc/hosts bs=1m"}' "dd of= behind other operands"
# ALLOW cases: over-reach is what gets a guard disabled.
t ALLOW containment "$REPO" bash '{"command":"rm -rf ../sibling/zz"}' "sibling repo in the org"
t ALLOW containment "$REPO" bash '{"command":"rm -rf /tmp/zz"}' "/tmp scratch"
t ALLOW containment "$REPO" bash '{"command":"rm -f target/debug/zz"}' "relative path in the repo"
t ALLOW containment "$REPO" bash '{"command":"cp '"$HOME"'/.zshrc /tmp/backup"}' "backup OUT to tmp: source is read"
t ALLOW containment "$REPO" bash '{"command":"cat '"$HOME"'/.zshrc"}' "a READ outside the org"
# if= is a READ, so an out-of-org source with an in-scratch destination is fine.
t ALLOW containment "$REPO" bash '{"command":"dd if=/etc/hosts of=/tmp/x"}' "dd if= out, of= to scratch"
# The verb as DATA inside a quoted argument, which is prose about the guard.
t ALLOW containment "$REPO" bash '{"command":"python3 -c '"'"'print(\"rm -rf ~/Documents/x\")'"'"'"}' "verb as data"

# The org boundary from a product cwd: another org's tree and $HOME are outside, every
# sibling clone of this org is inside. Spelled every way the boundary has been crossed.
# Another org beside this one, made for the case: its tree is outside.
OTHER_ORG="$SELFTEST_ROOT/other-org"
mkdir -p "$OTHER_ORG"
for v in "rm -rf $OTHER_ORG/zz/x" "mv $OTHER_ORG/zz /tmp/zz" "cp /tmp/zz $OTHER_ORG/zz" \
    "dd if=/dev/zero of=$OTHER_ORG/zz" "rm -rf $HOME/zz" "cd $OTHER_ORG && rm -rf zz" \
    "cd /etc && rm -f zz" "rm -rf ../../other-org/zz" "rm -rf ~/zz" "rm -rf \$HOME/zz" \
    "rm -f ~/.jcode/config.toml" "rm -rf ~/.jcode/sessions" "rm -rf ~/.jcode/scratch/../config.toml"; do
    t BLOCK containment "$REPO" bash "$(jq -cn --arg c "$v" '{command:$c}')" "out of the org: $v"
done
for v in "rm -rf $ORG/sibling/zz" "rm -rf ../sibling/zz" "mv ../sibling/zz ../other/zz" \
    "cp /etc/hosts $ORG/sibling/zz" "rm -rf \$TMPDIR/zz" "rm -rf /var/folders/zz/x" \
    "rm -rf ~/.jcode/scratch/zz" "rm -rf $HOME/.jcode/scratch/lane/x"; do
    t ALLOW containment "$REPO" bash "$(jq -cn --arg c "$v" '{command:$c}')" "inside the org or scratch: $v"
done
# The session's own scratch dir counts only when the adapter passes it, and never widens to
# $HOME: a scratch dir set to $HOME or one of its ancestors is ignored.
t ALLOW containment "$REPO" bash '{"command":"rm -rf /Volumes/scratch-zz/x"}' "the session scratch dir" GUARD_SCRATCH_DIR=/Volumes/scratch-zz
t BLOCK containment "$REPO" bash '{"command":"rm -rf '"$HOME"'/zz"}' "a scratch dir of \$HOME opens nothing" GUARD_SCRATCH_DIR="$HOME"
t BLOCK containment "$REPO" bash '{"command":"rm -rf /Users/zz"}' "a scratch dir above \$HOME opens nothing" GUARD_SCRATCH_DIR="${HOME%/*}"
# An inherited GUARD_SCRATCH_ROOTS cannot widen containment either: a root of $HOME, of /, or
# holding `..` is ignored, whoever set it.
t BLOCK containment "$REPO" bash '{"command":"rm -rf '"$HOME"'/zz"}' "GUARD_SCRATCH_ROOTS=\$HOME opens nothing" GUARD_SCRATCH_ROOTS="$HOME"
t BLOCK containment "$REPO" bash '{"command":"rm -rf /etc/zz"}' "GUARD_SCRATCH_ROOTS=/ opens nothing" GUARD_SCRATCH_ROOTS=/
t BLOCK containment "$REPO" bash '{"command":"rm -f '"$HOME"'/.jcode/config.toml"}' "a root with .. opens nothing" GUARD_SCRATCH_ROOTS="$HOME/.jcode/scratch/.."
# A heredoc fed to python is the interpreter's input, not shell: its file writes and the
# words inside are not commands. Only a body fed to a SHELL is re-read as code.
# Removing is a move to the Trash: the Trash is an mv DESTINATION, and nothing more.
t ALLOW containment "$REPO" bash '{"command":"mv '"$REPO"'/.wt/done '"$HOME"'/.Trash/done-0152"}' "mv a clone into the Trash"
t ALLOW containment "$REPO" bash '{"command":"mv .wt/done ~/.Trash/done"}' "mv into the Trash, tilde form"
t BLOCK containment "$REPO" bash '{"command":"rm -rf '"$HOME"'/.Trash/done"}' "rm in the Trash is still outside the org"
t BLOCK containment "$REPO" bash '{"command":"mv '"$HOME"'/.Trash/done '"$HOME"'/Documents/x"}' "the Trash is no mv source"
t BLOCK containment "$REPO" bash '{"command":"mv '"$HOME"'/Documents/x '"$HOME"'/.Trash/x"}' "trashing a file outside the org"
t BLOCK containment "$REPO" bash '{"command":"cp .wt/done '"$HOME"'/.Trash/x"}' "cp into the Trash is a write outside the org"
# Each command is judged on its own: two Trash moves in one line, each to its own destination.
_tr="~/.Trash"
_out="$HOME/Docu""ments"
_v="mv"
_mv2="$_v .wt/a $_tr/a; $_v .wt/b $_tr/b"
t ALLOW containment "$REPO" bash '{"command":"'"$_mv2"'"}' "two mv into the Trash in one command"
t ALLOW containment "$REPO" bash '{"command":"'"${_mv2/;/ \&\&}"'"}' "two mv into the Trash joined by &&"
t BLOCK containment "$REPO" bash '{"command":"'"$_v .wt/a $_tr/a; $_v .wt/b $_out/b"'"}' "a Trash move does not cover a second mv out of the org"
t BLOCK containment "$REPO" bash '{"command":"'"$_v .wt/a $_tr/a; r""m -rf $_tr/a"'"}' "a Trash move does not cover an rm in the Trash"
t ALLOW containment "$REPO" bash '{"command":"python3 - <<'"'"'PY'"'"'\nimport pathlib\npathlib.Path(\"notes.md\").write_text(\"rm -rf ~/x and git stash\")\nPY"}' "a python heredoc that writes a file"
t ALLOW destructive_class "$REPO" bash '{"command":"python3 - <<'"'"'PY'"'"'\nopen(\"n.md\",\"w\").write(\"never git stash or sed -i\")\nPY"}' "a python heredoc naming the class as data"
t BLOCK destructive_class "$REPO" bash '{"command":"bash <<'"'"'SH'"'"'\ngit stash\nSH"}' "the same body fed to a shell is code"

echo
echo "shared checkout -- staging, stashing, commit messages"
t BLOCK git_add_scope "$REPO" bash '{"command":"git add ."}' "git add ."
t BLOCK git_add_scope "$REPO" bash '{"command":"git add ./"}' "git add ./"
t BLOCK git_add_scope "$REPO" bash '{"command":"git add -A"}' "git add -A"
t BLOCK private_index_copy "$REPO" bash '{"command":"cp .git/index .git/index-l10n"}' "a private index copied from the shared one"
t ALLOW private_index_copy "$REPO" bash '{"command":"GIT_INDEX_FILE=.git/index-l10n git read-tree HEAD"}' "a private index built from HEAD"
# A newline separates commands like `;`: each guard must still see the second one.
t BLOCK git_add_scope "$REPO" bash '{"command":"cd /tmp\ngit add ."}' "git add . after a newline"
t BLOCK git_stash "$REPO" bash '{"command":"cd /tmp\ngit stash push --keep-index"}' "stash after a newline"
t BLOCK destructive_class "$REPO" bash '{"command":"cd /tmp\nsed -i s/a/b/ f"}' "sed -i after a newline"
t BLOCK git_checkout_discards "$REPO" bash '{"command":"true\ngit checkout -- x"}' "checkout after a newline"
t BLOCK git_add_scope "$REPO" bash '{"command":"git add --all"}' "git add --all"
t BLOCK git_add_scope "$REPO" bash '{"command":"GIT_DIR=x git add ."}' "env-prefixed staging"
t BLOCK git_add_scope "$REPO" bash '{"command":"git -C ../sibling add ."}' "staging a sibling repo"
t ALLOW git_add_scope "$REPO" bash '{"command":"GIT_INDEX_FILE=.git/index-k git add a.txt b.txt"}' "explicit paths"
t BLOCK shared_index_staging "$REPO" bash '{"command":"git add x"}' "git add x writes the shared index"
t BLOCK shared_index_staging "$REPO" bash '{"command":"git rm x"}' "git rm x"
t BLOCK shared_index_staging "$REPO" bash '{"command":"git mv a b"}' "git mv a b"
t BLOCK shared_index_staging "$REPO" bash '{"command":"git add -A"}' "git add -A"
t BLOCK shared_index_staging "$REPO" bash '{"command":"git -C x add y"}' "git -C x add y"
t ALLOW shared_index_staging "$REPO" bash '{"command":"GIT_INDEX_FILE=.git/index-k git add x"}' "a private index"
t ALLOW shared_index_staging "$REPO/.wt/k" bash '{"command":"git add x"}' "a .wt/ worktree has its own index"
t ALLOW shared_index_staging "$REPO" bash '{"command":"git commit -m \"git add x\""}' "prose naming git add"
t ALLOW shared_index_staging "$REPO" bash '{"command":"git addx"}' "a different subcommand"
t BLOCK commit_plumbing "$REPO" bash '{"command":"git commit-tree abc -p HEAD -m x"}' "commit-tree skips every hook"
t BLOCK commit_plumbing "$REPO" bash '{"command":"git update-ref refs/heads/main abc"}' "update-ref moves a branch without hooks"
t BLOCK commit_plumbing "$REPO" bash '{"command":"git update-ref HEAD abc def"}' "update-ref HEAD"
t BLOCK commit_plumbing "$REPO" bash '{"command":"git -C x commit-tree t -m y"}' "commit-tree via -C"
t ALLOW commit_plumbing "$REPO" bash '{"command":"git commit -m x"}' "an ordinary commit runs the hooks"
t ALLOW commit_plumbing "$REPO" bash '{"command":"git update-ref refs/tags/v1 abc"}' "a tag ref is not a branch commit"
t ALLOW commit_plumbing "$REPO" bash '{"command":"git commit -m \"git commit-tree\""}' "prose naming commit-tree"
t ALLOW git_add_scope "$REPO" bash '{"command":"git status"}' "unrelated git command"

t BLOCK git_stash "$REPO" bash '{"command":"git stash"}' "bare stash"
t BLOCK git_stash "$REPO" bash '{"command":"git stash push -m wip"}' "stash push"
t BLOCK git_stash "$REPO" bash '{"command":"git -C /repo stash pop"}' "stash pop via -C"
# Reading the stash list is how you FIND stranded work; blocking it would punish
# the recovery rather than the mistake.
t ALLOW git_stash "$REPO" bash '{"command":"git stash list"}' "stash list"
t ALLOW git_stash "$REPO" bash '{"command":"git stash show -p"}' "stash show"

# Discarding a PATH BLOCKS, by owner ruling. An advisory is a message you can read
# and walk past, and AGENTS.md forbids the command outright, so the written rule and
# the mechanical rule must agree. The harm: another lane's uncommitted edits destroyed
# silently; afterwards the files simply read as unmodified.
t BLOCK git_checkout_discards "$REPO" bash '{"command":"git checkout -- crates/core/src/lib.rs"}' "checkout discards a path"
t BLOCK git_checkout_discards "$REPO" bash '{"command":"git restore crates/core/src/lib.rs"}' "restore discards a path"
# The form with no `--`, just a path, must match too.
t BLOCK git_checkout_discards "$REPO" bash '{"command":"git checkout crates/core/src/lib.rs"}' "checkout <path> without dashes"
# Switching branches is not discarding a path. The slash and dot cases are here
# because a "contains a slash" discriminator would block ordinary branch names.
# A bare path is judged against the request's cwd, not the engine's own (the checkout root
# here): `src/lib.rs` exists only below the crate the request runs in.
t BLOCK git_checkout_discards "$REPO/crates/core" bash '{"command":"git checkout src/lib.rs"}' "bare path resolved against the request cwd"
t ALLOW git_checkout_discards "$REPO" bash '{"command":"git checkout -b feature/x"}' "new branch"
t ALLOW git_checkout_discards "$REPO" bash '{"command":"git checkout main"}' "switch branch"
t ALLOW git_checkout_discards "$REPO" bash '{"command":"git checkout feature/my-work"}' "branch with a slash"
t ALLOW git_checkout_discards "$REPO" bash '{"command":"git checkout release/1.2"}' "branch with a dot"
t ALLOW git_checkout_discards "$REPO" bash '{"command":"git restore --staged Makefile"}' "unstage is not a discard"
# ...but `--staged --worktree` together DO discard the working tree, exactly as
# the equivalent `-SW` does: same effect, same verdict.
t BLOCK git_checkout_discards "$REPO" bash '{"command":"git restore --staged --worktree Makefile"}' "staged+worktree discards"
t BLOCK git_checkout_discards "$REPO" bash '{"command":"git restore --worktree --staged Makefile"}' "worktree+staged, either order"
t BLOCK git_checkout_discards "$REPO" bash '{"command":"git restore -SW Makefile"}' "short -SW form discards"
t ALLOW git_checkout_discards "$REPO" bash '{"command":"git status"}' "unrelated git command"

# A broad pattern kill matches the HUMAN's running app, not just the instance
# this session launched, while the agent already holds its own pid from launching
# its copy, so the wide pattern buys nothing.
#
# ADVISE rather than BLOCK: unlike a discard this destroys nothing on disk, and the
# legitimate anchored form below cannot be separated from the dangerous one by regex
# alone with enough confidence to refuse.
t ADVISE broad_process_kill "$REPO" bash '{"command":"pkill -f \"target/release/example-app\""}' "a pattern matching the owner's running app"
t ADVISE broad_process_kill "$REPO" bash '{"command":"killall example-app"}' "killall is equally broad"
# A legitimate anchored use. The leading `^` ties the match to a command line the
# script itself built, so it cannot reach another instance. If this case ever flips
# to ADVISE the rule is punishing correct work.
# The `$BIN` below is LITERAL fixture data: the guard scans the command
# STRING for a leading `^`, and must not expand anything.
# shellcheck disable=SC2016
t ALLOW broad_process_kill "$REPO" bash '{"command":"pkill -f \"^$BIN serve --transport http\""}' "anchored pattern is targeted"
t ALLOW broad_process_kill "$REPO" bash '{"command":"pkill -F /tmp/daemon.pid"}' "kill by pidfile is not a pattern"
t ALLOW broad_process_kill "$REPO" bash '{"command":"kill 96334"}' "kill by explicit pid"
t ALLOW broad_process_kill "$REPO" bash '{"command":"pgrep -af example-app"}' "pgrep only looks"

echo
echo "script content -- a destructive command hidden in a script the agent writes, then runs"
# The incident: a lane wrote .runs/hh-cycle.sh with a broad pkill on line 5, then ran
# `bash .runs/hh-cycle.sh`. The bash rules saw only the command line. Each script below is a
# fixture file the run cases name, and its body is the content the write cases send.
_sc_write() { printf '%s\n' "$2" >"$REPO/.runs/$1"; }
_sc_kill=$'#!/usr/bin/env bash\nset -u\ncd "$(dirname "$0")"\necho cycle\npkill -f "x.app/Contents/MacOS/x"\n'
_sc_stash=$'#!/usr/bin/env bash\necho saving\ngit stash push -m wip\n'
_sc_rm=$'#!/usr/bin/env bash\nrm -rf ~/Documents/zz\n'
_sc_discard=$'#!/usr/bin/env bash\ngit checkout -- README.md\n'
_sc_scoped=$'#!/usr/bin/env bash\napp &\npid=$!\nsleep 1\nkill "$pid"\n'
_sc_write sc-kill.sh "$_sc_kill"
_sc_write sc-stash.sh "$_sc_stash"
_sc_write sc-rm.sh "$_sc_rm"
_sc_write sc-discard.sh "$_sc_discard"
_sc_write sc-scoped.sh "$_sc_scoped"
_sc_write sc-stash.py $'import os\nos.system("echo hi")\n'
chmod +x "$REPO/.runs/sc-kill.sh"
_sc_in() { jq -cn --arg f "$REPO/.runs/$1" --arg c "$2" '{file_path:$f,content:$c}'; }
t ADVISE broad_process_kill "$REPO" bash '{"command":"bash .runs/sc-kill.sh"}' "bash <script> whose body has a broad pkill"
t ADVISE broad_process_kill "$REPO" bash '{"command":"./.runs/sc-kill.sh --fast"}' "an executable ./script whose body has a broad pkill"
t ADVISE broad_process_kill "$REPO" bash '{"command":"source .runs/sc-kill.sh"}' "source <script> whose body has a broad pkill"
t BLOCK git_stash "$REPO" bash '{"command":"bash .runs/sc-stash.sh"}' "bash <script> whose body runs git stash"
t BLOCK containment "$REPO" bash '{"command":"sh .runs/sc-rm.sh"}' "sh <script> whose body runs rm -rf out of the org"
t BLOCK git_checkout_discards "$REPO" bash '{"command":"zsh .runs/sc-discard.sh"}' "zsh <script> whose body discards a file"
t BLOCK destructive_class "$REPO" bash '{"command":"bash .runs/sc-stash.sh"}' "the destructive class sees a script body"
t ALLOW broad_process_kill "$REPO" bash '{"command":"bash .runs/sc-scoped.sh"}' "a script that kills its own pid"
t ALLOW git_stash "$REPO" bash '{"command":"python3 .runs/sc-stash.py"}' "a python script with no destructive command"
# A script near GUARD_SCRIPT_MAX with the pkill on its last line: the body is read whole and in
# time. Joining it line by line once cost 36 s at 256 KiB, past the hook's 4 s deadline.
{
    printf '#!/usr/bin/env bash\n'
    i=0
    while [ "$i" -lt 4000 ]; do
        printf 'echo line %s\n' "$i"
        i=$((i + 1))
    done
    printf 'pkill -f "x.app/Contents/MacOS/x"\n'
} >"$REPO/.runs/sc-big.sh"
# CPU time (user+sys, ms) of the waited-for children, from bash's `times`, into _sc_t. Scheduler
# waits under a parallel build are not CPU, so load cannot inflate the ratio. Not called through
# $(...): a subshell's `times` starts from zero.
_sc_now() {
    times >"$SELFTEST_ROOT/times"
    _sc_t=$(sed -n 2p "$SELFTEST_ROOT/times" | awk 'function f(x, m, r) { m = index(x, "m"); r = substr(x, m + 1); sub(/s$/, "", r); return substr(x, 1, m - 1) * 60 + r }
        { printf "%d", (f($1) + f($2)) * 1000 }')
}
# The cheaper of two calls per script, still, so one cold start cannot fail the case.
_sc_best() {
    local best="" a d label="$2"
    for _ in 1 2; do
        _sc_now
        a=$_sc_t
        t ADVISE broad_process_kill "$REPO" bash "{\"command\":\"bash .runs/$1\"}" "$label"
        _sc_now
        d=$((_sc_t - a))
        [ -z "$best" ] || [ "$d" -lt "$best" ] && best=$d
        label="$label (again)"
    done
    _sc_ms=$best
}
_sc_best sc-kill.sh "a small script with a broad pkill, the timing baseline"
_sc_small=$_sc_ms
_sc_best sc-big.sh "a script near the size cap with a broad pkill on its last line"
_sc_big=$_sc_ms
# A ratio of CPU time, not wall time: a loaded machine cannot inflate it. Linear reading keeps the big
# script within a few times the small one. The quadratic join ran it 20 times slower and more.
_assert yes "$([ "$_sc_big" -le $((_sc_small * 8 + 500)) ] && echo yes || echo no)" \
    "the script near the size cap costs at most 8 times a small one (${_sc_big} ms cpu vs ${_sc_small} ms cpu)"
t ADVISE script_content "$REPO" write "$(_sc_in hh-cycle.sh "$_sc_kill")" "writing a .sh with a broad pkill"
t BLOCK script_content "$REPO" write "$(_sc_in w.sh "$_sc_stash")" "writing a .sh that runs git stash"
t BLOCK script_content "$REPO" write "$(_sc_in w.zsh $'git reset --hard HEAD\n')" "writing a .zsh that runs git reset --hard"
t BLOCK script_content "$REPO" write "$(_sc_in w.bash "$_sc_rm")" "writing a .bash that runs rm -rf out of the org"
t BLOCK script_content "$REPO" edit "$(jq -cn --arg f "$REPO/Makefile" '{file_path:$f,old_string:"all:",new_string:"all:\n\tgit checkout -- README.md"}')" "an edit adding a discard to a Makefile"
t BLOCK script_content "$REPO" apply_patch "$(jq -cn '{patch_text:"*** Begin Patch\n*** Add File: tools/x.sh\n+#!/bin/sh\n+git stash\n*** End Patch"}')" "apply_patch adding git stash to a .sh"
t ALLOW script_content "$REPO" write "$(_sc_in ok.sh "$_sc_scoped")" "writing a script that kills its own pid"
t ALLOW script_content "$REPO" write "$(jq -cn --arg f "$REPO/docs/kill.md" '{file_path:$f,content:"Never run pkill -f on an app, and never git stash."}')" "prose naming pkill in a .md"
t ALLOW script_content "$REPO" apply_patch "$(jq -cn '{patch_text:"*** Begin Patch\n*** Add File: docs/x.md\n+git stash and pkill -f x\n*** End Patch"}')" "apply_patch adding prose to a .md"
# Unbounded work. The search cases are the walk that ran two hours; the wait cases are the
# `yes` burners that pinned every core because `wait` came before `kill`.
t BLOCK unbounded_search "$REPO" bash '{"command":"grep -rn cache_files '"$REPO"' -l"}' "recursive grep over the repo"
t BLOCK unbounded_search "$REPO" bash '{"command":"grep -rn foo '"$ORG"'"}' "recursive grep over the org"
t ALLOW unbounded_search "$REPO" bash '{"command":"grep -rn foo '"$REPO"'/crates"}' "absolute path inside a checkout is scoped"
t BLOCK unbounded_search "$REPO" bash '{"command":"grep -Rl foo ."}' "flag cluster holding R"
t BLOCK unbounded_search "$REPO" bash '{"command":"rg --no-ignore foo"}' "rg opting back into ignored trees"
t BLOCK unbounded_search "$REPO" bash '{"command":"find . -name x"}' "find at the repo root, unbounded"
t BLOCK unbounded_search "$REPO" bash '{"command":"find ~ -name x"}' "find at home"
t ALLOW unbounded_search "$REPO" bash '{"command":"grep -r --exclude-dir=target foo crates"}' "grep -r already bounded"
t ALLOW unbounded_search "$REPO" bash '{"command":"cat x | grep -c foo"}' "grep on stdin"
t ALLOW unbounded_search "$REPO" bash '{"command":"rg -n foo crates/"}' "rg respects .gitignore"
t ALLOW unbounded_search "$REPO" bash '{"command":"find .agents -name \"*.md\""}' "find in a scoped directory"
t ALLOW unbounded_search "$REPO" bash '{"command":"find . -maxdepth 2 -name x"}' "find bounded by depth"
t ALLOW unbounded_search "$REPO" bash '{"command":"git commit -m \"explain grep -r usage\""}' "a message mentioning grep -r"
# shellcheck disable=SC2016 # the JSON payload is a literal command under test
t BLOCK unbounded_wait "$REPO" bash '{"command":"for i in 1 2; do yes >/dev/null & B+=($!); done; sleep 1; wait; kill ${B[@]}"}' "wait before kill hangs on yes"
t BLOCK unbounded_wait "$REPO" bash '{"command":"tail -f log & sleep 2; wait"}' "bare wait on tail -f"
# shellcheck disable=SC2016 # the JSON payload is a literal command under test
t ALLOW unbounded_wait "$REPO" bash '{"command":"for i in 1 2; do yes >/dev/null & B+=($!); done; kill ${B[@]}; wait"}' "kill before wait is the fix"
# shellcheck disable=SC2016 # the JSON payload is a literal command under test
t ALLOW unbounded_wait "$REPO" bash '{"command":"cmd & p=$!; wait $p"}' "wait on a named pid"
t BLOCK live_desktop_input "$REPO" macos_computer_use '{"action":"run_applescript","script":"tell application \"System Events\"\n set p to first process whose unix id is 69709\n set frontmost of p to true\n keystroke \"z\" using command down\nend tell"}' "a script addressing a process by pid"
t BLOCK live_desktop_input "$REPO" macos_computer_use '{"action":"run_applescript","script":"tell application \"System Events\" to tell (first process whose unix id is 69709) to keystroke \"z\" using command down"}' "a keystroke addressed by pid"
t ALLOW live_desktop_input "$REPO" macos_computer_use '{"action":"run_applescript","script":"tell application \"System Events\" to tell (first process whose unix id is 33367) to click menu item 1 of menu 1 of menu bar item 4 of menu bar 1"}' "a menu click addressed to a pid"
t BLOCK live_desktop_input "$REPO" macos_computer_use '{"action":"click","x":812,"y":44}' "a click at a screen point"
t BLOCK live_desktop_input "$REPO" macos_computer_use '{"action":"drag","x":10,"y":10,"to_x":300,"to_y":300}' "a drag meant for the fixture lands in whatever is on top"
t BLOCK live_desktop_input "$REPO" macos_computer_use '{"action":"scroll","x":400,"y":400,"dx":0,"dy":-5}' "a scroll at a screen point"
t BLOCK live_desktop_input "$REPO" macos_computer_use '{"action":"move_window","app":"ExampleApp","x":0,"y":0}' "moving a window"
t BLOCK live_desktop_input "$REPO" macos_computer_use '{"action":"run_applescript","script":"tell application \"System Events\" to set position of window 1 of process \"ExampleApp\" to {0, 0}"}' "System Events moves a window"
t BLOCK live_desktop_input "$REPO" macos_computer_use '{"action":"run_applescript","script":"tell application \"System Events\" to click at {100, 200}"}' "System Events click at a position"
t ALLOW live_desktop_input "$REPO" macos_computer_use '{"action":"screenshot"}' "a screenshot"
t ALLOW live_desktop_input "$REPO" macos_computer_use '{"action":"window_screenshot","window_id":12}' "a window screenshot"
t ALLOW live_desktop_input "$REPO" macos_computer_use '{"action":"find_element","app":"ExampleApp","title":"Add"}' "reading the AX tree"
t ALLOW live_desktop_input "$REPO" macos_computer_use '{"action":"run_applescript","script":"tell application \"System Events\" to tell (first process whose unix id is 33367) to click menu item 1 of menu 1 of menu bar item 4 of menu bar 1"}' "a menu click addressed to a pid"
# The backtick in the fixture JSON below is LITERAL data the guard scans for,
# not a command substitution to expand.
# shellcheck disable=SC2016
t BLOCK git_commit_backticks "$REPO" bash '{"command":"git commit -m \"x `id` y\""}' "backticks in -m"
t ALLOW git_commit_backticks "$REPO" bash '{"command":"git commit -m \"plain\""}' "plain message"
t ALLOW git_commit_backticks "$REPO" bash '{"command":"git commit -F - <<MSG"}' "-F heredoc form"
# The engine appends every script the call runs to the command: a backtick in a script's
# docstring, on another line, is no part of the commit message.
t ALLOW git_commit_backticks "$REPO" bash '{"command":"git commit -q -m \"x\"\n\"\"\"Read `tools/a.sh`.\"\"\""}' "a backtick on another line"
t ALLOW git_commit_backticks "$REPO" bash '{"command":"git commit -m \"x\"; echo `date`"}' "a backtick after the commit"
t BLOCK git_commit_backticks "$REPO" bash '{"command":"echo hi\ngit commit -m \"x `id` y\""}' "backticks in -m on a later line"

# git commit -a ADVISES rather than blocks: same hazard class as `git add .`, but
# the false-positive cost differs and a guard that fires on ordinary intent gets
# routed around.
t ADVISE git_commit_all "$REPO" bash '{"command":"git commit -am wip"}' "commit -a advises"

# THE DECOY: the guards are installed in one clone and judge commands in another, so
# GUARD_REPO (where the engine lives) is not the checkout the agent works in. The decoy is
# a dirty repo standing in for the guards' clone: decoy.txt staged, then modified again.
# Every probe below points GUARD_REPO at it. A rule that reads GUARD_REPO's index instead
# of the cwd's speaks in the CLEAN repo and the "cwd, not GUARD_REPO" cases go red.
decoy_repo="$(cd "$REPO/.." && pwd)/.guard-decoy-probe.$$"
clean_repo="$(cd "$REPO/.." && pwd)/.guard-clean-probe.$$"
mkdir -p "$decoy_repo" "$clean_repo"
git -C "$decoy_repo" init -q 2>/dev/null
git -C "$clean_repo" init -q 2>/dev/null
printf 'staged\n' >"$decoy_repo/decoy.txt"
git -C "$decoy_repo" add decoy.txt 2>/dev/null
printf 'staged\nmodified after staging\n' >"$decoy_repo/decoy.txt"
_cwd_not_repo() { # $1 command  $2 text that must NOT appear  $3 label
    local got
    got=$(printf '{"event":"pre_tool","tool":"bash","cwd":"%s","status":"ok","input":{"command":"%s"}}' "$clean_repo" "$1" |
        env "${UNSET_ENV[@]}" GUARD_REPO="$decoy_repo" bash "$ENGINE" 2>&1 >/dev/null)
    CASES=$((CASES + 1))
    case "$got" in
        *"$2"*)
            echo "  FAIL cwd, not GUARD_REPO: $3 -- the rule read the guards' own clone"
            FAIL_LOG="${FAIL_LOG}  FAIL cwd, not GUARD_REPO: $3
"
            FAILED=1
            ;;
        *) echo "  ok   [ALLOW ] cwd, not GUARD_REPO: $3" ;;
    esac
}
_cwd_not_repo "git commit -F -" "takes the whole INDEX" "a clean cwd stays silent while the guards' clone has a staged file"
_cwd_not_repo "git commit decoy.txt -m x" "BOTH staged and further modified" "a pathspec hunk in the guards' clone is not the cwd's"

# git_commit_index_scope reads LIVE repo state (the index), so the harness
# pins GUARD_SELFTEST_NO_INDEX for every case and this one deliberately unpins
# it: without that pin, staging any file would flip unrelated ALLOW cases to
# ADVISE and the suite would measure the checkout instead of the guards.
# The case has to CREATE the condition it tests rather than assume it, so it
# builds a throwaway repo and stages a file in it. GUARD_REPO points at a DIFFERENT dirty
# repo (the decoy below), because the guards live in their own clone: a rule must read
# the index of the cwd's checkout, never the guards' own.
# The probe lives under the ORG root, not $TMPDIR: the engine abstains outright on
# any cwd outside GUARD_ORG, and on macOS $TMPDIR's realpath differs from the
# literal, so a probe there returns "allow" without consulting a rule.
index_probe="$(cd "$REPO/.." && pwd)/.guard-index-probe.$$"
mkdir -p "$index_probe"
git -C "$index_probe" init -q 2>/dev/null
: >"$index_probe/staged.txt"
git -C "$index_probe" add staged.txt 2>/dev/null
if [ -n "$(git -C "$index_probe" diff --cached --name-only 2>/dev/null)" ]; then
    got_index=$(printf '{"event":"pre_tool","tool":"bash","cwd":"%s","status":"ok","input":{"command":"git commit -F -"}}' "$index_probe" |
        env "${UNSET_ENV[@]}" GUARD_REPO="$decoy_repo" bash "$ENGINE" 2>&1 >/dev/null)
    case "$got_index" in
        *"takes the whole INDEX"*)
            echo "  ok   [ADVISE] git_commit_index_scope -- bare commit with a non-empty index"
            ;;
        *)
            echo "  FAIL expected ADVISE got nothing -- git_commit_index_scope on a non-empty index"
            FAILED=1
            ;;
    esac
    # The negative control, and the half that makes the case worth anything: the
    # SAME dirty index must stay silent once the commit carries a pathspec.
    got_ok=$(printf '{"event":"pre_tool","tool":"bash","cwd":"%s","status":"ok","input":{"command":"git commit -- staged.txt -F -"}}' "$index_probe" |
        env "${UNSET_ENV[@]}" GUARD_REPO="$decoy_repo" bash "$ENGINE" 2>&1 >/dev/null)
    case "$got_ok" in
        *"takes the whole INDEX"*)
            echo "  FAIL expected ALLOW got ADVISE -- a pathspec on the commit answers the question"
            FAILED=1
            ;;
        *) echo "  ok   [ALLOW ] git_commit_index_scope -- pathspec on the commit is silent" ;;
    esac
else
    echo "  FAIL could not stage a file in the probe repo -- the case cannot run, which is not the same as passing"
    FAILED=1
fi
rm -rf "$index_probe"
# This case is hand-rolled rather than a `t` line (it has to BUILD a dirty index
# in a throwaway repo), so it must declare its own coverage: `t` is what normally
# appends to COVERED, and a rule tested only here would otherwise be reported as
# untested by the check at the bottom of this file.
COVERED="$COVERED git_commit_index_scope "

# git_commit_pathspec_discards_hunk needs a file that is BOTH staged and further
# modified, which no `t` line can express, so it builds one the same way.
#
# The negative control is the half that matters: the SAME dirty repo must stay
# silent when the commit names a DIFFERENT file. Without it the case would pass
# on a rule that fired on every pathspec commit, which is the failure mode a
# guard most needs to avoid: one that cries wolf gets routed around.
hunk_probe="$(cd "$REPO/.." && pwd)/.guard-hunk-probe.$$"
mkdir -p "$hunk_probe"
git -C "$hunk_probe" init -q 2>/dev/null
printf 'staged\n' >"$hunk_probe/shared.txt"
printf 'staged\n' >"$hunk_probe/second.txt"
printf 'other\n' >"$hunk_probe/other.txt"
git -C "$hunk_probe" add shared.txt second.txt other.txt 2>/dev/null
printf 'staged\nmodified after staging\n' >"$hunk_probe/shared.txt"
printf 'staged\nmodified after staging\n' >"$hunk_probe/second.txt"
if [ -n "$(git -C "$hunk_probe" diff --name-only 2>/dev/null)" ]; then
    got_hunk=$(printf '{"event":"pre_tool","tool":"bash","cwd":"%s","status":"ok","input":{"command":"git commit shared.txt -m x"}}' "$hunk_probe" |
        env "${UNSET_ENV[@]}" GUARD_REPO="$decoy_repo" bash "$ENGINE" 2>&1 >/dev/null)
    case "$got_hunk" in
        *"BOTH staged and further modified"*)
            echo "  ok   [ADVISE] git_commit_pathspec_discards_hunk -- pathspec on a staged+modified file"
            ;;
        *)
            echo "  FAIL expected ADVISE got nothing -- git_commit_pathspec_discards_hunk"
            FAILED=1
            ;;
    esac
    got_quiet=$(printf '{"event":"pre_tool","tool":"bash","cwd":"%s","status":"ok","input":{"command":"git commit other.txt -m x"}}' "$hunk_probe" |
        env "${UNSET_ENV[@]}" GUARD_REPO="$decoy_repo" bash "$ENGINE" 2>&1 >/dev/null)
    case "$got_quiet" in
        *"BOTH staged and further modified"*)
            echo "  FAIL expected ALLOW got ADVISE -- a path that is only staged is not this bug"
            FAILED=1
            ;;
        *) echo "  ok   [ALLOW ] git_commit_pathspec_discards_hunk -- silent on a clean pathspec" ;;
    esac
    # Breaking out of the loop on the first hit would tell a two-path commit about
    # only one discarded path; fixing that one and re-running discards the other.
    #
    # Scoped to THIS advisory's own block with sed, not grepped over the whole
    # stderr: git_commit_index_scope prints the full staged list on the same
    # channel, so a bare `grep shared.txt` is satisfied by the SIBLING rule.
    got_multi=$(printf '{"event":"pre_tool","tool":"bash","cwd":"%s","status":"ok","input":{"command":"git commit shared.txt second.txt -m x"}}' "$hunk_probe" |
        env "${UNSET_ENV[@]}" GUARD_REPO="$decoy_repo" bash "$ENGINE" 2>&1 >/dev/null |
        sed -n '/BOTH staged and further modified/,/To keep the staged selection/p')
    if grep -q 'shared\.txt' <<<"$got_multi" &&
        grep -q 'second\.txt' <<<"$got_multi"; then
        echo "  ok   [ADVISE] git_commit_pathspec_discards_hunk -- names EVERY discarded path"
    else
        echo "  FAIL expected BOTH paths named -- git_commit_pathspec_discards_hunk stopped at the first"
        FAILED=1
    fi
else
    echo "  FAIL could not create a staged+modified file -- the case cannot run"
    FAILED=1
fi
rm -rf "$hunk_probe" "$decoy_repo" "$clean_repo"
COVERED="$COVERED git_commit_pathspec_discards_hunk "

# lane_git_write reads GUARD_LANE. Reads must stay open: blocking `git log`
# would punish diagnosing the very mistake.
_lw() { t "$2" lane_git_write "$REPO" bash '{"command":"'"$1"'"}' "$3" "${@:4}"; }
_lw "git commit -F -" BLOCK "a read-only lane may not commit" GUARD_LANE=ro
_lw "git log --oneline -5" ALLOW "reads stay open for a read-only lane" GUARD_LANE=ro
_lw "git diff HEAD" ALLOW "diff is a read" GUARD_LANE=ro
_lw "git commit -F -" ALLOW "a lane whose brief permits git is untouched" GUARD_LANE=rw
_lw "git commit -F -" ALLOW "no variable means coordinator" GUARD_LANE=
# The product's own lane variable (GUARD_LANE_ENV) reaches the rule as GUARD_LANE.
_lw "git commit -F -" BLOCK "the product's lane variable says ro" GUARD_LANE_ENV=EXAMPLE_LANE EXAMPLE_LANE=ro
_lw "git commit -F -" ALLOW "the product's lane variable says rw" GUARD_LANE_ENV=EXAMPLE_LANE EXAMPLE_LANE=rw
# The DELIVERY half: lanes share one host process, so the engine resolves a
# session id to a role from a coordinator-written file. The rule could be
# perfect and still never fire.
# Unique per run: a fixed path raced with a concurrent suite's `rm`.
_roles=$(mktemp "$_SCRATCH_PREFIX.roles.XXXXXX")
printf 'session_selftest_lane ro\nsession_selftest_coord rw\n' >"$_roles"
_lw "git commit -F -" BLOCK "role: a session listed ro is refused" GUARD_SESSION_ID=session_selftest_lane GUARD_LANE_ROLES="$_roles"
_lw "git commit -F -" ALLOW "role: a session listed rw is not" GUARD_SESSION_ID=session_selftest_coord GUARD_LANE_ROLES="$_roles"
_lw "git commit -F -" ALLOW "role: absent from the file means coordinator" GUARD_SESSION_ID=session_not_in_file GUARD_LANE_ROLES="$_roles"
_lw "git log -5" ALLOW "role: reads stay open for a resolved lane" GUARD_SESSION_ID=session_selftest_lane GUARD_LANE_ROLES="$_roles"
rm -f "$_roles"

# Feature lane: no full gate, no writes to the main checkout. An unset role changes nothing. The
# cases name their own gate pattern: a product's guards.env may not set GUARD_FULL_GATE_CMDS yet (the
# rule is opt-in) or may name another command, and the shared cases must judge the rule either way.
_fg() { t "$2" feature_lane_full_gate "$REPO" bash '{"command":"'"$1"'"}' "$3" GUARD_FULL_GATE_CMDS='make[[:space:]]+gate' "${@:4}"; }
_fg "make gate" BLOCK "feature lane: the full gate" GUARD_LANE=feature
_fg "cd /tmp && make gate" BLOCK "feature lane: the full gate after a cd" GUARD_LANE=feature
_fg "GATE_OK=1 make gate" ALLOW "feature lane: GATE_OK=1 forces the gate" GUARD_LANE=feature
_fg "make test-one" ALLOW "feature lane: targeted tests stay open" GUARD_LANE=feature
_fg "make gate" ALLOW "no role: the full gate is open" GUARD_LANE=
_fg "make gate" ALLOW "ro lane: not a feature lane" GUARD_LANE=ro
# A product whose lane variable has its own name: GUARD_LANE_ENV names it, and the engine reads it.
_fg "make gate" BLOCK "feature lane via the product's lane variable" GUARD_LANE= GUARD_LANE_ENV=SELFTEST_LANE SELFTEST_LANE=feature
_mc() { t "$2" feature_lane_main_checkout "$4" "$5" "$1" "$3" GUARD_LANE=feature GUARD_MAIN_CHECKOUT="$REPO" "${@:6}"; }
mkdir -p "$REPO/.wt/lane1" "$ORG/elsewhere"
ln -s "$REPO" "$ORG/elsewhere/tomain"
_mc '{"file_path":"'"$REPO"'/README.md","content":"x"}' BLOCK "feature lane: write in the main checkout" "$REPO" write
_mc '{"file_path":"'"$REPO"'/.wt/lane1/README.md","content":"x"}' ALLOW "feature lane: write in its lane clone" "$REPO" write
_mc '{"file_path":"'"$ORG"'/elsewhere/tomain/README.md","content":"x"}' BLOCK "feature lane: write through a symlink into main" "$REPO/.wt/lane1" write
_mc '{"file_path":"'"$ORG"'/sibling/f.txt","content":"x"}' ALLOW "feature lane: write outside main" "$REPO" write
_mc '{"command":"git commit -m x"}' BLOCK "feature lane: git commit with cwd main" "$REPO" bash
_mc '{"command":"git commit -m x"}' ALLOW "feature lane: git commit with cwd in its clone" "$REPO/.wt/lane1" bash
_mc '{"command":"git -C '"$REPO"' add x"}' BLOCK "feature lane: git -C main add" "$REPO/.wt/lane1" bash
_mc '{"command":"git -C '"$ORG"'/elsewhere/tomain add x"}' BLOCK "feature lane: git -C a symlink to main" "$REPO/.wt/lane1" bash
_mc '{"command":"git -C '"$REPO"'/.wt/lane1 add x"}' ALLOW "feature lane: git -C its clone" "$REPO" bash
_mc '{"command":"git log -3"}' ALLOW "feature lane: reads in main stay open" "$REPO" bash
_mc '{"file_path":"'"$REPO"'/README.md","content":"x"}' ALLOW "no role: the coordinator writes main" "$REPO" write GUARD_LANE=
rm -rf "$REPO/.wt" "$ORG/elsewhere"
COVERED="$COVERED feature_lane_full_gate feature_lane_main_checkout "

echo
echo "stream edits and protected files"
t BLOCK destructive_class "$REPO" bash '{"command":"sed -i '"''"' s/a/b/ f"}' "sed -i"
t BLOCK destructive_class "$REPO" bash '{"command":"sed --in-place s/a/b/ f"}' "sed --in-place"
t BLOCK destructive_class "$REPO" bash '{"command":"gsed -i s/a/b/ f"}' "gsed -i"
t BLOCK destructive_class "$REPO" bash '{"command":"gsed --in-place s/a/b/ f"}' "gsed --in-place"
t BLOCK destructive_class "$REPO" bash '{"command":"perl -pi -e s/a/b/ f"}' "perl -pi"
t ALLOW destructive_class "$REPO" bash '{"command":"sed -n 5p f"}' "read-only sed"
t ALLOW destructive_class "$REPO" bash '{"command":"grep -i pattern f"}' "grep -i is not sed -i"

t BLOCK protected_files "$REPO" write '{"file_path":"'"$REPO"'/.env"}' ".env"                                                   # lint-path-ok
t BLOCK protected_files "$REPO" write '{"file_path":"'"$REPO"'/.env.local"}' ".env.local"                                       # lint-path-ok
t BLOCK protected_files "$REPO" write '{"file_path":"'"$REPO"'/certs/zz.pem"}' "*.pem"                                          # lint-path-ok
t BLOCK protected_files "$REPO" write '{"file_path":"'"$REPO"'/zz.key"}' "*.key"                                                # lint-path-ok
t BLOCK protected_files "$REPO" write '{"file_path":"'"$REPO"'/docs/generated/api.md"}' "a generated doc (GUARD_PROTECTED_GLOBS)" GUARD_PROTECTED_GLOBS='*/docs/generated/* */vendor/* */tests/golden/*'  # lint-path-ok
t BLOCK protected_files "$REPO" edit '{"file_path":"'"$REPO"'/web/vendor/mermaid.min.js"}' "vendored code" GUARD_PROTECTED_GLOBS='*/docs/generated/* */vendor/* */tests/golden/*'
t BLOCK protected_files "$REPO" write '{"file_path":"'"$REPO"'/crates/core/tests/golden/revisions.txt"}' "a golden file" GUARD_PROTECTED_GLOBS='*/docs/generated/* */vendor/* */tests/golden/*'
t BLOCK protected_files "$REPO" edit '{"file_path":"'"$REPO"'/schwarzwald/kit/tokens/theme-tokens.json"}' "the synced kit copy"
t ALLOW protected_files "$REPO" edit '{"file_path":"'"$REPO"'/schwarzwald/config/guards.env"}' "the product's own kit config"
t BLOCK protected_files "$REPO" edit '{"file_path":"'"$REPO"'/docs/notes.md"}' "an install's own glob" GUARD_PROTECTED_GLOBS='*/docs/notes.md'  # lint-path-ok
t ALLOW protected_files "$REPO" edit '{"file_path":"'"$REPO"'/docs/generated/api.md"}' "an install that protects nothing" GUARD_PROTECTED_GLOBS=' '
t ALLOW protected_files "$REPO" edit '{"file_path":"'"$REPO"'/web/static/app.js"}' "a static file that is not vendored" GUARD_PROTECTED_GLOBS='*/docs/generated/* */vendor/* */tests/golden/*'
t ALLOW protected_files "$REPO" edit '{"file_path":"'"$REPO"'/docs/architecture.md"}' "a hand-written doc" GUARD_PROTECTED_GLOBS='*/docs/generated/* */vendor/* */tests/golden/*'
t ALLOW protected_files "$REPO" write '{"file_path":"'"$REPO"'/crates/core/src/lib.rs"}' "an ordinary source file"
# A filename merely CONTAINING "env" is not an env file.
t ALLOW protected_files "$REPO" write '{"file_path":"'"$REPO"'/crates/core/src/environment.rs"}' "environment.rs" # lint-path-ok

echo
echo "build and test workflow"
t BLOCK workspace_test "$REPO" bash '{"command":"cargo nextest run --workspace"}' "bare workspace nextest"
t BLOCK workspace_test "$REPO" bash '{"command":"cargo test --workspace"}' "bare workspace test"
t ALLOW workspace_test "$REPO" bash '{"command":"FULL_SUITE_OK=1 cargo test --workspace"}' "explicit override"
t ALLOW workspace_test "$REPO" bash '{"command":"cargo test --doc --workspace"}' "doc tests are fine"
t ALLOW workspace_test "$REPO" bash '{"command":"cargo test -p core"}' "scoped to one crate"
# PROSE inside a HEREDOC, not an invocation. Both directions of the same rule are
# already above; this is the third construct that had to be taught, after shell
# quotes and `guard_cmd_head`'s own truncation: `guard_cmd_unquoted` must be
# composed with `guard_cmd_head` to strip it.
t ALLOW workspace_test "$REPO" bash '{"command": "cat > /tmp/n.txt <<EOF\nnotes quoting cargo nextest run --workspace as prose\nEOF"}' "the flag named as prose in a heredoc"
# The SAME class in a QUOTED ARGUMENT, e.g. a `git commit -m "..."` whose
# value names the flag. The heredoc case above cannot stand in for it, because the
# two are stripped by different halves (`guard_cmd_head` truncates at `<<`,
# `guard_cmd_unquoted` removes quoted runs), so one can regress with the other green.
t ALLOW workspace_test "$REPO" bash '{"command":"echo \"we ran cargo test --workspace once\""}' "the flag named as prose in a quoted argument"

t BLOCK unquoted_include_glob "$REPO" bash '{"command":"grep -r --include=*.rs foo crates"}' "unquoted include glob"
t ALLOW unquoted_include_glob "$REPO" bash '{"command":"grep -r --include='"'"'*.rs'"'"' foo crates"}' "quoted include glob"

t BLOCK worktree_location "$REPO" bash '{"command":"git worktree add -b x /tmp/x"}' "worktree outside .wt/"
t BLOCK worktree_location "$REPO" bash '{"command":"git worktree add -b x .wt/x main"}' "a worktree even in .wt/: work goes in a clone"
t BLOCK worktree_location "$REPO" bash '{"command":"git -C /x worktree add --detach y"}' "worktree add behind git -C"
t ALLOW worktree_location "$REPO" bash '{"command":"git worktree list"}' "listing worktrees"
t ALLOW worktree_location "$REPO" bash '{"command":"git -C \"$repo\" worktree add -q --detach \"$wt\" \"$rev\""}' "a script's temporary worktree at a variable path"
t ALLOW worktree_location "$REPO" bash '{"command":"echo use clone.sh, not git worktree add"}' "the command named as prose"

t BLOCK clone_location "$REPO" bash '{"command":"git clone --no-local '"$REPO"' '"$HOME"'/.jcode/scratch/x"}' "a clone of an org repo outside its .wt/"
t BLOCK clone_location "$REPO" bash '{"command":"git clone '"$REPO"' '"$ORG"'/product-copy"}' "a clone as a sibling of the repo"
t BLOCK clone_location "$ORG" bash '{"command":"git clone product"}' "a clone with no destination lands in the cwd"
t BLOCK clone_location "$REPO" bash '{"command":"git clone . .wt/../x"}' "a destination that climbs out of .wt/"
t BLOCK clone_location "$REPO" bash '{"command":"git clone -q --no-local '"$REPO"'/.wt/a '"$REPO"'/.wt/../b"}' "a clone of a clone outside .wt/"
t ALLOW clone_location "$REPO" bash '{"command":"git clone --no-local '"$REPO"' '"$REPO"'/.wt/x"}' "a clone at <repo>/.wt/<label>"
t ALLOW clone_location "$REPO" bash '{"command":"git clone -q --no-local . .wt/x"}' "a relative clone into .wt/"
t ALLOW clone_location "$ORG" bash '{"command":"cd product && git clone -b main . .wt/y"}' "after a cd, with a branch flag"
t ALLOW clone_location "$REPO" bash '{"command":"git clone --depth 1 https://example.invalid/tap.git /tmp/tap"}' "a URL source is no lane clone"
t ALLOW clone_location "$REPO" bash '{"command":"git clone -q \"$clone\" \"$tmp/w\""}' "a script's temporary clone at a variable path"
t ALLOW clone_location "$REPO" bash '{"command":"git clone -q '"$ORG"'/../elsewhere /tmp/e"}' "a source outside the org"
t ALLOW clone_location "$REPO" bash '{"command":"git clone . y/x"}' "an install's own clone dir" GUARD_WORKTREE_DIR=y
t ALLOW clone_location "$REPO" bash '{"command":"git clone --no-local . '"$REPO"'/.wt/x"}' "a relative source, an absolute .wt destination"
t BLOCK clone_location "$ORG/sibling" bash '{"command":"git clone --no-local ../product '"$ORG"'/sibling/.wt/x"}' "a relative source from outside, into another repo's .wt"
t ALLOW clone_location "$ORG" bash '{"command":"cd product && git clone --no-local . '"$REPO"'/.wt/x"}' "a relative source after a cd"

# hookless_commit reads the checkout's hooks dir: one repo whose hooks are wired, one fresh
# clone with none, and one with no scripts/hooks at all (a repo that keeps no hooks).
_hk="$REPO/.runs/hooks-fixture"
mkdir -p "$_hk/wired/scripts/hooks" "$_hk/bare/scripts/hooks" "$_hk/plain"
for _d in wired bare plain; do git init -q "$_hk/$_d"; done
for _h in commit-msg pre-commit; do
    printf '#!/bin/sh\n' >"$_hk/wired/scripts/hooks/$_h"
    printf '#!/bin/sh\n' >"$_hk/bare/scripts/hooks/$_h"
    chmod +x "$_hk/wired/scripts/hooks/$_h" "$_hk/bare/scripts/hooks/$_h"
done
(cd "$_hk/wired" && git config core.hooksPath scripts/hooks)
t BLOCK hookless_commit "$_hk/bare" bash '{"command":"git commit x -m \"feat: x\""}' "a commit in a clone whose hooks are not installed"
t BLOCK hookless_commit "$REPO" bash '{"command":"git -C '"$_hk"'/bare commit -m x"}' "the same commit through git -C"
t ALLOW hookless_commit "$_hk/wired" bash '{"command":"git commit x -m \"feat: x\""}' "a commit where core.hooksPath runs the hooks"
t ALLOW hookless_commit "$_hk/plain" bash '{"command":"git commit -m x"}' "a repo that keeps no hooks"
t ALLOW hookless_commit "$_hk/bare" bash '{"command":"git log -1 && git status"}' "reading in a hookless clone is fine"
t ALLOW shared_index_staging "$REPO/lanes/k" bash '{"command":"git add x"}' "an install's own worktree has its own index" GUARD_WORKTREE_DIR=lanes

# bare_cargo_bypasses_make is ADVISORY, so both directions are ALLOW; what is
# pinned is WHICH commands draw the advice. The scoped forms must stay silent --
# AGENTS.md prints `cargo test -p <crate> <name>` as the way to run one test,
# and a rule that nagged about the documented workflow would be tuned out.
t ADVISE bare_cargo_bypasses_make "$REPO" bash '{"command":"cargo build"}' "workspace-wide cargo outside make"
t ALLOW bare_cargo_bypasses_make "$REPO" bash '{"command":"FULL_SUITE_OK=1 cargo test --workspace"}' "an explicit override is not nagged"
t ALLOW bare_cargo_bypasses_make "$REPO" bash '{"command":"cargo test -p core test_x"}' "scoped cargo is the documented form"
t ALLOW bare_cargo_bypasses_make "$REPO" bash '{"command":"cargo build"}' "nested under the build lock" GUARD_BUILD_LOCK_ENV=EXAMPLE_BUILD_LOCK EXAMPLE_BUILD_LOCK=4242
t ALLOW bare_cargo_bypasses_make "$REPO" bash '{"command":"cargo fmt"}' "non-compiling cargo subcommand"

# long_run_detached: a foreground cap kills a 45-minute gate AT the cap and can
# report the WRAPPER's exit code instead of the job's.
t ADVISE long_run_detached "$REPO" bash '{"command":"make check"}' "a long gate in the foreground" GUARD_LONG_TARGETS_RE='check|test|lint' GUARD_DETACHED_RE='bg-run\.sh'
t ADVISE long_run_detached "$REPO" bash '{"command":"make lint"}' "the other long target" GUARD_LONG_TARGETS_RE='check|test|lint' GUARD_DETACHED_RE='bg-run\.sh'
t ALLOW long_run_detached "$REPO" bash '{"command":"make check","run_in_background":true}' "already detached by the harness" GUARD_LONG_TARGETS_RE='check|test|lint' GUARD_DETACHED_RE='bg-run\.sh'
t ALLOW long_run_detached "$REPO" bash '{"command":"nohup make check > /tmp/x.log 2>&1 &"}' "hand-detached is the same intent" GUARD_LONG_TARGETS_RE='check|test|lint' GUARD_DETACHED_RE='bg-run\.sh'
t ALLOW long_run_detached "$REPO" bash '{"command":"make fmt"}' "a SHORT target must not be nagged" GUARD_LONG_TARGETS_RE='check|test|lint' GUARD_DETACHED_RE='bg-run\.sh'
t ALLOW long_run_detached "$REPO" bash '{"command":"make docs-check"}' "a similarly-named short target" GUARD_LONG_TARGETS_RE='check|test|lint' GUARD_DETACHED_RE='bg-run\.sh'
t ALLOW long_run_detached "$REPO" bash '{"command":"scripts/dev/bg-run.sh gate -- make check"}' "already detached through the runner" GUARD_LONG_TARGETS_RE='check|test|lint' GUARD_DETACHED_RE='bg-run\.sh'
t ADVISE long_run_detached "$REPO" bash '{"command":"make gate-merged"}' "an install's own long target" GUARD_LONG_TARGETS_RE=gate-merged
t ALLOW long_run_detached "$REPO" bash '{"command":"make check"}' "under the build lock" GUARD_LONG_TARGETS_RE='check|test|lint' GUARD_DETACHED_RE='bg-run\.sh' GUARD_BUILD_LOCK_ENV=EXAMPLE_BUILD_LOCK EXAMPLE_BUILD_LOCK=4242

t BLOCK pipe_masks_exit_code "$REPO" bash '{"command":"make check | tail -5 && echo ok"}' "make | tail && ..."
t ALLOW pipe_masks_exit_code "$REPO" bash '{"command":"make fmt > /tmp/g.log 2>&1; echo $?"}' "redirect and read the code"
t ALLOW pipe_masks_exit_code "$REPO" bash '{"command":"make fmt | tail -5"}' "pipe with no && gate"
# A real pipeline is ONE line. This multi-line case is PROSE: the word cargo
# in one line, a quoted "| head" in the next, a trailing && in the third.
# Newline-excluding classes are the only thing keeping this ALLOW; widen them back
# to [[:space:]] and this case goes red.
t ALLOW pipe_masks_exit_code "$REPO" bash '{"command": "echo \"cargo install/fix, npm install) piped into head or tail.\"\necho \"  - reads piped anywhere (grep | head).\"\necho \"pipe_masks_exit_code needs a trailing && and matches make/cargo/swift.\""}' "multi-line PROSE is not a pipeline"

# no_silent_failure, pipe clause: the sibling case pipe_masks_exit_code does NOT cover: no
# trailing &&, and the command is a WRITE rather than a build. `git commit ... |
# tail -1` reports success whatever the commit did. A pipe into grep hides it too.
t ADVISE no_silent_failure "$REPO" bash '{"command":"git commit -F msg | tail -1"}' "git write | tail hides rejection"
t ADVISE no_silent_failure "$REPO" bash '{"command":"npm install | tail -1"}' "package install | tail hides rejection"
t ADVISE no_silent_failure "$REPO" bash '{"command":"git push origin main | grep -q main"}' "git write | grep hides rejection"
t ALLOW no_silent_failure "$REPO" bash '{"command":"git show HEAD --stat | tail -5"}' "a READ piped to tail is normal"
t ALLOW no_silent_failure "$REPO" bash '{"command":"git log --oneline | head -3"}' "READ piped to head is normal"
t ALLOW no_silent_failure "$REPO" bash '{"command":"grep -rn foo src | head -5"}' "grep piped to head is normal"
t ALLOW no_silent_failure "$REPO" bash '{"command":"set -o pipefail; git commit -F msg | tail -1"}' "pipefail preserves the status"
t ALLOW no_silent_failure "$REPO" bash '{"command":"git commit -F msg | tee /tmp/o"}' "tee keeps the whole message"
t ADVISE no_silent_failure "$REPO" bash '{"command":"example add x | tail -1"}' "the product CLI write | tail (GUARD_WRITE_CMDS_RE)" GUARD_WRITE_CMDS_RE='example[[:space:]]+(add|edit|delete)'
t ALLOW no_silent_failure "$REPO" bash '{"command":"example list | tail -5"}' "a product CLI read piped to tail is normal" GUARD_WRITE_CMDS_RE='example[[:space:]]+(add|edit|delete)'
t ALLOW no_silent_failure "$REPO" bash '{"command":"example add x | tail -1"}' "an install whose CLI is another" GUARD_WRITE_CMDS_RE='other[[:space:]]+add'

# no_silent_failure, substitution clause: the OTHER half of the same class, which the
# pipe clause cannot see: no pipe, nothing truncated. A failed `$(cat f)` is a valid EMPTY
# STRING, so the write succeeds with an argument nobody supplied.

# shellcheck disable=SC2016 # the substitution must reach the guard as LITERAL text
t ADVISE no_silent_failure "$REPO" bash '{"command":"git commit -m \"$(cat /tmp/msg.txt)\""}' "a failed file read is an empty message"
# shellcheck disable=SC2016 # literal, as above
t ADVISE no_silent_failure "$REPO" bash '{"command":"git tag -a v1 -m \"$(cat /tmp/r.txt)\""}' "a failed read is an empty tag message"
# shellcheck disable=SC2016 # literal, as above
t ALLOW no_silent_failure "$REPO" bash '{"command":"grep \"$(cat /tmp/pattern.txt)\" src/"}' "a READ with an empty pattern writes nothing"
# shellcheck disable=SC2016 # literal, as above
t ALLOW no_silent_failure "$REPO" bash '{"command":"git commit -m \"$(git rev-parse HEAD)\""}' "not a file read; empty means a different bug"
# shellcheck disable=SC2016 # literal, as above
t ALLOW no_silent_failure "$REPO" bash '{"command":"set -o pipefail; git commit -m \"$(cat /tmp/msg.txt)\""}' "the author asked for strictness"
# shellcheck disable=SC2016 # literal, as above
t ADVISE no_silent_failure "$REPO" bash '{"command":"example add \"$(cat /tmp/r.txt)\""}' "a failed read is an empty product CLI write" GUARD_WRITE_CMDS_RE='example[[:space:]]+(add|edit|delete)'
# Both shapes on one call: ONE rule speaks, once, with both clauses. Two rules would print two
# advisories for one command.
# shellcheck disable=SC2016 # literal, as above
_run_engine '{"event":"pre_tool","tool":"bash","cwd":"'"$REPO"'","status":"ok","input":{"command":"git commit -m \"$(cat f)\" | tail -1"}}' "$REPO" GUARD_SELFTEST_NO_DESTRUCTIVE=1
_assert ONCE "$([ "$(grep -c '^guard-advise:' <<<"$_RUN_ERR")" = 1 ] && grep -q 'piped into head' <<<"$_RUN_ERR" &&
    grep -q 'EMPTY STRING' <<<"$_RUN_ERR" && echo ONCE || echo "got: $(grep '^guard-' <<<"$_RUN_ERR" | tr '\n' ' ')")" \
    "a piped commit with a file substitution gets one advisory with both clauses" no_silent_failure

t BLOCK rtk_proxy_reads "$REPO" bash '{"command":"git show HEAD:README.md"}' "git show ref:path"
t BLOCK rtk_proxy_reads "$REPO" bash '{"command":"git cat-file -p HEAD:README.md"}' "git cat-file -p"
t ALLOW rtk_proxy_reads "$REPO" bash '{"command":"rtk proxy git show HEAD:README.md"}' "through the proxy"
t ALLOW rtk_proxy_reads "$REPO" bash '{"command":"git show HEAD"}' "a plain git show"

# A build tree is a broad root at any depth.
t BLOCK unbounded_search "$REPO" bash '{"command":"grep -rn foo target/"}' "recursive grep over target/"
t BLOCK unbounded_search "$REPO" bash '{"command":"grep -rn foo crates/core/target"}' "a nested target/"
t ALLOW unbounded_search "$REPO" bash '{"command":"grep -rn foo crates/core/src"}' "a source dir is scoped"

echo
echo "lane model tier -- an Opus lane must name the class the cheaper model would get wrong"
t BLOCK lane_model_tier "$REPO" swarm '{"action":"spawn","model":"claude-opus-5-5","prompt":"Fix the panel hairline token"}' "an Opus lane with no class word"
t BLOCK lane_model_tier "$REPO" swarm '{"action":"spawn","model":"claude-opus-5-5","initial_message":"tidy the footer"}' "the prompt alias is read too"
t ALLOW lane_model_tier "$REPO" swarm '{"action":"spawn","model":"claude-opus-5-5","prompt":"relay schema v2 and its migrator"}' "a schema lane names its class" GUARD_OPUS_CLASS_RE='schema|migration|migrator|sync|crypto|security|irreversible'
t ALLOW lane_model_tier "$REPO" swarm '{"action":"spawn","model":"claude-opus-5-5","prompt":"change the store format"}' "an install's own class list" GUARD_OPUS_CLASS_RE='store format'
t BLOCK lane_model_tier "$REPO" swarm '{"action":"spawn","model":"claude-opus-5-5","prompt":"relay schema v2"}' "a class the install does not list" GUARD_OPUS_CLASS_RE='store format'
t ALLOW lane_model_tier "$REPO" swarm '{"action":"spawn","model":"claude-sonnet-5-5","prompt":"Fix the panel hairline token"}' "a cheaper lane needs no reason"
t ALLOW lane_model_tier "$REPO" swarm '{"action":"message","model":"claude-opus-5-5","message":"hi"}' "only a spawn is judged"
t BLOCK lane_model_tier "$REPO" swarm '{"action":"spawn","model":"claude-opus-5-5","prompt":"rename a label, e.g. \"security\" stays as is"}' "a class word inside a quoted example is not the brief"

echo
echo "cwd -- the command must be able to find what it needs"
t BLOCK cwd_has_marker "$ORG" bash '{"command":"make check"}' "make where there is no Makefile"
t BLOCK cwd_has_marker "$ORG" bash '{"command":"cargo build -p core"}' "cargo where there is no Cargo.toml"
t ALLOW cwd_has_marker "$REPO" bash '{"command":"make fmt"}' "make in the repo"
t ALLOW cwd_has_marker "$ORG" bash '{"command":"cd '"$REPO"' && make fmt"}' "explicit cd"
t ALLOW cwd_has_marker "$ORG" bash '{"command":"git -C '"$REPO"' commit -m x"}' "git -C is self-locating"


echo
echo "read size -- an unbounded read of a big file is reported, a bounded one is not"
# 2000 lines is over the default of 400, 50 is under it.
seq 1 2000 >"$REPO/big.log"
seq 1 50 >"$REPO/small.log"
printf '\211PNG\r\n\032\n' >"$REPO/shot.png"
seq 1 2000 >"$REPO/lane-accept-diff.txt"
t ADVISE read_size "$REPO" read '{"file_path":"'"$REPO"'/big.log"}' "read-size: a 2000-line file read whole"
t ADVISE read_size "$REPO" read '{"file_path":"big.log"}' "read-size: a relative path resolves against the cwd"
t ALLOW read_size "$REPO" read '{"file_path":"'"$REPO"'/small.log"}' "read-size: a small file read whole"
t ALLOW read_size "$REPO" read '{"file_path":"'"$REPO"'/big.log","limit":100}' "read-size: a read with a limit"
t ALLOW read_size "$REPO" read '{"file_path":"'"$REPO"'/big.log","offset":500}' "read-size: a read with an offset"
t ALLOW read_size "$REPO" read '{"file_path":"'"$REPO"'/big.log","start_line":500}' "read-size: a read with a start line"
t ALLOW read_size "$REPO" read '{"file_path":"'"$REPO"'/no-such-file.log"}' "read-size: a missing file is the tool's error to give"
t ALLOW read_size "$REPO" read '{"file_path":"'"$REPO"'/shot.png"}' "read-size: an image is not text lines"
t ALLOW read_size "$REPO" read '{"file_path":"'"$REPO"'/lane-accept-diff.txt"}' "read-size: a lane-accept diff is read whole"
t ADVISE read_size "$REPO" read '{"file_path":"'"$REPO"'/small.log"}' "read-size: the threshold is a key" GUARD_READ_MAX_LINES=10
t ALLOW read_size "$REPO" read '{"file_path":"'"$REPO"'/big.log"}' "read-size: a raised threshold" GUARD_READ_MAX_LINES=5000
t BLOCK read_size "$REPO" read '{"file_path":"'"$REPO"'/big.log"}' "read-size: block mode refuses" GUARD_READ_MODE=block
t ALLOW read_size "$REPO" read '{"file_path":"'"$REPO"'/big.log","limit":100}' "read-size: block mode still lets a limit through" GUARD_READ_MODE=block
t ADVISE read_size "$REPO" bash '{"command":"cat big.log"}' "read-size: cat of a 2000-line file"
t ALLOW read_size "$REPO" bash '{"command":"head big.log"}' "read-size: head with no count shows ten lines"
t ADVISE read_size "$REPO" bash '{"command":"cd '"$REPO"' && cat big.log"}' "read-size: cat after a cd"
t ALLOW read_size "$REPO" bash '{"command":"cat small.log"}' "read-size: cat of a small file"
t ALLOW read_size "$REPO" bash '{"command":"cat big.log | grep 7"}' "read-size: a piped cat"
t ALLOW read_size "$REPO" bash '{"command":"head -n 20 big.log"}' "read-size: head with a count"
t ALLOW read_size "$REPO" bash '{"command":"tail -50 big.log"}' "read-size: tail with a count"
t ALLOW read_size "$REPO" bash '{"command":"cat big.log > /tmp/zz-copy"}' "read-size: cat redirected away from the session"
t ALLOW read_size "$REPO" bash '{"command":"cat big.log lane-accept-out.txt"}' "read-size: a command naming lane-accept output"
t ALLOW read_size "$REPO" bash '{"command":"cat lane-accept-diff.txt"}' "read-size: a lane-accept diff file by cat"
t ALLOW read_size "$REPO" bash '{"command":"echo cat big.log"}' "read-size: the verb as data"

echo
echo "guard friction -- sandboxed test scripts, scratch worktrees, the quote-limit message"
# (1) A script FILE that sources test-sandbox.sh or carries `# guard: sandboxed` runs git in a
# throwaway repo: its git add / bare commit is not the shared index. Still caught: the same git
# typed beside it, a bash -c string, an unmarked script, a marker past line 10, an unreadable file.
_sb="$REPO/.runs/sbx"
mkdir -p "$_sb"
printf '#!/usr/bin/env bash\n. "$(dirname "$0")/../tools/test-sandbox.sh"\ngit add x\ngit commit -m t\n' >"$_sb/a_test.sh"
printf '#!/usr/bin/env bash\n# guard: sandboxed\ngit add x\n' >"$_sb/b_test.sh"
printf '#!/usr/bin/env bash\ngit add x\n' >"$_sb/plain.sh"
printf '#!/usr/bin/env bash\n. tools/test-sandbox.sh\ngit add x\n' >"$_sb/fresh_test.sh"
printf '#!/usr/bin/env bash\n. tools/test_sandbox.sh\ngit add x\n' >"$_sb/mod_test.sh"
git -C "$_sb" init -q 2>/dev/null
git -C "$_sb" add a_test.sh b_test.sh plain.sh mod_test.sh
git -C "$_sb" -c user.name=t -c user.email=t@t commit -qm sbx 2>/dev/null
printf 'git add -A\n' >>"$_sb/mod_test.sh"
t ALLOW shared_index_staging "$REPO" bash '{"command":"bash .runs/sbx/a_test.sh"}' "a tracked clean *_test.sh sourcing test-sandbox.sh may git add"
t BLOCK shared_index_staging "$REPO" bash '{"command":"bash .runs/sbx/b_test.sh"}' "a guard: sandboxed marker no longer exempts"
t BLOCK shared_index_staging "$REPO" bash '{"command":"bash .runs/sbx/fresh_test.sh"}' "an untracked script sourcing test-sandbox is not exempt"
t BLOCK shared_index_staging "$REPO" bash '{"command":"bash .runs/sbx/mod_test.sh"}' "a tracked but modified script is not exempt"
t ALLOW git_commit_index_scope "$REPO" bash '{"command":"bash .runs/sbx/a_test.sh"}' "a sandboxed script's bare commit"
t BLOCK shared_index_staging "$REPO" bash '{"command":"bash .runs/sbx/plain.sh"}' "an unmarked script still blocked"
t BLOCK shared_index_staging "$REPO" bash '{"command":"bash .runs/sbx/a_test.sh; git add x"}' "git typed beside a sandboxed script"
t BLOCK git_add_scope "$REPO" bash '{"command":"bash .runs/sbx/b_test.sh; git add -A"}' "git add -A beside a sandboxed script"

# (5) The refusal over the quote bound says what to do.
_q=$(printf "'a' %.0s" $(seq 1 600))
t BLOCK destructive_class "$REPO" bash "$(jq -cn --arg c "echo $_q" '{command:$c}')" "over 1000 quote chars is refused"
if _only "over 1000 quote chars is refused"; then
    case "$_RUN_ERR" in
        *'write the command to a file with the write tool and run `bash <file>`'*) _assert OK OK "the quote refusal names the write tool and bash <file>" ;;
        *) _assert OK BAD "the quote refusal names the write tool and bash <file>" ;;
    esac
fi

# (6) A detached worktree under the session scratch dir may checkout/reset. Still refused: the
# shared checkout, a branch-attached scratch worktree, a symlink out of scratch, chained commands.
_scr="$SELFTEST_ROOT/scr"
mkdir -p "$_scr"
_scr=$(cd "$_scr" && pwd -P)
git -C "$REPO" worktree add -q --detach "$_scr/wt" HEAD 2>/dev/null
git -C "$REPO" worktree add -q -b scr-attached "$_scr/att" HEAD 2>/dev/null
ln -sfn "$REPO" "$_scr/escape"
printf 'x\n' >"$_scr/wt/f.txt"
t ALLOW destructive_class "$_scr/wt" bash '{"command":"git checkout -- f.txt"}' "checkout in a detached scratch worktree" GUARD_SCRATCH_DIR="$_scr"
t ALLOW destructive_class "$_scr/wt" bash '{"command":"git reset --hard HEAD"}' "reset in a detached scratch worktree" GUARD_SCRATCH_DIR="$_scr"
t ALLOW destructive_class "$REPO" bash '{"command":"git -C '"$_scr"'/wt reset --hard"}' "-C names the detached scratch worktree" GUARD_SCRATCH_DIR="$_scr"
t BLOCK destructive_class "$REPO" bash '{"command":"git reset --hard HEAD"}' "reset on the shared checkout" GUARD_SCRATCH_DIR="$_scr"
t BLOCK destructive_class "$REPO" bash '{"command":"git checkout -- README.md"}' "checkout on the shared checkout" GUARD_SCRATCH_DIR="$_scr"
t BLOCK destructive_class "$REPO" bash '{"command":"git -C '"$_scr"'/att reset --hard"}' "a scratch worktree on a branch" GUARD_SCRATCH_DIR="$_scr"
t BLOCK destructive_class "$_scr/escape" bash '{"command":"git reset --hard"}' "a symlink out of scratch" GUARD_SCRATCH_DIR="$_scr"
t BLOCK destructive_class "$REPO" bash '{"command":"git -C '"$_scr"'/escape reset --hard"}' "-C through a symlink out of scratch" GUARD_SCRATCH_DIR="$_scr"
t BLOCK destructive_class "$_scr/wt" bash '{"command":"git reset --hard; git -C '"$REPO"' reset --hard"}' "chained onto the shared checkout" GUARD_SCRATCH_DIR="$_scr"
t BLOCK destructive_class "$REPO" bash '{"command":"git -C '"$_scr"'/wt reset --hard"}' "no scratch dir set means no exemption"
# The allowance is judged by guard_scratch_worktree_git itself: the other rules never refuse a
# branch-moving checkout, so only the verdict of the function shows a hole. $1=want yes|no.
_sw() {
    local want="$1" cwd="$2" cmd="$3" scr="${4-$_scr}" got=no
    _only "$5" || return 0
    # shellcheck source=/dev/null
    if (. "$ENGINE" && GUARD_CMD="$cmd" GUARD_CWD="$cwd" GUARD_SCRATCH_DIR="$scr" guard_scratch_worktree_git); then got=yes; fi
    _assert "$want" "$got" "$5"
}
_sw no "$_scr/wt" "git checkout -B main" "$_scr" "checkout -B refused in scratch"
_sw no "$_scr/wt" "git checkout -b x" "$_scr" "checkout -b refused in scratch"
_sw no "$_scr/wt" "git checkout --orphan x" "$_scr" "checkout --orphan refused in scratch"
_sw no "$_scr/wt" "git checkout scr-attached" "$_scr" "checkout of a branch name refused in scratch"
_sw no "$_scr/wt" "git reset --keep HEAD" "$_scr" "reset --keep refused in scratch"
_sw no "$_scr/wt" "git reset --hard HEAD" "" "reset with no scratch dir refused"
_sw no "$REPO" "git reset --hard HEAD" "$_scr" "reset run outside scratch refused"
_sw yes "$_scr/wt" "git reset --hard HEAD" "$_scr" "reset --hard allowed in detached scratch"
_sw yes "$_scr/wt" "git checkout --detach HEAD" "$_scr" "checkout --detach allowed in scratch"
_sw yes "$_scr/wt" "git checkout HEAD" "$_scr" "checkout <rev> allowed while detached"
_sw yes "$_scr/wt" "git checkout -- f.txt" "$_scr" "checkout -- paths allowed in scratch"
# A bash -c string has no file: no path, no exemption.
_bc=no
# shellcheck source=/dev/null
(. "$ENGINE" && _guard_body_sandboxed "" ". tools/test-sandbox.sh") && _bc=yes
_assert no "$_bc" "a bash -c string is never a sandboxed script"
git -C "$REPO" worktree remove --force "$_scr/wt" 2>/dev/null
git -C "$REPO" worktree remove --force "$_scr/att" 2>/dev/null
git -C "$REPO" branch -qD scr-attached 2>/dev/null
rm -rf "$_sb" "$_scr"

echo
echo "every refusal names a remedy"
# read from the tree under test, so a red probe on remedy.sh in a copy is felt
. "$GUARDS_SRC/tests/remedy.sh"
_off=$(remedy_offenders "$GUARDS_SRC")
if [ -z "$_off" ]; then
    echo "  ok   [REMEDY] every guard_block and fallback refusal carries a remedy line"
else
    echo "  FAIL refusal without a remedy line:"
    echo "$_off"
    FAILED=1
fi
CASES=$((CASES + 1))
# Green and red on a fixture: one remedy passes, a lone message is named.
_rt=$(mktemp -d "$_SCRATCH_PREFIX.remedy.XXXXXX")
mkdir -p "$_rt/rules/fallback" "$_rt/adapters"
printf 'guard_block "a" \\\n    "b"\n' >"$_rt/rules/ok.sh"
: >"$_rt/engine.sh"; : >"$_rt/rules/fallback/fail-closed.sh"; : >"$_rt/adapters/jcode-pre.sh"
if [ -z "$(remedy_offenders "$_rt")" ]; then echo "  ok   [REMEDY] fixture with a remedy passes"; else echo "  FAIL fixture with a remedy was flagged"; FAILED=1; fi
printf 'guard_block "only a message"\n' >"$_rt/rules/bad.sh"
if [ -n "$(remedy_offenders "$_rt")" ]; then echo "  ok   [REMEDY] fixture with a lone message is flagged"; else echo "  FAIL a block with no remedy went unseen"; FAILED=1; fi
rm -f "$_rt/rules/bad.sh"
printf 'guard_block "a" \\\n    "bash vendor/schwarzwald/kit/tools/clone.sh x"\n' >"$_rt/rules/lit.sh"
if [ -n "$(remedy_offenders "$_rt")" ]; then echo "  ok   [REMEDY] a literal kit path in a remedy is flagged"; else echo "  FAIL a literal kit path in a remedy went unseen"; FAILED=1; fi
rm -f "$_rt/rules/lit.sh"
printf 'guard_block "a" \\\n    "bash ${GUARD_KIT_DIR:-schwarzwald/kit}/tools/clone.sh x"\n' >"$_rt/rules/dflt.sh"
if [ -n "$(remedy_offenders "$_rt")" ]; then echo "  ok   [REMEDY] a hardcoded default kit path is flagged"; else echo "  FAIL a default kit path went unseen"; FAILED=1; fi
rm -f "$_rt/rules/dflt.sh"
printf 'guard_block "a" \\\n    "bash ${GUARD_KIT_DIR}/tools/clone.sh x"\n' >"$_rt/rules/good.sh"
if [ -z "$(remedy_offenders "$_rt")" ]; then echo "  ok   [REMEDY] a GUARD_KIT_DIR remedy passes"; else echo "  FAIL a GUARD_KIT_DIR remedy was flagged"; FAILED=1; fi
printf 'guard_block "only a message"\n' >"$_rt/rules/fallback/bad.sh"
if [ -n "$(remedy_offenders "$_rt")" ]; then echo "  ok   [REMEDY] the scan covers rules/fallback/"; else echo "  FAIL a fallback block with no remedy went unseen"; FAILED=1; fi
rm -rf "$_rt"
CASES=$((CASES + 6))

echo
echo "witness log records rule, verdict and session"
_wl=$(mktemp "$_SCRATCH_PREFIX.witness.XXXXXX")
_run_engine '{"event":"pre_tool","tool":"bash","cwd":"'"$REPO"'","session_id":"sess-from-input","status":"ok","input":{"command":"git stash"}}' "$REPO" GUARD_WITNESS="$_wl"
if grep -q 'sess-from-input bash .* pre_tool block git_stash' "$_wl"; then
    echo "  ok   [WITNESS] a block logs its rule, the verdict and the session from the hook input"
else
    echo "  FAIL witness line lacks rule, verdict or session: $(tail -2 "$_wl")"
    FAILED=1
fi
: >"$_wl"
_run_engine '{"event":"pre_tool","tool":"bash","cwd":"'"$REPO"'","session_id":"sess-two","status":"ok","input":{"command":"git status"}}' "$REPO" GUARD_WITNESS="$_wl"
if grep -q 'sess-two bash' "$_wl" && ! grep -q ' block ' "$_wl"; then
    echo "  ok   [WITNESS] an allowed call logs the session and no block"
else
    echo "  FAIL an allowed call was logged as a block or lost its session"
    FAILED=1
fi
rm -f "$_wl"
CASES=$((CASES + 2))

echo
echo "refusals do not advise the git show route"
_gs=$(grep -n 'git show HEAD:<path>' "$GUARDS_SRC"/engine.sh "$GUARDS_SRC"/rules/*.sh "$GUARDS_SRC"/rules/fallback/*.sh 2>/dev/null)
if [ -z "$_gs" ]; then
    echo "  ok   [HINT] no refusal advises git show ref:path, which rtk refuses"
else
    echo "  FAIL a refusal advises git show ref:path; name baseline.sh save|restore: $_gs"
    FAILED=1
fi
_run_engine '{"event":"pre_tool","tool":"bash","cwd":"'"$REPO"'","status":"ok","input":{"command":"git stash"}}' "$REPO"
# Only the git_stash rule's own lines count: the destructive class also blocks a stash with the same hint.
if awk '/^guard-block: /{on=($2=="git_stash")} on' <<<"$_RUN_ERR" | grep -q 'baseline.sh save <path>'; then
    echo "  ok   [HINT] the stash refusal names baseline.sh"
else
    echo "  FAIL the stash refusal does not name baseline.sh"
    FAILED=1
fi
CASES=$((CASES + 2))
