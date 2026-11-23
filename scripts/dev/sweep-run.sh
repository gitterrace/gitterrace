#!/usr/bin/env bash
# Move the work clones in .wt/ whose work is already on main to .wt/.trash/, keep every other one.
#
#   scripts/dev/sweep-run.sh [--dry-run] [--limit N] [--ratchet] [<clone>]
#   scripts/dev/sweep-run.sh --self-test
#
# <clone> is a name under .wt/ or a path to one; without it every clone is looked at. A clone is
# removed only when ALL of these hold, and each clone kept is printed with the reason:
#   clean     no change and no untracked file, no stash
#   started   a commit was made in it (its HEAD reflog): a fresh clone is work about to start
#   landed    every commit of every local branch is on main: `git cherry` shows no `+` line, or
#             the paths changed since the merge base hold main's content (a squashed landing,
#             which cherry misses)
#   idle      no process has its cwd inside the clone (lsof, else /proc; neither keeps the clone)
#   settled   its HEAD commit and its newest file (outside .git) are 30 minutes old or more: a
#             clone landed while someone still writes to it is kept
# A removal is a move to .wt/.trash/<name>-<epoch>, never an rm: a misjudged clone can be taken
# back. Each real sweep purges trash entries older than 24 hours. .wt/.trash is not a clone and
# is not counted by --limit.
# --ratchet is --dry-run with the limit read from landed_clones in schwarzwald/config/ratchet.toml.
# --dry-run only reports. --limit N (with --dry-run) exits 1 when more than N clones could be
# removed: the `make lint` ratchet on piled-up landed clones.
# scripts/dev/land-run.sh runs it on the clone it just landed.
set -euo pipefail
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE

# SWEEP_ROOT names the main checkout when this copy runs from a clone (land-run.sh sets it).
root=${SWEEP_ROOT:-$(cd "$(dirname "$0")/../.." && pwd)}
MIN_AGE=30         # minutes a clone must sit untouched before it may be moved
TRASH_MAX_MIN=1440 # minutes a trashed clone is kept before the next sweep purges it

