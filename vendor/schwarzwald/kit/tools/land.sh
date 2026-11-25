#!/usr/bin/env bash
# Land one checked commit from a clone onto the local main of a repository.
#
#   land.sh [--full-gate|--no-gate|--gate-change|--no-cache] <repo> <clone-dir> <sha>        (or LAND_REPO=<repo> land.sh <clone-dir> <sha>)
#   land.sh [--full-gate|--no-gate|--gate-change|--no-cache] --plan <repo> <clone-dir> <base> <plan-file>
#
# Cherry-picks the commit onto main, stamps author and committer date to now (one second after
# main's tip if the clock is behind), keeps the message and the author identity. The message gets
# no trailer: each landed sha is appended to <git-common-dir>/landed, a local file never committed.
# Refuses a dirty main, a commit since .git/land-tip that is not in that file (opt-in, see below),
# a range, a conflict, and a subject that is a partial step (rules/deny.toml [partial_subject],
# read from the copy beside this part). A change already on main is skipped.
# A commit whose parent is not main's tip is refused ("rebase onto main"). The last line printed is
# the landed log: sha, author|committer identity, the dates and the subject.
# Exit: 0 landed or already there, 1 refused, 2 conflict, 3 partial step.
#
# Both modes build the result in a temporary clone and run the product's gate there before main
# moves, by a fast-forward only, so a red gate leaves main untouched.
#
# Plan mode: plan lines are `<sha> [fold]` (fold squashes into the previous commit and keeps its
# message). The clone's HEAD must be the plan's last sha and <base> must be main's tip, else the
# plan is stale. The commits are rebuilt on <base> in a temporary clone with author date =
# committer date = landing time, +1 s per commit, one identity from git config. The rebuilt tree
# must equal the clone's HEAD tree.
#
# The gate, in both modes: the
# schwarzwald/config/land.toml `[landing]` table, `gate = [cmds]` and `messages = "cmd {base}"`
# ({base} is the real base sha), each through bash -c. No file or no [landing] table prints
# "landing gate: none configured" and refuses unless --no-gate is given. --no-gate only allows a
# missing gate: a configured [landing] gate always runs, so a landing cannot skip it. A red gate
# prints its FAIL and error lines, then its last 10 lines. The product's full
# scripts/land-check.sh runs only with --full-gate (as the gate, and as
# `--messages <base>`). A red result refuses. Then main fast-forwards, refusing a dirty checkout
# or a non-ff. The gate (land.toml and land-check.sh) is read from main's tip in <repo>, never
# from the commits being landed, so a candidate cannot set its own gate. A landing that changes
# either file is refused unless --gate-change is given. With --gate-change the candidate's own gate
# judges the result tree, after the full diff of both files is printed: a landing that moves the
# gate's own scripts (a kit move) can land, an empty candidate gate is refused, and every command of
# main's gate whose files all still exist in the result runs too, so a candidate cannot drop a
# check by rewriting the list.
#
# Gate cache: a green run of the whole gate is kept in <git-common-dir>/land-cache/, keyed by the
# result tree, the commands, the kit version, the toolchain and the gate env. A hit skips the gate
# and prints one note with the cached run's date and log. Red runs are never kept, an entry expires
# after 24h, and a command named in land.toml `[landing] fresh = ["cmd"]` (time-dependent, such as
# an advisory check) is never cached. --no-cache always runs the gate.
#
# Refusals are remembered. A gate that sends a tip back records it with
#   land.sh --sent-back <repo> <sha> <reason>
# (a local ref refs/gate/sent-back/<tree>, never pushed), and land.sh refuses any landing whose
# result tree carries one, under any commit. One gate per clone: <repo>/.git/GATE-OWNER holds the
# landing's session (LAND_SESSION, else the pid) while it runs, and a second land.sh on the same
# repository is refused unless the holder's pid is gone.
#
# After a sanctioned history rewrite of main that keeps the content (a reword):
#   land.sh --rewritten <repo> <old-tip> <new-tip>
# moves land-tip to the new tip and lists the rewritten commits in the landed ledger. It refuses
# unequal trees, a new tip that is not main, and an old tip that is not land-tip.
set -euo pipefail
# --sent-back <repo> <sha> <reason>: record a refused tip, then exit. Keyed by the tree, so the
# same content re-offered under another commit is refused too.
if [[ ${1:-} == --sent-back ]]; then
  [[ $# == 4 ]] || { echo "usage: land.sh --sent-back <repo> <sha> <reason>" >&2; exit 1; }
  [[ $3 =~ ^[0-9a-fA-F]{7,64}$ ]] || { echo "refused: '$3' is not a commit id (7 to 64 hex digits)" >&2; exit 1; }
  tree=$(git -C "$2" rev-parse --verify --end-of-options "$3^{tree}") || exit 1
  blob=$(printf 'sent back %s: %s\n' "$3" "$4" | git -C "$2" hash-object -w --stdin)
  git -C "$2" update-ref "refs/gate/sent-back/$tree" "$blob"
  echo "recorded: tree $tree is sent back ($4)"
  exit 0
fi
# --rewritten <repo> <old-tip> <new-tip>: after a sanctioned rewrite of main (a reword, the owner's
# OK), move land-tip to the new tip and record it as landed. Only a rewrite that keeps the content
# is accepted: the two trees must be equal, and the new tip must be main.
if [[ ${1:-} == --rewritten ]]; then
  [[ $# == 4 ]] || { echo "usage: land.sh --rewritten <repo> <old-tip> <new-tip>" >&2; exit 1; }
  for x in "$3" "$4"; do
    [[ $x =~ ^[0-9a-fA-F]{7,64}$ ]] || { echo "refused: '$x' is not a commit id (7 to 64 hex digits)" >&2; exit 1; }
  done
  old=$(git -C "$2" rev-parse --verify --end-of-options "$3^{commit}") || exit 1
  new=$(git -C "$2" rev-parse --verify --end-of-options "$4^{commit}") || exit 1
  [ "$(git -C "$2" rev-parse "$old^{tree}")" = "$(git -C "$2" rev-parse "$new^{tree}")" ] ||
    { echo "refused: the trees of $3 and $4 differ, so this is not a message-only rewrite" >&2; exit 1; }
  [ "$(git -C "$2" rev-parse main)" = "$new" ] || { echo "refused: $4 is not main's tip" >&2; exit 1; }
  c=$(git -C "$2" rev-parse --git-common-dir); [[ $c == /* ]] || c="$2/$c"
  if [ -f "$c/land-tip" ] && [ "$(cat "$c/land-tip")" != "$old" ]; then
    echo "refused: land-tip is $(cat "$c/land-tip"), not the old tip $3" >&2; exit 1
  fi
  echo "$new" >"$c/land-tip"
  touch "$c/landed"
  add=$(git -C "$2" rev-list "$new" | grep -vxFf "$c/landed" || true)
  [ -z "$add" ] || printf '%s\n' "$add" >>"$c/landed"
  echo "land-tip moved from $(git -C "$2" rev-parse --short "$old") to $(git -C "$2" rev-parse --short "$new")"
  exit 0
fi
plan_mode=0 full_gate=0 no_gate=0 gate_change=0 no_cache=0
while [[ ${1:-} == --full-gate || ${1:-} == --no-gate || ${1:-} == --plan || ${1:-} == --gate-change || ${1:-} == --no-cache ]]; do
  case $1 in --full-gate) full_gate=1 ;; --no-gate) no_gate=1 ;; --plan) plan_mode=1 ;; --gate-change) gate_change=1 ;; --no-cache) no_cache=1 ;; esac; shift
done
usage() { sed -n '2,33s/^# \{0,1\}//p' "$0" >&2; exit 1; }
GATE_FILES=(schwarzwald/config/land.toml scripts/land-check.sh)
if [[ $plan_mode == 0 ]]; then
  if [[ -n ${LAND_REPO:-} && $# == 2 ]]; then set -- "${LAND_REPO}" "$@"; fi
  [[ $# == 3 ]] || usage
  repo=$1 clone=$2 sha=$3
else
  if [[ -n ${LAND_REPO:-} && $# == 3 ]]; then set -- "${LAND_REPO}" "$@"; fi
  [[ $# == 4 ]] || usage
  repo=$1 clone=$2 base_arg=$3 plan=$4
  [[ $plan == /* ]] || plan="$PWD/$plan"
fi
# A relative <repo> or <clone> is resolved here, before any cd: after the cd into the temporary
# clone "." would name the clone, and the landing would never reach main.
repo=$(cd "$repo" 2>/dev/null && pwd) || { echo "refused: no repository at '$repo'" >&2; exit 1; }
clone=$(cd "$clone" 2>/dev/null && pwd) || { echo "refused: no clone at '$clone'" >&2; exit 1; }
# Every rev a caller or a plan names is a hex object id, checked before any git call sees it: a
# word starting with - would be taken as an option (--upload-pack=, --output=).
is_oid() { [[ $1 =~ ^[0-9a-fA-F]{7,64}$ ]]; }
if [[ $plan_mode == 0 ]]; then
  is_oid "$sha" || { echo "refused: '$sha' is not a commit id (7 to 64 hex digits)" >&2; exit 1; }
else
  is_oid "$base_arg" || { echo "refused: base '$base_arg' is not a commit id (7 to 64 hex digits)" >&2; exit 1; }
  [[ -f $plan ]] || { echo "refused: no plan file $plan" >&2; exit 1; }
  while read -r s mode extra; do
    [ -z "$s" ] && continue
    is_oid "$s" || { echo "refused: plan line '$s' is not a commit id (7 to 64 hex digits)" >&2; exit 1; }
    [[ -z $mode || $mode == fold ]] || { echo "refused: plan line '$s $mode': unknown mode '$mode', want fold or nothing" >&2; exit 1; }
    [[ -z $extra ]] || { echo "refused: plan line '$s $mode $extra': unknown mode, too many words" >&2; exit 1; }
  done <"$plan"
fi
deny="$(cd "$(dirname "${BASH_SOURCE[0]}")/../rules" && pwd)/deny.toml"
# The [partial_subject] patterns are Python regexes: read them with the first python3 >= 3.11.
py=""
for c in python3 python3.15 python3.14 python3.13 python3.12 python3.11; do
  if command -v "$c" >/dev/null && "$c" -c 'import sys; sys.exit(sys.version_info < (3, 11))'; then py=$c; break; fi
done
[[ -n $py ]] || { echo "land.sh needs python 3.11 or newer to read deny.toml" >&2; exit 1; }
partial() { # partial <subject>: succeeds when the subject is a partial step
  "$py" -c '
import re, sys, tomllib
pats = tomllib.load(open(sys.argv[1], "rb"))["partial_subject"]["patterns"]
sys.exit(0 if any(re.search(p, sys.argv[2]) for p in pats) else 1)' "$deny" "$1"
}

# landed_log <git log args...>: sha, author|committer identity, the dates, the subject
landed_log() { git log --format='%h %an <%ae>|%cn <%ce> %ad|%cd %s' --date=iso "$@"; }
# landing_cfg <land.toml>: print the [landing] gate commands, a "--" marker, the messages command
# (with {base} left in), a "--" marker, then the time-dependent `fresh` commands, NUL separated. Nothing for an empty or missing file.
landing_cfg() {
  [[ -s $1 ]] || return 0
  "$py" -c '
import sys, tomllib
c = tomllib.load(open(sys.argv[1], "rb")).get("landing")
if c is None: sys.exit(0)
for g in c.get("gate", []): sys.stdout.write(g + "\0")
sys.stdout.write("--\0" + c.get("messages", "") + "\0--\0")
for g in c.get("fresh", []): sys.stdout.write(g + "\0")' "$1"
}

# gate_out <text>: a red gate's FAIL and error lines (the failing check), then its last 10 lines.
gate_out() {
  printf '%s\n' "$1" | grep -E '(^|[[:space:]])(FAIL|ERROR|error)([:[:space:]]|$)' >&2 || true
  printf '%s\n' "$1" | tail -10 >&2
}
# gate_env <cmd...>: run a gate command without the identity and dates this tool sets for its
# own commits. A gate that makes fixture commits would otherwise get the lander's identity and
# different shas than on the product's own runs.
gate_env() {
  # Config from the environment (GIT_CONFIG_COUNT with its KEY_n/VALUE_n, GIT_CONFIG_PARAMETERS)
  # is dropped too: it would override the fixture's own config, core.hooksPath among others.
  local -a drop=(-u GIT_CONFIG_COUNT -u GIT_CONFIG_PARAMETERS)
  local v
  while IFS= read -r v; do drop+=(-u "$v"); done < <(compgen -e | grep -E '^GIT_CONFIG_(KEY|VALUE)_[0-9]+$' || true)
  env "${drop[@]}" -u GIT_AUTHOR_NAME -u GIT_AUTHOR_EMAIL -u GIT_AUTHOR_DATE \
    -u GIT_COMMITTER_NAME -u GIT_COMMITTER_EMAIL -u GIT_COMMITTER_DATE -u LAND_TOOL "$@"
}
# make_retired <make args...>: main's `make` gate command without the targets the result's
# Makefile no longer has, so a gate change can retire a target. Flags, their values and VAR=value
# words stay, and each target is probed with them. Only make's own "No rule to make target" drops
# a target: a Makefile that fails to parse keeps every target, so its gate still runs and fails.
# Prints the command and a note per dropped target, and fails when no target is left.
make_retired() {
  local a err flags=() targets=() kept=() takes=""
  for a in "$@"; do
    if [ -n "$takes" ]; then flags+=("$a"); takes=""; continue; fi
    case $a in
      -C | -f | -I | -o | -W | --directory | --file | --include-dir) flags+=("$a"); takes=1 ;;
      -* | *=* | [0-9]*) flags+=("$a") ;;
      *) targets+=("$a") ;;
    esac
  done
  for a in "${targets[@]+"${targets[@]}"}"; do
    err=$(make "${flags[@]+"${flags[@]}"}" -n -q "$a" 2>&1 >/dev/null) || true
    case $err in
      *"No rule to make target"*) echo "gate change: main's make target $a is skipped, the result's Makefile has no rule for it" >&2 ;;
      *) kept+=("$a") ;;
    esac
  done
  [ "${#kept[@]}" -gt 0 ] || return 1
  echo "make ${flags[*]+${flags[*]} }${kept[*]}"
}

# The gate cache: a green run of the whole gate is kept in <git-common-dir>/land-cache/<key>,
# a file with the run's epoch and date and a .log beside it. The key covers the result tree, the
# commit messages (the message gate reads them), every command, the full-gate flag and
# land-check.sh, the kit version (this file and its manifest.json), the toolchain (rustc,
# xcodebuild, the python) and the gate env allowlist. An entry older than 24h misses.
CACHE_TTL=86400
cache_dir() {
  local c
  c=$(git -C "$repo" rev-parse --git-common-dir)
  [[ $c == /* ]] || c="$repo/$c"
  echo "$c/land-cache"
}
cache_key() { # cache_key <base> <gate-changed>: a hash of everything that judged the tree
  local here
  here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
  {
    git rev-parse 'HEAD^{tree}'
    git log --format=%B "$1..HEAD" | git hash-object --stdin
    printf 'full=%s change=%s\n' "$full_gate" "$2"
    printf 'gate=%s\n' "${gates[@]+"${gates[@]}"}"
    printf 'messages=%s\n' "$msgcmd"
    if [ -s scripts/land-check.sh ]; then git hash-object scripts/land-check.sh; fi
    git hash-object "$here/land.sh"
    if [ -f "$here/../manifest.json" ]; then git hash-object "$here/../manifest.json"; fi
    rustc -V 2>&1 || true
    if command -v xcodebuild >/dev/null; then xcodebuild -version 2>&1 || true; fi
    "$py" -V 2>&1
    env | grep -E '^(CARGO|RUST|RUSTUP|RUSTFLAGS|PYTHON|CC|CXX|CFLAGS|CXXFLAGS|LDFLAGS|MAKEFLAGS|MACOSX_DEPLOYMENT_TARGET|SDKROOT|DEVELOPER_DIR)[A-Z_]*=' | LC_ALL=C sort || true
  } | git hash-object --stdin
}
cache_hit() { # cache_hit <entry>: succeeds, with one note, for a fresh entry
  local f=$1 ts=0 when="" now
  [ -f "$f" ] || return 1
  { read -r ts; read -r when; } <"$f" || true
  [[ $ts =~ ^[0-9]+$ ]] || return 1
  now=$(date +%s)
  { [ $((now - ts)) -lt "$CACHE_TTL" ] && [ "$ts" -le "$now" ]; } || return 1
  echo "gate cached: the same tree passed this gate on $when (log $f.log). --no-cache runs it again." >&2
}
cache_store() { # cache_store <dir> <key> <log>
  mkdir -p "$1"
  printf '%s' "$3" >"$1/$2.log"
  printf '%s\n%s\n' "$(date +%s)" "$(date '+%Y-%m-%d %H:%M:%S %z')" >"$1/$2.tmp.$$"
  mv "$1/$2.tmp.$$" "$1/$2"
}

# run_gate <base>: the product's gate, in the current directory (a temporary clone holding the
# result exactly as main would get it). Exits refusing on a red gate, or on no gate without
# --no-gate.
run_gate() {
  local base=$1 item g out seen=0 msgcmd="" hooks gd changed f tree st=0
  local -a gates=() fresh=()
  # A tree a gate sent back stays refused, whatever commit carries it now.
  tree=$(git rev-parse 'HEAD^{tree}')
  if git -C "$repo" show-ref --verify -q "refs/gate/sent-back/$tree"; then
    echo "refused: a gate sent this tree back: $(git -C "$repo" cat-file blob "refs/gate/sent-back/$tree")" >&2
    echo "  Change it and send it again. To clear a wrong record: git update-ref -d refs/gate/sent-back/$tree" >&2
    exit 1
  fi
  # The gate is main's, read from the base commit in the target repository, never from the
  # result tree: a candidate that edits land.toml or land-check.sh would otherwise pick its gate.
  changed=$(git diff --name-only "$base" HEAD -- "${GATE_FILES[@]}")
  if [ -n "$changed" ] && [ "$gate_change" = 0 ]; then
    echo "refused: the landing changes the landing gate ($(echo "$changed" | tr '\n' ' ')). Pass --gate-change to land it, and the candidate's gate judges it." >&2; exit 1
  fi
  gd=$(mktemp -d "$tmp/gate.XXXX")
  for f in "${GATE_FILES[@]}"; do
    git -C "$repo" cat-file blob "$base:$f" >"$gd/${f##*/}" 2>/dev/null || : >"$gd/${f##*/}"
  done
  local cfg=$gd/land.toml
  if [ -n "$changed" ]; then
    # The candidate's own gate judges a gate change, shown in full first: a reviewer reads exactly
    # what judged the landing. Main's commands still run where their first file still exists.
    echo "gate change: the candidate's gate judges this landing. The gate files differ from main:" >&2
    git diff "$base" HEAD -- "${GATE_FILES[@]}" >&2
    cfg=schwarzwald/config/land.toml
  fi
  # A fresh clone has no core.hooksPath. A gate that checks the hooks are wired would refuse every
  # landing, so the target repository's setting is carried over. It is set here, after the
  # landing commits, so the hooks judge only what the gate runs.
  hooks=$(gate_env git -C "$repo" config --get core.hooksPath 2>/dev/null || true)
  [ -z "$hooks" ] || git config core.hooksPath "$hooks"
  while IFS= read -r -d '' item; do
    case $st in
      0) if [ "$item" = -- ]; then seen=1 st=1; else gates+=("$item"); fi ;;
      1) msgcmd=$item st=2 ;;
      2) st=3 ;;
      *) fresh+=("$item") ;;
    esac
  done < <(landing_cfg "$cfg")
  if [ -n "$changed" ]; then
    [ "${#gates[@]}" -gt 0 ] || { echo "refused: the candidate's gate is empty, so nothing would judge this gate change" >&2; exit 1; }
    # Every command of main's gate whose files the result still has runs as well: a candidate
    # cannot drop a check by rewriting the list, only by moving a file it runs. Every word with a
    # slash counts: a runner script that stayed put must not run a script that moved.
    local mitem mseen=0 mcmd w x mst=0
    local -a ws=()
    while IFS= read -r -d '' mitem; do
      case $mst in
        0) if [ "$mitem" = -- ]; then mseen=1 mst=1; continue; fi ;;
        1) mst=2; continue ;;
        2) mst=3; continue ;;
        *) continue ;;
      esac
      for mcmd in "${gates[@]}"; do [ "$mcmd" = "$mitem" ] && continue 2; done
      read -ra ws <<<"$mitem"
      w=""
      for x in "${ws[@]}"; do case $x in */*) [ -e "$x" ] || { w=$x; break; } ;; esac; done
      if [ -n "$w" ]; then echo "gate change: main's '$mitem' is skipped, $w is not in the result" >&2; continue; fi
      if [ "${ws[0]:-}" = make ]; then
        mitem=$(make_retired "${ws[@]:1}") || { echo "gate change: main's '$mitem' is skipped, the result's Makefile has none of its targets" >&2; continue; }
      fi
      gates+=("$mitem")
    done < <(landing_cfg "$gd/land.toml")
  fi
  if [ "$seen" = 0 ]; then
    echo "landing gate: none configured (schwarzwald/config/land.toml [landing] is missing)" >&2
    [ "$no_gate" = 1 ] || { echo "refused: no landing gate. Add [landing] to schwarzwald/config/land.toml, or pass --no-gate." >&2; exit 1; }
  fi
  # A green run of the whole gate is cached by what judged it (see cache_key). Only a gate with
  # something to run is cached, and never one that names a time-dependent (`fresh`) command.
  local ckey="" clog="" cdir
  if [ "$no_cache" = 0 ] && { [ "${#gates[@]}" -gt 0 ] || [ -n "$msgcmd" ]; }; then
    ckey=$(cache_key "$base" "$changed")
    for g in "${gates[@]+"${gates[@]}"}" ${msgcmd:+"$msgcmd"}; do
      for f in "${fresh[@]+"${fresh[@]}"}"; do [ "$g" != "$f" ] || { ckey=""; break 2; }; done
    done
  fi
  cdir=$(cache_dir)
  if [ -n "$ckey" ] && cache_hit "$cdir/$ckey"; then return 0; fi
  for g in "${gates[@]+"${gates[@]}"}"; do
    out=$(gate_env bash -c "$g" 2>&1) || { echo "refused: the product gate failed ($g):" >&2; gate_out "$out"; exit 1; }
    clog+="== $g"$'\n'"$out"$'\n'
  done
  if [ -n "$msgcmd" ]; then
    out=$(gate_env bash -c "${msgcmd//\{base\}/$base}" 2>&1) || { echo "refused: the message gate failed:" >&2; gate_out "$out"; exit 1; }
    clog+="== messages"$'\n'"$out"$'\n'
  fi
  # The tree's own land-check.sh runs in place. For a gate change that is the candidate's, like
  # its land.toml. Without one, main must have had one for --full-gate to run anything.
  local check=scripts/land-check.sh run_full=0
  if [ -n "$changed" ]; then [ -s "$check" ] && run_full=1; else [ -s "$gd/land-check.sh" ] && run_full=1; fi
  if [ "$full_gate" = 1 ] && [ "$run_full" = 1 ]; then
    out=$(gate_env bash "$check" 2>&1) || { echo "refused: the full product gate failed:" >&2; gate_out "$out"; exit 1; }
    clog+="== land-check.sh"$'\n'"$out"$'\n'
    out=$(gate_env bash "$check" --messages "$base" 2>&1) || { echo "refused: the message gate failed:" >&2; gate_out "$out"; exit 1; }
    clog+="== land-check.sh --messages"$'\n'"$out"$'\n'
  fi
  [ -z "$ckey" ] || cache_store "$cdir" "$ckey" "$clog"
}

cd "$repo"
[ "$(git symbolic-ref --short HEAD)" = main ] || { echo "not on main" >&2; exit 1; }
[ -z "$(git status --porcelain)" ] || { echo "main tree is dirty" >&2; exit 1; }
# Opt-in (MAIN_CHECKOUT_LANDS_ONLY=1 in the product's guards.env): every commit since the last
# landed tip (.git/land-tip) must be listed in <git-common-dir>/landed, since --no-verify skips
# the hook. The ledger is local and never committed, so a landed message carries nothing extra.
# Read through the guards' own loader, so every spelling the config shape allows counts.
lands_only=0
# shellcheck source=/dev/null
. "$(dirname "${BASH_SOURCE[0]}")/../guards/config.sh" || { echo "land.sh cannot load guards/config.sh beside it" >&2; exit 1; }
guard_config_value schwarzwald/config/guards.env MAIN_CHECKOUT_LANDS_ONLY
[ "$GUARD_CONFIG_VALUE" = 1 ] && lands_only=1
common="$(git rev-parse --git-common-dir)"
[[ $common == /* ]] || common="$PWD/$common"
tipfile="$common/land-tip" ledger="$common/landed"
# One gate per clone: a second landing on the same repository waits for nobody, it is refused.
# The lock is taken with noclobber (O_EXCL) and released on exit. A holder whose pid is gone left
# it by a crash, so its lock is taken over.
owner="$common/GATE-OWNER" locked=0 tmp=""
cleanup() { [ -z "$tmp" ] || rm -rf "$tmp"; [ "$locked" = 0 ] || rm -f "$owner"; }
trap cleanup EXIT
take_lock() { ( set -C; printf '%s %s\n' "$$" "${LAND_SESSION:-pid}" >"$owner" ) 2>/dev/null; }
if ! take_lock; then
  read -r opid osess <"$owner" || opid=""
  if [[ $opid =~ ^[0-9]+$ ]] && kill -0 "$opid" 2>/dev/null; then
    echo "refused: another gate holds this repository ($osess, pid $opid, $owner). One gate per clone." >&2; exit 1
  fi
  rm -f "$owner"
  take_lock || { echo "refused: could not take $owner" >&2; exit 1; }
fi
locked=1
if [ "$lands_only" = 1 ] && [ -f "$tipfile" ]; then
  tip=$(cat "$tipfile")
  if git cat-file -e "$tip^{commit}" 2>/dev/null; then
    for c in $(git rev-list "$tip..HEAD"); do
      grep -qx "$c" "$ledger" 2>/dev/null || {
        echo "refused: $(git rev-parse --short "$c") sits between land-tip and HEAD and is not in the landed ledger. Move it to a worktree, then reset main to $(git rev-parse --short "$tip")." >&2; exit 1; }
    done
  fi
fi

if [ "$plan_mode" = 1 ]; then
  old=$(git -C "$clone" rev-parse HEAD)
  base=$(git rev-parse --verify --end-of-options "$base_arg^{commit}")
  [ "$base" = "$(git rev-parse HEAD)" ] || { echo "refused: stale plan, base $(git rev-parse --short "$base") is not main's tip $(git rev-parse --short HEAD)" >&2; exit 1; }
  lastsha=$(awk 'NF {s=$1} END {print s}' "$plan")
  [ -n "$lastsha" ] && [ "$(git -C "$clone" rev-parse --verify --end-of-options "$lastsha^{commit}")" = "$old" ] || { echo "refused: stale plan, it ends at ${lastsha:-nothing}, the clone HEAD is $(git rev-parse --short "$old")" >&2; exit 1; }
  while read -r s _; do
    [ -z "$s" ] && continue
    subj=$(git -C "$clone" log -1 --format=%s --end-of-options "$s")
    if partial "$subj"; then echo "refused: $s '$subj' is a partial step. Squash it into its change." >&2; exit 3; fi
  done <"$plan"
  tmp=$(mktemp -d)
  git clone -q "$clone" "$tmp/w"
  git -C "$tmp/w" fetch -q "$repo" "$base"
  name=$(git -C "$clone" config user.name) email=$(git -C "$clone" config user.email)
  export GIT_AUTHOR_NAME=$name GIT_AUTHOR_EMAIL=$email GIT_COMMITTER_NAME=$name GIT_COMMITTER_EMAIL=$email LAND_TOOL=1
  cd "$tmp/w"
  git checkout -q --detach "$base"
  t=$(date +%s); b=$(git log -1 --format=%ct "$base"); zone=$(date +%z)
  [ "$t" -gt "$b" ] || t=$((b + 1))
  stamp=""
  while read -r s mode _; do
    [ -z "$s" ] && continue
    if [ "$mode" = fold ]; then
      git cherry-pick -n --end-of-options "$s" >/dev/null 2>&1 || { echo "conflict: $s does not apply on $(git rev-parse --short "$base")" >&2; exit 2; }
    else
      git cherry-pick --end-of-options "$s" >/dev/null 2>&1 || { echo "conflict: $s does not apply on $(git rev-parse --short "$base")" >&2; exit 2; }
      stamp="$t $zone"; t=$((t + 1))
    fi
    GIT_AUTHOR_DATE=$stamp GIT_COMMITTER_DATE=$stamp git commit -q --amend --no-edit --no-verify --date="$stamp"
  done <"$plan"
  [ "$(git rev-parse 'HEAD^{tree}')" = "$(git -C "$clone" rev-parse "$old^{tree}")" ] || { echo "refused: the rebuilt tree differs from the plan's tree" >&2; exit 1; }
  run_gate "$base"
  new=$(git rev-parse HEAD)
  cd "$repo"
  [ -z "$(git status --porcelain)" ] || { echo "main tree is dirty" >&2; exit 1; }
  git fetch -q "$tmp/w" "$new"
  git merge -q --ff-only "$new" >/dev/null 2>&1 || { echo "refused: not a fast-forward" >&2; exit 1; }
  [ "$(git -C "$repo" rev-parse main)" = "$new" ] || { echo "refused: main did not move to $(git rev-parse --short "$new"), nothing recorded" >&2; exit 1; }
  git rev-list "$base..$new" >>"$ledger"
  if [ "$lands_only" = 1 ]; then git rev-parse HEAD >"$tipfile"; fi
  echo "landed plan as $(git rev-parse --short HEAD)"
  landed_log "$base..HEAD"
  exit 0
fi

git fetch -q -- "$clone" "$(git -C "$clone" rev-parse --verify --end-of-options "$sha^{commit}")" 2>/dev/null || git fetch -q -- "$clone"
full=$(git rev-parse --verify --end-of-options "$sha^{commit}")
[ "$(git rev-list --count "$full^!")" = 1 ] || { echo "not a single commit" >&2; exit 1; }
# One change, one commit: a fixup, a re-bless or a style pass belongs inside its change.
subj=$(git log -1 --format=%s "$full")
if partial "$subj"; then
  echo "refused: '$subj' is a partial step. Squash it into its change and send that." >&2; exit 3
fi
# Skip if the same change is already on main.
pid=$(git show "$full" | git patch-id --stable | cut -d' ' -f1)
if git log --format=%H -200 main | xargs -I{} sh -c 'git show {} | git patch-id --stable' | cut -d' ' -f1 | grep -qx "$pid"; then
  echo "already on main: $sha"; exit 0
fi
before=$(git rev-parse HEAD)
git merge-base --is-ancestor "$before" "$full" || { echo "refused: $sha is not based on main's tip $(git rev-parse --short HEAD), rebase onto main first" >&2; exit 1; }
# The landing cherry-picks one commit: a parent that is not main's tip would be dropped silently.
parent=$(git rev-parse "$full^")
[ "$parent" = "$before" ] || { echo "refused: $sha has an ancestor not on main: $(git rev-parse --short "$parent") sits between main's tip $(git rev-parse --short "$before") and it. Land that first, or send a rebased single commit." >&2; exit 1; }
now=$(date +%s)
last=$(git log -1 --format=%ct)
[ "$now" -gt "$last" ] || now=$((last + 1))
zone=$(date +%z)
name=$(git log -1 --format=%an "$full") email=$(git log -1 --format=%ae "$full")
# The landing is built and gated in a temporary clone of main; main moves only by a fast-forward
# once the gate is green, so a red gate leaves it untouched.
tmp=$(mktemp -d)
git clone -q --no-hardlinks "$repo" "$tmp/w"
cd "$tmp/w"
git fetch -q "$repo" "$full"
git checkout -q --detach "$before"
if ! git cherry-pick --no-commit "$full" >/dev/null 2>&1; then
  echo "conflict: rebase $sha onto main $(git -C "$repo" rev-parse --short HEAD) first" >&2; exit 2
fi
# The message goes through -F, not -C: -C would keep the old author date.
git log -1 --format=%B "$full" | LAND_TOOL=1 GIT_AUTHOR_NAME=$name GIT_AUTHOR_EMAIL=$email \
  GIT_COMMITTER_NAME=$name GIT_COMMITTER_EMAIL=$email \
  GIT_AUTHOR_DATE="$now $zone" GIT_COMMITTER_DATE="$now $zone" \
  git commit -q --no-verify --cleanup=whitespace -F -
[ "$(git rev-parse HEAD^)" = "$before" ] || { echo "unexpected parent" >&2; exit 1; }
run_gate "$before"
landed=$(git rev-parse HEAD)
cd "$repo"
[ -z "$(git status --porcelain)" ] || { echo "main tree is dirty" >&2; exit 1; }
git fetch -q "$tmp/w" "$landed"
git merge -q --ff-only "$landed" >/dev/null 2>&1 || { echo "refused: main moved during the gate, not a fast-forward" >&2; exit 1; }
[ "$(git rev-parse main)" = "$landed" ] || { echo "refused: main did not move to $(git rev-parse --short "$landed"), nothing recorded" >&2; exit 1; }
new=$(git rev-parse --short HEAD)
git rev-parse HEAD >>"$ledger"
if [ "$lands_only" = 1 ]; then git rev-parse HEAD >"$tipfile"; fi
echo "landed $sha as $new"
landed_log -1
