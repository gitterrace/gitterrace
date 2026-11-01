#!/bin/sh
# Test the example actions in docs/actions against stub commands. Each profile gets a bin
# dir holding the real basics (jq, cat, ...) plus recording stubs for the tools that
# profile has, and nothing else. Not shipped to users.
# shellcheck disable=SC2016  # single quotes hold payload text that must stay literal
set -u
root=$(cd "$(dirname "$0")/.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
fails=0
fail() { echo "FAIL: $*"; fails=$((fails + 1)); }

payload='{"title":"t $(touch PWNED) `touch PWNED2`","markdown":"md $(touch PWNED) body"}'

# profile NAME TOOL...: build $tmp/NAME/bin
profile() {
  name=$1; shift
  bin="$tmp/$name/bin"; mkdir -p "$bin"
  for t in jq cat basename dirname date mkdir; do
    p=$(command -v "$t") && ln -s "$p" "$bin/$t"
  done
  for t in "$@"; do
    printf '#!/bin/sh\nprintf "%%s\\n" "$0" "$@" >> "$GT_LOG"\ncat >> "$GT_LOG.stdin"\nexit 0\n' > "$bin/$t"
    chmod +x "$bin/$t"
  done
}

# run PROFILE SCRIPT [VAR=value...]: sets out, rc, log, stdin_log
run() {
  prof=$1; script=$2; shift 2
  d="$tmp/$prof"; GT_LOG="$d/log"; : > "$GT_LOG"; : > "$GT_LOG.stdin"
  mkdir -p "$d/work"
  out=$(cd "$d/work" && printf '%s' "$payload" | env -i "PATH=$d/bin" "HOME=$d" "GT_LOG=$GT_LOG" \
    'GITTERRACE_TITLE=t $(touch PWNED)' "GITTERRACE_ROOT=$d/my repo" 'GITTERRACE_PATH=a b/$(x).rs' \
    GITTERRACE_LINE_START=7 "$@" /bin/dash "$root/docs/actions/$script" 2>&1)
  rc=$?
  log=$(cat "$GT_LOG")
  stdin_log=$(cat "$GT_LOG.stdin")
}
expect_log() { case $log in *"$2"*) ;; *) fail "$1: log lacks '$2': $log" ;; esac; }
expect_rc() { [ "$rc" = "$2" ] || fail "$1: rc $rc, want $2 ($out)"; }
expect_clean() { [ ! -e "$tmp/$1/work/PWNED" ] && [ ! -e "$tmp/$1/work/PWNED2" ] || fail "$1: payload was executed"; }
one_line() {
  [ "$(printf '%s\n' "$out" | wc -l | tr -d ' ')" = 1 ] || fail "$1: not one line: $out"
  case $out in "gitterrace:"*) ;; *) fail "$1: message: $out" ;; esac
}

profile mac pbcopy open code tendir curl
profile x11 xclip xdg-open code
profile wayland wl-copy xdg-open
profile xsel xsel xdg-open
profile wsl clip.exe explorer.exe
profile none
profile clionly tendir
profile ed vim nano subl mystery

# clipboard
run mac clipboard.sh;     expect_rc mac-clip 0; expect_log mac-clip "/pbcopy"
case $stdin_log in *'md $(touch PWNED) body'*) ;; *) fail "mac-clip stdin: $stdin_log" ;; esac
run x11 clipboard.sh;     expect_rc x11-clip 0; expect_log x11-clip "/xclip"; expect_log x11-clip "clipboard"
run wayland clipboard.sh; expect_rc wl-clip 0;  expect_log wl-clip "/wl-copy"
run xsel clipboard.sh;    expect_rc xsel-clip 0; expect_log xsel-clip "--input"
run wsl clipboard.sh;     expect_rc wsl-clip 0; expect_log wsl-clip "clip.exe"
run none clipboard.sh;    expect_rc none-clip 1; one_line none-clip

