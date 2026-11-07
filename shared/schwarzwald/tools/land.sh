#!/usr/bin/env bash
# Land one checked commit from a clone onto the local main of a repository.
#
#   land.sh [--full-gate|--no-gate] <repo> <clone-dir> <sha>        (or LAND_REPO=<repo> land.sh <clone-dir> <sha>)
#   land.sh [--full-gate|--no-gate] --plan <repo> <clone-dir> <base> <plan-file>
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
# Plan mode: plan lines are `<sha> [fold]` (fold squashes into the previous commit and keeps its
# message). The clone's HEAD must be the plan's last sha and <base> must be main's tip, else the
# plan is stale. The commits are rebuilt on <base> in a temporary clone with author date =
# committer date = landing time, +1 s per commit, one identity from git config. The rebuilt tree
# must equal the clone's HEAD tree. Then the product's gate runs in the rebuilt clone: the
# config/schwarzwald/land.toml `[landing]` table, `gate = [cmds]` and `messages = "cmd {base}"`
# ({base} is the real base sha), each through bash -c. No file or no [landing] table prints
# "landing gate: none configured" and refuses unless --no-gate is given. The product's full scripts/land-check.sh runs only with --full-gate (as the gate, and as
# `--messages <base>`). A red result refuses. Then main fast-forwards, refusing a dirty checkout
# or a non-ff.
set -euo pipefail
plan_mode=0 full_gate=0 no_gate=0
while [[ ${1:-} == --full-gate || ${1:-} == --no-gate || ${1:-} == --plan ]]; do
  case $1 in --full-gate) full_gate=1 ;; --no-gate) no_gate=1 ;; --plan) plan_mode=1 ;; esac; shift
