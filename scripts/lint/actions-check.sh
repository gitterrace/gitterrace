#!/bin/sh
# Test the example actions in docs/actions against stub commands. Each profile gets a bin
# dir holding the real basics (jq, cat, ...) plus recording stubs for the tools that
# profile has, and nothing else. Not shipped to users.
# shellcheck disable=SC2016  # single quotes hold payload text that must stay literal
# No errexit: a failing action is the thing under test, its status is read from $? and
# recorded by fail(), so the run goes on to report every case.
set -u
# pipefail is POSIX since 2024 but older dash lacks it, so it is set only where supported.
# shellcheck disable=SC3040
(set -o pipefail) 2>/dev/null && set -o pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
fails=0
fail() {
  echo "FAIL: $*"
  fails=$((fails + 1))
}

payload='{"title":"t $(touch PWNED) `touch PWNED2`","markdown":"md $(touch PWNED) body"}'

# profile NAME TOOL...: build $tmp/NAME/bin
profile() {
  name=$1
  shift
  bin="$tmp/$name/bin"
  mkdir -p "$bin"
  for t in jq cat basename dirname date mkdir; do
    p=$(command -v "$t") && ln -s "$p" "$bin/$t"
  done
  for t in "$@"; do
    printf '#!/bin/sh\nprintf "%%s\\n" "$0" "$@" >> "$TEST_ACTIONS_LOG"\ncat >> "$TEST_ACTIONS_LOG.stdin"\nexit 0\n' >"$bin/$t"
    chmod +x "$bin/$t"
  done
}

# run PROFILE SCRIPT [VAR=value...]: sets out, rc, log, stdin_log
run() {
  prof=$1
  script=$2
  shift 2
  d="$tmp/$prof"
  TEST_ACTIONS_LOG="$d/log"
  : >"$TEST_ACTIONS_LOG"
  : >"$TEST_ACTIONS_LOG.stdin"
  mkdir -p "$d/work"
  out=$(cd "$d/work" && printf '%s' "$payload" | env -i "PATH=$d/bin" "HOME=$d" "TEST_ACTIONS_LOG=$TEST_ACTIONS_LOG" \
    'GITTERRACE_TITLE=t $(touch PWNED)' "GITTERRACE_ROOT=$d/my repo" 'GITTERRACE_PATH=a b/$(x).rs' \
    GITTERRACE_LINE_START=7 "$@" /bin/dash "$root/docs/actions/$script" 2>&1)
  rc=$?
  log=$(cat "$TEST_ACTIONS_LOG")
  stdin_log=$(cat "$TEST_ACTIONS_LOG.stdin")
}
# runa PROFILE SCRIPT STDIN ARG...: as run, for a script the action calls with placeholder
# arguments. STDIN is what stdin = "markdown" or "none" sends; the args are what gitterrace
# expands, one element each.
runa() {
  prof=$1
  script=$2
  in=$3
  shift 3
  d="$tmp/$prof"
  TEST_ACTIONS_LOG="$d/log"
  : >"$TEST_ACTIONS_LOG"
  : >"$TEST_ACTIONS_LOG.stdin"
  mkdir -p "$d/work"
  out=$(cd "$d/work" && printf '%s' "$in" | env -i "PATH=$d/bin" "HOME=$d" "TEST_ACTIONS_LOG=$TEST_ACTIONS_LOG" \
    'GITTERRACE_TITLE=t $(touch PWNED)' ${TEST_EDITOR:+"$TEST_EDITOR"} /bin/dash "$root/docs/actions/$script" "$@" 2>&1)
  rc=$?
  log=$(cat "$TEST_ACTIONS_LOG")
  stdin_log=$(cat "$TEST_ACTIONS_LOG.stdin")
}
expect_log() { case $log in *"$2"*) ;; *) fail "$1: log lacks '$2': $log" ;; esac }
expect_rc() { [ "$rc" = "$2" ] || fail "$1: rc $rc, want $2 ($out)"; }
expect_clean() {
  if [ -e "$tmp/$1/work/PWNED" ] || [ -e "$tmp/$1/work/PWNED2" ]; then
    fail "$1: payload was executed"
  fi
}
one_line() {
  [ "$(printf '%s\n' "$out" | wc -l | tr -d ' ')" = 1 ] || fail "$1: not one line: $out"
  case $out in "gitterrace:"*) ;; *) fail "$1: message: $out" ;; esac
}

