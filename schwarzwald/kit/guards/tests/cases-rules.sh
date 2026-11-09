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
_sc_now() { perl -MTime::HiRes=time -e 'printf "%d\n", time * 1000'; }
# The faster of two calls per script: a load spike during one call cannot fail the case.
_sc_best() {
    local best="" a d label="$2"
    for _ in 1 2; do
        a=$(_sc_now)
        t ADVISE broad_process_kill "$REPO" bash "{\"command\":\"bash .runs/$1\"}" "$label"
        d=$(($(_sc_now) - a))
        [ -z "$best" ] || [ "$d" -lt "$best" ] && best=$d
        label="$label (again)"
    done
    _sc_ms=$best
}
_sc_best sc-kill.sh "a small script with a broad pkill, the timing baseline"
_sc_small=$_sc_ms
_sc_best sc-big.sh "a script near the size cap with a broad pkill on its last line"
_sc_big=$_sc_ms
# A ratio, not a wall-clock budget: load slows both calls alike. Linear reading keeps the big
# script within a few times the small one. The quadratic join ran it 20 times slower and more.
_assert yes "$([ "$_sc_big" -le $((_sc_small * 8 + 500)) ] && echo yes || echo no)" \
    "the script near the size cap costs at most 8 times a small one (${_sc_big} ms vs ${_sc_small} ms)"
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
t ALLOW worktree_location "$REPO" bash '{"command":"git worktree add -b x .wt/x main"}' "worktree in .wt/"
t ALLOW worktree_location "$REPO" bash '{"command":"git worktree list"}' "listing worktrees"
t ALLOW worktree_location "$REPO" bash '{"command":"git worktree add -b x lanes/x main"}' "an install's own worktree dir" GUARD_WORKTREE_DIR=lanes
t BLOCK worktree_location "$REPO" bash '{"command":"git worktree add -b x .wt/x main"}' "the default dir is not an install's dir" GUARD_WORKTREE_DIR=lanes
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
