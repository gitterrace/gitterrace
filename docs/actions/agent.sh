#!/bin/sh
# Hand the selection to a coding agent in a new terminal. Use with
#   command = ["~/.config/gitterrace/agent.sh", "claude"]
#   stdin   = "markdown"
#   cwd     = "repo"
# The note, with its source line, becomes the agent's first prompt. The agent starts
# interactive, so you see and approve what it does. This script only opens the window and
# returns at once, so the action never waits on the agent.
#
# Needs (all OSes): the agent on PATH: claude, codex, kiro-cli, gemini or any command
# that takes a prompt last. A terminal: Terminal on macOS, $TERMINAL or xterm on Linux.
set -eu
# pipefail is POSIX since 2024 but older dash lacks it, so it is set only where supported.
# shellcheck disable=SC3040
(set -o pipefail) 2>/dev/null && set -o pipefail
if [ "$#" != 1 ]; then
  echo "gitterrace: agent.sh takes one agent: claude, codex, kiro, gemini or a command."
  exit 1
fi
case "$1" in
claude) run="claude" ;;
codex) run="codex" ;;
kiro) run="kiro-cli chat" ;;
gemini) run="gemini -i" ;;
*) run=$1 ;;
esac
bin=${run%% *}
if ! command -v "$bin" >/dev/null 2>&1; then
  echo "gitterrace: $bin not found on PATH. Install it, or add its folder to PATH."
  exit 1
fi
# The prompt goes through a file, so no part of the note is ever read as shell code.
prompt=$(mktemp "${TMPDIR:-/tmp}/gitterrace-agent.XXXXXX")
cat >"$prompt"
# Single-quote a value for the inner shell line: ' becomes '\''.
q() { printf "'%s'" "$(printf '%s' "$1" | sed "s/'/'\\\\''/g")"; }
line="cd $(q "$PWD") && p=\$(cat $(q "$prompt")) && rm -f $(q "$prompt") && exec $run \"\$p\""
if [ "$(uname)" = Darwin ]; then
  osascript -e 'on run argv' -e 'tell application "Terminal"' \
    -e 'do script (item 1 of argv)' -e 'activate' -e 'end tell' -e 'end run' "$line" >/dev/null
else
  for t in "${TERMINAL:-}" x-terminal-emulator gnome-terminal konsole xterm; do
    if [ -n "$t" ] && command -v "$t" >/dev/null 2>&1; then
      case "$t" in
      gnome-terminal) setsid "$t" -- sh -c "$line" >/dev/null 2>&1 & ;;
      *) setsid "$t" -e sh -c "$line" >/dev/null 2>&1 & ;;
      esac
      echo "opened $bin"
      exit 0
    fi
  done
  rm -f "$prompt"
  echo "gitterrace: no terminal found. Set \$TERMINAL to your terminal's command."
  exit 1
fi
echo "opened $bin"