profile mac pbcopy open code
profile x11 xclip xdg-open code
profile wayland wl-copy xdg-open
profile xsel xsel xdg-open
profile wsl clip.exe explorer.exe
profile none
profile ed vim nano subl mystery
profile zedonly zed

# clipboard: stdin = "markdown", so the text arrives raw and jq is not needed
md='md $(touch PWNED) body'
runa mac clipboard.sh "$md"
expect_rc mac-clip 0
expect_log mac-clip "/pbcopy"
[ "$stdin_log" = "$md" ] || fail "mac-clip stdin: $stdin_log"
runa x11 clipboard.sh "$md"
expect_rc x11-clip 0
expect_log x11-clip "/xclip"
expect_log x11-clip "clipboard"
runa wayland clipboard.sh "$md"
expect_rc wl-clip 0
expect_log wl-clip "/wl-copy"
runa xsel clipboard.sh "$md"
expect_rc xsel-clip 0
expect_log xsel-clip "--input"
runa wsl clipboard.sh "$md"
expect_rc wsl-clip 0
expect_log wsl-clip "clip.exe"
runa none clipboard.sh "$md"
expect_rc none-clip 1
one_line none-clip

# obsidian: daily append (new notes go through obsidian://new, no script)
run mac obsidian.sh
expect_rc daily 0
ls "$tmp"/mac/Notes/Daily/*.md >/dev/null 2>&1 || fail "daily note not in default place"
run mac obsidian.sh "GITTERRACE_VAR_VAULT=$tmp/my vault" GITTERRACE_VAR_FOLDER=In
expect_rc daily-folder 0
grep -q 'md \$(touch PWNED) body' "$tmp/my vault/In/$(date +%F).md" 2>/dev/null || fail "daily note not written in FOLDER"
run mac obsidian.sh "GITTERRACE_VAR_VAULT=~/Tilde"
expect_rc obs-tilde 0
ls "$tmp"/mac/Tilde/Daily/*.md >/dev/null 2>&1 || fail "tilde in vault not expanded to HOME"
run none obsidian.sh
expect_rc none-daily 0

# editor: command = [editor.sh, "{line_start}", "--", "{root}/{path}"], stdin = "none"
f="$tmp/my repo/a b/\$(x).rs"
runa mac editor.sh '' 7 -- "$f"
expect_rc mac-ed 0
expect_log mac-ed "--goto"
expect_log mac-ed "my repo/a b/\$(x).rs:7"
runa mac editor.sh '' '' -- "$f"
expect_rc ed-noline 0
expect_log ed-noline ".rs:1"
runa mac editor.sh ''
expect_rc ed-noargs 1
one_line ed-noargs
runa wayland editor.sh '' 7 -- "$f"
expect_rc none-ed 1
one_line none-ed
TEST_EDITOR=VISUAL=vim runa ed editor.sh '' 7 "$f"
expect_rc vim 0
expect_log vim "+7"
TEST_EDITOR=EDITOR=nano runa ed editor.sh '' 7 "$f"
expect_rc nano 0
expect_log nano "+7"
TEST_EDITOR=EDITOR=subl runa ed editor.sh '' 7 "$f"
expect_rc subl 0
expect_log subl ".rs:7"
TEST_EDITOR="EDITOR=vim -u NONE" runa ed editor.sh '' 7 "$f"
expect_rc vimflags 0
expect_log vimflags "NONE"
TEST_EDITOR='' runa zedonly editor.sh '' 7 "$f"
expect_rc zed-fallback 0
expect_log zed-fallback ".rs:7"
TEST_EDITOR=EDITOR=zed runa zedonly editor.sh '' 7 "$f"
expect_rc zed 0
expect_log zed ".rs:7"
TEST_EDITOR=EDITOR=mystery runa ed editor.sh '' 7 "$f"
expect_rc mystery 0
case $log in
*"+7"* | *":7"*) fail "mystery editor got a line: $log" ;;
*) ;;
esac

for p in mac x11 wayland xsel wsl none ed; do expect_clean "$p"; done

if [ "$fails" = 0 ]; then echo "actions tests: ok"; else
  echo "actions tests: $fails failure(s)"
  exit 1
fi