# kept_reason <clone> <main-ref>: print why the clone must stay, nothing when it may go.
kept_reason() {
  local c=$1 m=$2 base b p d names differs cwds
  [ -z "$(git -C "$c" status --porcelain --untracked-files=all 2>/dev/null)" ] || {
    echo "dirty or untracked files"
    return
  }
  [ -z "$(git -C "$c" stash list)" ] || {
    echo "has a stash"
    return
  }
  # A clone nobody has committed in yet holds work about to start, not work already landed:
  # with no commit of its own it would pass every landed check below. The reflog is read whole
  # first: grep -q stops early, and under pipefail the SIGPIPE to git would read as no commit.
  local moves
  moves=$(git -C "$c" log -g --format=%gs HEAD 2>/dev/null) || moves=""
  grep -qE '^(commit|rebase|cherry-pick|merge|am|revert)' <<<"$moves" || {
    echo "no commit made in it yet"
    return
  }
  while read -r b; do
    [ -n "$b" ] || continue
    if git -C "$c" cherry "$m" "$b" | grep -q '^+'; then
      base=$(git -C "$c" merge-base "$m" "$b" 2>/dev/null) || {
        echo "branch $b shares no history with main"
        return
      }
      # a loop, not mapfile: the macOS bash 3.2 has none. A failed diff counts as unlanded.
      differs=0
      names=$(git -C "$c" -c core.quotepath=off diff --name-only "$base" "$b") || differs=1
      while IFS= read -r p; do
        [ -z "$p" ] || git -C "$c" diff --quiet "$b" "$m" -- "$p" || differs=1
      done <<<"$names"
      if [ "$differs" = 1 ]; then
        echo "unlanded commits on $b"
        return
      fi
    fi
  done < <(
    git -C "$c" for-each-ref --format='%(refname)' refs/heads
    echo HEAD
  )
  # Age floor: whatever else holds, a clone touched in the last 30 minutes is still in use.
  # .git is skipped because the sweep's own fetch writes there.
  local ct
  ct=$(git -C "$c" log -1 --format=%ct HEAD 2>/dev/null) || ct=$(date +%s)
  if [ $(($(date +%s) - ct)) -lt $((MIN_AGE * 60)) ]; then
    echo "HEAD commit is younger than $MIN_AGE minutes"
    return
  fi
  local recent
  recent=$(find "$c" -path "$c/.git" -prune -o -mmin "-$MIN_AGE" -print 2>/dev/null | sed -n 1p)
  if [ -n "$recent" ]; then
    echo "a file is younger than $MIN_AGE minutes"
    return
  fi
  # lsof where it exists (macOS), else /proc (Linux CI images ship without lsof).
  if command -v lsof >/dev/null 2>&1; then
    cwds=$(lsof -d cwd -Fn 2>/dev/null | sed -n 's/^n//p')
  elif [ -d /proc/self/cwd ]; then
    cwds=$(for p in /proc/[0-9]*/cwd; do readlink "$p" 2>/dev/null; done)
  else
    echo "no lsof and no /proc, cannot tell if it is in use"
    return
  fi
  while IFS= read -r d; do
    case $d in "$c" | "$c"/*)
      echo "a process has its cwd inside"
      return
      ;;
    *) ;;
    esac
  done <<<"$cwds"
}

# sweep_one <clone> <dry>: prints "kept", "would remove" or "removed" and returns 0 when removable.
sweep_one() {
  local c=$1 dry=$2 name why
  name=$(basename "$c")
  [ -d "$c/.git" ] && [ "$(git -C "$c" rev-parse --show-toplevel)" = "$c" ] || {
    echo "kept $name: not a git clone of its own"
    return 1
  }
  git -C "$c" fetch -q "$root" main 2>/dev/null || {
    echo "kept $name: cannot read main"
    return 1
  }
  why=$(kept_reason "$c" FETCH_HEAD)
  if [ -n "$why" ]; then
    echo "kept $name: $why"
    return 1
  fi
  if [ "$dry" = 1 ]; then echo "would remove $name"; else
    local trash dest
    trash=$(dirname "$c")/.trash
    dest=$trash/$name-$(date +%s)
    mkdir -p "$trash"
    mv -- "$c" "$dest"
    touch "$dest" # the purge age counts from the move, not from the clone's own mtime
    echo "removed $name (moved to .wt/.trash/${dest##*/})"
  fi
}

# sweep <dry> <limit> [<clone>]
sweep() {
  local dry=$1 limit=$2 arg=${3:-} dir c n=0
  dir=$(cd "$root/.wt" 2>/dev/null && pwd -P) || return 0
  if [ "$dry" = 0 ] && [ -d "$dir/.trash" ]; then
    find "$dir/.trash" -mindepth 1 -maxdepth 1 -mmin "+$TRASH_MAX_MIN" -exec rm -rf {} +
  fi
  if [ -n "$arg" ]; then
    case $arg in */*) c=$(cd "$arg" && pwd -P) ;; *) c=$dir/$arg ;; esac
    [ "$(dirname "$c")" = "$dir" ] || {
      echo "sweep: $arg is not a clone directly under .wt/" >&2
      return 2
    }
    [ -d "$c" ] || {
      echo "sweep: no clone $arg" >&2
      return 2
    }
    sweep_one "$c" "$dry" || true
    return 0
  fi
  for c in "$dir"/*/; do
    [ -d "$c" ] || continue
    sweep_one "${c%/}" "$dry" && n=$((n + 1)) || true
  done
  if [ -n "$limit" ] && [ "$n" -gt "$limit" ]; then
    echo "sweep: $n clones are clean and landed, more than $limit. Run scripts/dev/sweep-run.sh" >&2
    return 1
  fi
}

# --- self-test: a temp repo with one clone per case ---
st_fail() {
  echo "sweep-run self-test: $*" >&2
  exit 1
}

