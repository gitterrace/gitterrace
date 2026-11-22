#!/usr/bin/env bash
# Remove the work clones in .wt/ whose work is already on main, and keep every other one.
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
# --ratchet is --dry-run with the limit read from landed_clones in schwarzwald/config/ratchet.toml.
# --dry-run only reports. --limit N (with --dry-run) exits 1 when more than N clones could be
# removed: the `make lint` ratchet on piled-up landed clones.
# scripts/dev/land-run.sh runs it on the clone it just landed.
set -euo pipefail
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE

# SWEEP_ROOT names the main checkout when this copy runs from a clone (land-run.sh sets it).
root=${SWEEP_ROOT:-$(cd "$(dirname "$0")/../.." && pwd)}

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
    rm -rf -- "$c"
    echo "removed $name"
  fi
}

# sweep <dry> <limit> [<clone>]
sweep() {
  local dry=$1 limit=$2 arg=${3:-} dir c n=0
  dir=$(cd "$root/.wt" 2>/dev/null && pwd -P) || return 0
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
  local main out c sw
  t=$(cd "$(mktemp -d)" && pwd -P)
  trap 'rm -rf "$t"' EXIT
  main=$t/main
  g() { git -C "$1" -c user.name=t -c user.email=t@t "${@:2}"; }
  git init -q -b main "$main"
  mkdir -p "$main/.wt" "$main/scripts/dev"
  cp "$0" "$main/scripts/dev/sweep-run.sh"
  sw=$main/scripts/dev/sweep-run.sh
  echo .wt >"$main/.git/info/exclude"
  printf 'one\n' >"$main/a.txt"
  g "$main" add a.txt scripts && g "$main" commit -q -m base
  for c in dirty untracked unlanded busy cherry squash fresh; do
    git clone -q --no-local "$main" "$main/.wt/$c"
  done
  printf 'edit\n' >>"$main/.wt/dirty/a.txt"
  : >"$main/.wt/untracked/new.txt"
  # unlanded: a commit main lacks
  printf 'x\n' >"$main/.wt/unlanded/u.txt"
  g "$main/.wt/unlanded" add u.txt && g "$main/.wt/unlanded" commit -q -m unlanded
  # busy: a landed clone with a process inside
  printf 'b\n' >"$main/.wt/busy/b.txt"
  g "$main/.wt/busy" add b.txt && g "$main/.wt/busy" commit -q -m busy
  g "$main" fetch -q "$main/.wt/busy" HEAD && g "$main" cherry-pick FETCH_HEAD >/dev/null
  (cd "$main/.wt/busy" && exec sleep 60) &
  local busy=$!
  # cherry: land the commit by cherry-pick, as land.sh does
  printf 'c\n' >"$main/.wt/cherry/c.txt"
  g "$main/.wt/cherry" add c.txt && g "$main/.wt/cherry" commit -q -m cherry
  g "$main" fetch -q "$main/.wt/cherry" HEAD && g "$main" cherry-pick FETCH_HEAD >/dev/null
  # squash: two commits in the clone land as one commit on main, which cherry cannot match
  printf 'two\n' >"$main/.wt/squash/a.txt"
  g "$main/.wt/squash" commit -q -am s1
  printf 'three\n' >>"$main/.wt/squash/a.txt"
  g "$main/.wt/squash" commit -q -am s2
  cp "$main/.wt/squash/a.txt" "$main/a.txt"
  g "$main" commit -q -am squashed
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
  bash "$sw" >/dev/null
  [ ! -d "$main/.wt/squash" ] || st_fail "the squash-landed clone was kept"
  for c in dirty untracked unlanded busy fresh; do
    [ -d "$main/.wt/$c" ] || st_fail "a real run removed red case $c"
  done
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