done
usage() { sed -n '2,30s/^# \{0,1\}//p' "$0" >&2; exit 1; }
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
[[ $clone == /* ]] || clone="$PWD/$clone"
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
# landing_cfg <product-dir>: print the [landing] gate commands, a "--" marker, then the messages
# command (with {base} left in), NUL separated. Nothing without the file.
landing_cfg() {
  [[ -f $1/config/schwarzwald/land.toml ]] || return 0
  "$py" -c '
import sys, tomllib
c = tomllib.load(open(sys.argv[1], "rb")).get("landing")
if c is None: sys.exit(0)
for g in c.get("gate", []): sys.stdout.write(g + "\0")
sys.stdout.write("--\0" + c.get("messages", "") + "\0")' "$1/config/schwarzwald/land.toml"
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
guard_config_value config/schwarzwald/guards.env MAIN_CHECKOUT_LANDS_ONLY
[ "$GUARD_CONFIG_VALUE" = 1 ] && lands_only=1
common="$(git rev-parse --git-common-dir)"
[[ $common == /* ]] || common="$PWD/$common"
tipfile="$common/land-tip" ledger="$common/landed"
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
  base=$(git rev-parse "$base_arg^{commit}")
  [ "$base" = "$(git rev-parse HEAD)" ] || { echo "refused: stale plan, base $(git rev-parse --short "$base") is not main's tip $(git rev-parse --short HEAD)" >&2; exit 1; }
  lastsha=$(awk 'NF {s=$1} END {print s}' "$plan")
  [ -n "$lastsha" ] && [ "$(git -C "$clone" rev-parse "$lastsha^{commit}")" = "$old" ] || { echo "refused: stale plan, it ends at ${lastsha:-nothing}, the clone HEAD is $(git rev-parse --short "$old")" >&2; exit 1; }
  while read -r s _; do
    [ -z "$s" ] && continue
    subj=$(git -C "$clone" log -1 --format=%s "$s")
    if partial "$subj"; then echo "refused: $s '$subj' is a partial step. Squash it into its change." >&2; exit 3; fi
  done <"$plan"
  tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
  git clone -q "$clone" "$tmp/w"
  git -C "$tmp/w" fetch -q "$repo" "$base"
  if [ -f .personal-denylist ]; then cp .personal-denylist "$tmp/w/"; fi
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
      git cherry-pick -n "$s" >/dev/null 2>&1 || { echo "conflict: $s does not apply on $(git rev-parse --short "$base")" >&2; exit 2; }
    else
      git cherry-pick "$s" >/dev/null 2>&1 || { echo "conflict: $s does not apply on $(git rev-parse --short "$base")" >&2; exit 2; }
      stamp="$t $zone"; t=$((t + 1))
    fi
    GIT_AUTHOR_DATE=$stamp GIT_COMMITTER_DATE=$stamp git commit -q --amend --no-edit --no-verify --date="$stamp"
  done <"$plan"
  [ "$(git rev-parse 'HEAD^{tree}')" = "$(git -C "$clone" rev-parse "$old^{tree}")" ] || { echo "refused: the rebuilt tree differs from the plan's tree" >&2; exit 1; }
  gates=() msgcmd="" seen=0
  while IFS= read -r -d '' item; do
    if [ "$seen" = 0 ] && [ "$item" = -- ]; then seen=1; elif [ "$seen" = 0 ]; then gates+=("$item"); else msgcmd=$item; fi
  done < <(landing_cfg "$PWD")
  if [ "$seen" = 0 ]; then
    echo "landing gate: none configured (config/schwarzwald/land.toml [landing] is missing)" >&2
    [ "$no_gate" = 1 ] || { echo "refused: no landing gate. Add [landing] to config/schwarzwald/land.toml, or pass --no-gate." >&2; exit 1; }
  fi
  for g in "${gates[@]+"${gates[@]}"}"; do
    out=$(bash -c "$g" 2>&1) || { echo "refused: the product gate failed ($g):" >&2; printf '%s\n' "$out" | tail -5 >&2; exit 1; }
  done
  if [ -n "$msgcmd" ]; then
    out=$(bash -c "${msgcmd//\{base\}/$base}" 2>&1) || { echo "refused: the message gate failed:" >&2; printf '%s\n' "$out" | tail -5 >&2; exit 1; }
  fi
  if [ "$full_gate" = 1 ] && [ -f scripts/land-check.sh ]; then
    out=$(bash scripts/land-check.sh 2>&1) || { echo "refused: the full product gate failed:" >&2; printf '%s\n' "$out" | tail -5 >&2; exit 1; }
    out=$(bash scripts/land-check.sh --messages "$base" 2>&1) || { echo "refused: the message gate failed:" >&2; printf '%s\n' "$out" | tail -5 >&2; exit 1; }
  fi
  new=$(git rev-parse HEAD)
  cd "$repo"
  [ -z "$(git status --porcelain)" ] || { echo "main tree is dirty" >&2; exit 1; }
  git fetch -q "$tmp/w" "$new"
  git merge -q --ff-only "$new" >/dev/null 2>&1 || { echo "refused: not a fast-forward" >&2; exit 1; }
  git rev-list "$base..$new" >>"$ledger"
  if [ "$lands_only" = 1 ]; then git rev-parse HEAD >"$tipfile"; fi
  echo "landed plan as $(git rev-parse --short HEAD)"
  landed_log "$base..HEAD"
  exit 0
fi

git fetch -q "$clone" "$(git -C "$clone" rev-parse "$sha")" 2>/dev/null || git fetch -q "$clone"
full=$(git rev-parse "$sha^{commit}")
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
now=$(date +%s)
last=$(git log -1 --format=%ct)
[ "$now" -gt "$last" ] || now=$((last + 1))
zone=$(date +%z)
name=$(git log -1 --format=%an "$full") email=$(git log -1 --format=%ae "$full")
if ! git cherry-pick --no-commit "$full" >/dev/null 2>&1; then
  git cherry-pick --abort 2>/dev/null || git reset -q --hard "$before"
  echo "conflict: rebase $sha onto main $(git rev-parse --short HEAD) first" >&2; exit 2
fi
# The message goes through -F, not -C: -C would keep the old author date.
git log -1 --format=%B "$full" | LAND_TOOL=1 GIT_AUTHOR_NAME=$name GIT_AUTHOR_EMAIL=$email \
  GIT_COMMITTER_NAME=$name GIT_COMMITTER_EMAIL=$email \
  GIT_AUTHOR_DATE="$now $zone" GIT_COMMITTER_DATE="$now $zone" \
  git commit -q --no-verify --cleanup=whitespace -F -
new=$(git rev-parse --short HEAD)
[ "$(git rev-parse HEAD^)" = "$before" ] || { echo "unexpected parent" >&2; exit 1; }
git rev-parse HEAD >>"$ledger"
if [ "$lands_only" = 1 ]; then git rev-parse HEAD >"$tipfile"; fi
echo "landed $sha as $new"
landed_log -1