t=""
self_test() {
  local main out c sw old
  t=$(cd "$(mktemp -d)" && pwd -P)
  trap 'rm -rf "$t"' EXIT
  main=$t/main
  # GD, when set, dates a commit: the clones that must be old enough to go are committed in 2020
  g() {
    env ${GD:+GIT_AUTHOR_DATE="$GD" GIT_COMMITTER_DATE="$GD"} \
      git -C "$1" -c user.name=t -c user.email=t@t "${@:2}"
  }
  old=2020-01-01T00:00:00
  age() { find "$1" -path "$1/.git" -prune -o -exec touch -t 202001010000 {} +; }
  git init -q -b main "$main"
  mkdir -p "$main/.wt" "$main/scripts/dev"
  cp "$0" "$main/scripts/dev/sweep-run.sh"
  sw=$main/scripts/dev/sweep-run.sh
  echo .wt >"$main/.git/info/exclude"
  printf 'one\n' >"$main/a.txt"
  g "$main" add a.txt scripts && g "$main" commit -q -m base
  for c in dirty untracked unlanded busy cherry squash fresh young writing; do
    git clone -q --no-local "$main" "$main/.wt/$c"
  done
  printf 'edit\n' >>"$main/.wt/dirty/a.txt"
  : >"$main/.wt/untracked/new.txt"
  # unlanded: a commit main lacks
  printf 'x\n' >"$main/.wt/unlanded/u.txt"
  g "$main/.wt/unlanded" add u.txt && g "$main/.wt/unlanded" commit -q -m unlanded
  # busy: a landed clone with a process inside
  printf 'b\n' >"$main/.wt/busy/b.txt"
  GD=$old g "$main/.wt/busy" add b.txt && GD=$old g "$main/.wt/busy" commit -q -m busy
  g "$main" fetch -q "$main/.wt/busy" HEAD && g "$main" cherry-pick FETCH_HEAD >/dev/null
  (cd "$main/.wt/busy" && exec sleep 600) &
  local busy=$!
  # cherry: land the commit by cherry-pick, as land.sh does
  printf 'c\n' >"$main/.wt/cherry/c.txt"
  g "$main/.wt/cherry" add c.txt && GD=$old g "$main/.wt/cherry" commit -q -m cherry
  g "$main" fetch -q "$main/.wt/cherry" HEAD && g "$main" cherry-pick FETCH_HEAD >/dev/null
  # squash: two commits in the clone land as one commit on main, which cherry cannot match
  printf 'two\n' >"$main/.wt/squash/a.txt"
  GD=$old g "$main/.wt/squash" commit -q -am s1
  printf 'three\n' >>"$main/.wt/squash/a.txt"
  GD=$old g "$main/.wt/squash" commit -q -am s2
  cp "$main/.wt/squash/a.txt" "$main/a.txt"
  g "$main" commit -q -am squashed
  # young: landed, committed now. writing: landed with an old commit, but a file was just written.
  printf 'y\n' >"$main/.wt/young/y.txt"
  g "$main/.wt/young" add y.txt && g "$main/.wt/young" commit -q -m young
  g "$main" fetch -q "$main/.wt/young" HEAD && g "$main" cherry-pick FETCH_HEAD >/dev/null
  printf 'w\n' >"$main/.wt/writing/w.txt"
  g "$main/.wt/writing" add w.txt && GD=$old g "$main/.wt/writing" commit -q -m writing
  g "$main" fetch -q "$main/.wt/writing" HEAD && g "$main" cherry-pick FETCH_HEAD >/dev/null
  for c in busy cherry squash writing; do age "$main/.wt/$c"; done
  touch "$main/.wt/writing/w.txt"
  sleep 1
  out=$(bash "$sw" --dry-run) || true
  for c in dirty untracked unlanded busy fresh; do
    grep -q "^kept $c: " <<<"$out" || st_fail "red case $c was not kept:\n$out"
    [ -d "$main/.wt/$c" ] || st_fail "red case $c was removed in dry-run"
  done
  grep -q '^kept fresh: no commit made' <<<"$out" || st_fail "fresh reason missing:\n$out"
  grep -q '^kept dirty: dirty' <<<"$out" || st_fail "dirty reason missing"
  grep -q '^kept untracked: dirty' <<<"$out" || st_fail "untracked reason missing"
  grep -q '^kept unlanded: unlanded' <<<"$out" || st_fail "unlanded reason missing"
  grep -q '^kept busy: a process' <<<"$out" || st_fail "busy reason missing:\n$out"
  grep -q '^kept young: HEAD commit is younger' <<<"$out" || st_fail "young not kept:\n$out"
  grep -q '^kept writing: a file is younger' <<<"$out" || st_fail "writing not kept:\n$out"
  for c in cherry squash; do
    grep -q "^would remove $c\$" <<<"$out" || st_fail "green case $c is not removable:\n$out"
    [ -d "$main/.wt/$c" ] || st_fail "dry-run removed $c"
  done
  # limit ratchet: two removable clones are over a limit of 1, within 2
  bash "$sw" --dry-run --limit 1 >/dev/null 2>&1 && st_fail "limit 1 passed with 2 removable"
  bash "$sw" --dry-run --limit 2 >/dev/null 2>&1 || st_fail "limit 2 failed with 2 removable"
  # the ratchet reads its limit from the registry: 1 is red, 2 is green
  mkdir -p "$main/schwarzwald/config"
  printf '[counts]\nlanded_clones = 1\n' >"$main/schwarzwald/config/ratchet.toml"
  bash "$sw" --ratchet >/dev/null 2>&1 && st_fail "ratchet 1 passed with 2 removable"
  printf '[counts]\nlanded_clones = 2\n' >"$main/schwarzwald/config/ratchet.toml"
  bash "$sw" --ratchet >/dev/null 2>&1 || st_fail "ratchet 2 failed with 2 removable"
  rm -r "$main/schwarzwald"
  # real run on the clone argument: green goes, red stays
  bash "$sw" busy >/dev/null
  [ -d "$main/.wt/busy" ] || st_fail "an explicit busy clone was removed"
  bash "$sw" unlanded >/dev/null
  [ -d "$main/.wt/unlanded" ] || st_fail "an explicit unlanded clone was removed"
  bash "$sw" cherry >/dev/null
  [ ! -d "$main/.wt/cherry" ] || st_fail "the cherry-landed clone was kept"
  # trash, not rm: the clone is moved, and the trash is not a clone to the limit ratchet
  set -- "$main"/.wt/.trash/cherry-*
  [ -f "$1/c.txt" ] || st_fail "the removed clone is not in .wt/.trash"
  bash "$sw" --dry-run --limit 5 2>&1 | grep -q trash && st_fail "the dry run looked at .trash"
  local trashed=$1
  bash "$sw" busy >/dev/null
  [ -d "$trashed" ] || st_fail "a fresh trash entry was purged"
  bash "$sw" >/dev/null
  [ ! -d "$main/.wt/squash" ] || st_fail "the squash-landed clone was kept"
  for c in dirty untracked unlanded busy fresh young writing; do
    [ -d "$main/.wt/$c" ] || st_fail "a real run removed red case $c"
  done
  touch -t 202001010000 "$trashed"
  bash "$sw" >/dev/null
  [ ! -d "$trashed" ] || st_fail "an old trash entry was not purged"
  compgen -G "$main/.wt/.trash/squash-*" >/dev/null || st_fail "the fresh trash entry was purged"
  # once the process is gone the busy clone is removable: the lsof condition was the only reason
  kill "$busy" 2>/dev/null || true
  wait "$busy" 2>/dev/null || true
  bash "$sw" busy >/dev/null
  [ ! -d "$main/.wt/busy" ] || st_fail "the idle clone was kept"
  echo "sweep-run self-test: ok"
}

dry=0 limit="" arg="" mode=sweep
while [ $# -gt 0 ]; do
  case $1 in
  --dry-run) dry=1 ;;
  --limit)
    limit=${2:?--limit needs a number}
    shift
    ;;
  --ratchet)
    dry=1
    reg=$root/schwarzwald/config/ratchet.toml
    limit=$(sed -n 's/^landed_clones *= *\([0-9][0-9]*\).*/\1/p' "$reg")
    [ -n "$limit" ] || {
      echo "sweep: landed_clones is missing in schwarzwald/config/ratchet.toml" >&2
      exit 2
    }
    ;;
  --self-test) mode=self-test ;;
  -*)
    echo "usage: sweep-run.sh [--dry-run] [--limit N] [--ratchet] [<clone>] | --self-test" >&2
    exit 2
    ;;
  *) arg=$1 ;;
  esac
  shift
done
[ "$mode" = sweep ] || {
  self_test
  exit 0
}
[ -z "$limit" ] || [ "$dry" = 1 ] || {
  echo "sweep: --limit needs --dry-run" >&2
  exit 2
}
sweep "$dry" "$limit" "$arg"
