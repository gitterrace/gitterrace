#!/usr/bin/env bash
# Land one checked commit from a clone onto the local main of a repository.
#
#   land.sh <repo> <clone-dir> <sha>        (or LAND_REPO=<repo> land.sh <clone-dir> <sha>)
#
# Cherry-picks the commit onto main, stamps author and committer date to now (one second after
# main's tip if the clock is behind), keeps the message and the author identity. Refuses a dirty
# main, a range, a conflict, and a subject that is a partial step (rules/deny.toml
# [partial_subject], read from the copy beside this part). A change already on main is skipped.
# The product's gate runs on the result; a red gate reverts the landing.
# Exit: 0 landed or already there, 1 refused, 2 conflict, 3 partial step.
set -euo pipefail
if [[ -n ${LAND_REPO:-} && $# == 2 ]]; then set -- "${LAND_REPO}" "$@"; fi
[[ $# == 3 ]] || { sed -n '2,11s/^# \{0,1\}//p' "$0" >&2; exit 1; }
repo=$1 clone=$2 sha=$3
deny="$(cd "$(dirname "${BASH_SOURCE[0]}")/../rules" && pwd)/deny.toml"
# The [partial_subject] patterns are Python regexes: read them with the first python3 >= 3.11.
py=""
for c in python3 python3.15 python3.14 python3.13 python3.12 python3.11; do
  if command -v "$c" >/dev/null && "$c" -c 'import sys; sys.exit(sys.version_info < (3, 11))'; then py=$c; break; fi
done
[[ -n $py ]] || { echo "land.sh needs python 3.11 or newer to read deny.toml" >&2; exit 1; }

cd "$repo"
[ "$(git symbolic-ref --short HEAD)" = main ] || { echo "not on main" >&2; exit 1; }
[ -z "$(git status --porcelain)" ] || { echo "main tree is dirty" >&2; exit 1; }
git fetch -q "$clone" "$(git -C "$clone" rev-parse "$sha")" 2>/dev/null || git fetch -q "$clone"
full=$(git rev-parse "$sha^{commit}")
[ "$(git rev-list --count "$full^!")" = 1 ] || { echo "not a single commit" >&2; exit 1; }
# One change, one commit: a fixup, a re-bless or a style pass belongs inside its change.
subj=$(git log -1 --format=%s "$full")
if ! "$py" - "$deny" "$subj" <<'EOF'; then
import re, sys, tomllib
pats = tomllib.load(open(sys.argv[1], "rb"))["partial_subject"]["patterns"]
sys.exit(1 if any(re.search(p, sys.argv[2]) for p in pats) else 0)
EOF
  echo "refused: '$subj' is a partial step. Squash it into its change and send that." >&2; exit 3
fi
# Skip if the same change is already on main.
pid=$(git show "$full" | git patch-id --stable | cut -d' ' -f1)
if git log --format=%H -200 main | xargs -I{} sh -c 'git show {} | git patch-id --stable' | cut -d' ' -f1 | grep -qx "$pid"; then
  echo "already on main: $sha"; exit 0
fi
before=$(git rev-parse HEAD)
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
git log -1 --format=%B "$full" | GIT_AUTHOR_NAME=$name GIT_AUTHOR_EMAIL=$email \
  GIT_COMMITTER_NAME=$name GIT_COMMITTER_EMAIL=$email \
  GIT_AUTHOR_DATE="$now $zone" GIT_COMMITTER_DATE="$now $zone" \
  git commit -q --no-verify --cleanup=whitespace -F -
new=$(git rev-parse --short HEAD)
[ "$(git rev-parse HEAD^)" = "$before" ] || { echo "unexpected parent" >&2; exit 1; }
echo "landed $sha as $new ($(git log -1 --format='%ad | %cd' --date=format:%H:%M:%S))"
