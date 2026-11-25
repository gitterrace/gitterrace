#!/usr/bin/env bash
# quiet.sh: run a command only when the machine is quiet, one such command at a time.
#
#   quiet.sh [--max-load N] [--timeout S] -- cmd...
#
# Takes one machine-wide lock (a per-user 0700 dir under ${XDG_RUNTIME_DIR:-$TMPDIR}, mkdir is the
# atomic step, the holder pid is written inside, a lock whose pid is dead is cleared), then waits
# until the 1-minute load average is below N, runs cmd and releases the lock on any exit. N comes
# from --max-load, else $QUIET_MAX_LOAD, else twice the CPU count: background agents keep a busy
# machine near one load per core, so the CPU count alone would wait forever. Exit code is cmd's,
# or 75 on timeout. Use it for benches, timing tests and
# full gates so lanes do not start them together and read noise.
# QUIET_LOAD_CMD (tests only) replaces the load reading: a command printing a number.
set -u
max="${QUIET_MAX_LOAD:-}"; timeout=0
while [ $# -gt 0 ]; do
  case $1 in
    --max-load) max=${2:?--max-load needs a value}; shift 2 ;;
    --timeout) timeout=${2:?--timeout needs a value}; shift 2 ;;
    --) shift; break ;;
    *) break ;;
  esac
done
[ $# -gt 0 ] || { echo "usage: quiet.sh [--max-load N] [--timeout S] -- cmd..." >&2; exit 2; }
if [ -z "$max" ]; then
  max=$(( 2 * $(sysctl -n hw.ncpu 2>/dev/null || nproc 2>/dev/null || echo 1) ))
fi
load() {
  if [ -n "${QUIET_LOAD_CMD:-}" ]; then sh -c "$QUIET_LOAD_CMD"; return; fi
  if [ -r /proc/loadavg ]; then cut -d' ' -f1 /proc/loadavg
  else sysctl -n vm.loadavg | tr -d '{}' | awk '{print $1}'; fi
}
base=${XDG_RUNTIME_DIR:-${TMPDIR:-/tmp}}
dir="${base%/}/quiet-$(id -u)"
mkdir -p "$dir" && chmod 700 "$dir" || exit 1
lock="$dir/lock"
start=$SECONDS
holder() { cat "$lock/pid" 2>/dev/null || echo "?"; }
announced=0
while :; do
  if mkdir "$lock" 2>/dev/null; then
    echo $$ >"$lock/pid"
    break
  fi
  p=$(holder)
  if [ "$p" != "?" ] && [ -n "$p" ] && ! kill -0 "$p" 2>/dev/null; then
    rm -rf "$lock"; continue
  fi
  if [ "$announced" = 0 ]; then echo "quiet: waiting, lock held, holder pid $p" >&2; announced=1; fi
  if [ "$timeout" -gt 0 ] && [ $((SECONDS - start)) -ge "$timeout" ]; then exit 75; fi
  sleep 1
done
trap 'rm -rf "$lock"' EXIT
trap 'exit 143' TERM
trap 'exit 130' INT
announced=0
while :; do
  l=$(load)
  if awk -v l="$l" -v m="$max" 'BEGIN{exit !(l < m)}'; then break; fi
  if [ "$announced" = 0 ]; then echo "quiet: waiting, load $l > $max, holder pid $$" >&2; announced=1; fi
  if [ "$timeout" -gt 0 ] && [ $((SECONDS - start)) -ge "$timeout" ]; then exit 75; fi
  sleep 1
done
"$@"