# obsidian: url mode
run mac obsidian.sh GITTERRACE_VAR_MODE=url; expect_rc mac-obs 0; expect_log mac-obs "/open"; expect_log mac-obs "obsidian://new?vault=Notes"
case $log in *'$('*|*'`'*) fail "mac-obs: unencoded payload: $log" ;; esac
run x11 obsidian.sh GITTERRACE_VAR_MODE=url "GITTERRACE_VAR_VAULT=My Vault"; expect_rc x11-obs 0; expect_log x11-obs "/xdg-open"; expect_log x11-obs "vault=My%20Vault"
run wsl obsidian.sh GITTERRACE_VAR_MODE=url; expect_rc wsl-obs 0; expect_log wsl-obs "explorer.exe"
run none obsidian.sh GITTERRACE_VAR_MODE=url; expect_rc none-obs 1; one_line none-obs
# obsidian: file modes
run mac obsidian.sh; expect_rc daily 0
ls "$tmp"/mac/Notes/Daily/*.md >/dev/null 2>&1 || fail "daily note not in default place"
run mac obsidian.sh GITTERRACE_VAR_MODE=inbox "GITTERRACE_VAR_VAULT=$tmp/my vault" GITTERRACE_VAR_FOLDER=In; expect_rc inbox 0
grep -q 'md \$(touch PWNED) body' "$tmp/my vault/In/gitterrace.md" 2>/dev/null || fail "inbox note not written"
run mac obsidian.sh GITTERRACE_VAR_MODE=bogus; expect_rc obs-bogus 1; one_line obs-bogus
run mac obsidian.sh "GITTERRACE_VAR_VAULT=~/Tilde"; expect_rc obs-tilde 0
ls "$tmp"/mac/Tilde/Daily/*.md >/dev/null 2>&1 || fail "~/ in vault not expanded to HOME"
run none obsidian.sh; expect_rc none-daily 0

# tendir
run mac tendir.sh;           expect_rc mac-tendir 0; expect_log mac-tendir "/open"; expect_log mac-tendir "tendir://new-task?title="
case $log in *'$('*|*'`'*) fail "mac-tendir: unencoded payload: $log" ;; esac
run mac tendir.sh GITTERRACE_VAR_MODE=cli; expect_rc tendir-cli 0; expect_log tendir-cli "/tendir"; expect_log tendir-cli "--notes"
run x11 tendir.sh GITTERRACE_VAR_MODE=cli; expect_rc tendir-cli-missing 1; one_line tendir-cli-missing
run x11 tendir.sh;           expect_rc x11-tendir 0; expect_log x11-tendir "/xdg-open"
run clionly tendir.sh;       expect_rc clionly 0; expect_log clionly "--notes"
run mac tendir.sh GITTERRACE_VAR_MODE=bogus; expect_rc tendir-bogus 1; one_line tendir-bogus
run none tendir.sh;          expect_rc none-tendir 1; one_line none-tendir

# editor
run mac editor.sh;           expect_rc mac-ed 0; expect_log mac-ed "--goto"; expect_log mac-ed "my repo/a b/\$(x).rs:7"
run wayland editor.sh;       expect_rc none-ed 1; one_line none-ed
run ed editor.sh VISUAL=vim;    expect_rc vim 0; expect_log vim "+7"
run ed editor.sh EDITOR=nano;   expect_rc nano 0; expect_log nano "+7"
run ed editor.sh EDITOR=subl;   expect_rc subl 0; expect_log subl ".rs:7"
run ed editor.sh "EDITOR=vim -u NONE"; expect_rc vimflags 0; expect_log vimflags "NONE"
run ed editor.sh EDITOR=mystery; expect_rc mystery 0
case $log in *"+7"*|*":7"*) fail "mystery editor got a line: $log" ;; esac

# webhook
run mac webhook.sh GITTERRACE_VAR_URL=https://x.test/h; expect_rc mac-hook 0; expect_log mac-hook "https://x.test/h"
run none webhook.sh;         expect_rc none-hook 1; one_line none-hook

for p in mac x11 wayland xsel wsl none clionly ed; do expect_clean "$p"; done

if [ "$fails" = 0 ]; then echo "actions tests: ok"; else echo "actions tests: $fails failure(s)"; exit 1; fi
