#!/usr/bin/env bash
# commit-dates.sh as a reference-transaction hook: an update to main that brings in a commit whose
# author date is not its commit date, or is older than its parent, is refused on every path
# (commit, update-ref, fast-forward merge). Ordinary commits pass, and the override passes.
#
#   bash hooks/commit-dates_test.sh
#   HOOK=/path/to/always-pass.sh bash hooks/commit-dates_test.sh   # must FAIL
set -uo pipefail
unset GIT_DIR GIT_INDEX_FILE GIT_WORK_TREE GIT_PREFIX GIT_COMMON_DIR SCHWARZWALD_ALLOW_OLD_DATES
HOOK="${HOOK:-$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/commit-dates.sh}"
HOOK="$(cd "$(dirname "${HOOK}")" && pwd)/$(basename "${HOOK}")"
. "$(dirname "${BASH_SOURCE[0]}")/../tools/test-sandbox.sh"
sandbox_make commit-dates-test
sandbox_enter --multi
d=$SANDBOX_REPO
git -C "$d" init -q -b main
mkdir -p "$d/.git/hooks"
printf '#!/usr/bin/env bash\nexec bash %q "$@"\n' "$HOOK" >"$d/.git/hooks/reference-transaction"
chmod +x "$d/.git/hooks/reference-transaction"
cd "$d" || exit 1
g() { git -c user.name=t -c user.email=t@example.invalid "$@"; }
fail=0
# lands <label> <want yes|no> <cmd...>: run cmd, then compare whether the branch REF (default
# main) moved
lands() {
  local label=$1 want=$2 ref=${REF:-refs/heads/main} before after got
  shift 2
  before=$(git rev-parse -q --verify "$ref" || echo none)
  "$@" >/dev/null 2>&1
  after=$(git rev-parse -q --verify "$ref" || echo none)
  got=no
  [[ $before != "$after" ]] && got=yes
  if [[ $got == "$want" ]]; then echo "ok   $label"; else
    echo "FAIL $label ($ref moved: $got, want $want)"
    fail=1
  fi
}
echo a >a
lands "the first commit lands" yes g commit -qm one --allow-empty
lands "a commit dated now lands" yes g commit -qm two --allow-empty
tip=$(git rev-parse HEAD)
old=$(GIT_AUTHOR_DATE="2026-01-01T10:00:00" g commit-tree "$tip^{tree}" -p "$tip" -m lane)
lands "update-ref to a commit with an old author date is refused" no git update-ref refs/heads/main "$old"
inv=$(GIT_AUTHOR_DATE="2001-01-01T00:00:00" GIT_COMMITTER_DATE="2001-01-01T00:00:00" g commit-tree "$tip^{tree}" -p "$tip" -m inverted)
lands "a commit dated before its parent is refused" no git update-ref refs/heads/main "$inv"
git branch -q lane "$old"
lands "a fast-forward merge of an old-dated commit is refused" no git merge -q --ff-only lane
lands "the override lets a deliberate import through" yes env SCHWARZWALD_ALLOW_OLD_DATES=1 git update-ref refs/heads/main "$old"
lands "a commit dated now lands after it" yes g commit -qm three --allow-empty
mkdir -p config/schwarzwald
printf '[commit]\nallow_prefix = "PRODUCT_"\n' >config/schwarzwald/lint.toml
tip=$(git rev-parse HEAD)
old2=$(GIT_AUTHOR_DATE="2026-01-01T10:00:00" g commit-tree "$tip^{tree}" -p "$tip" -m lane2)
lands "the product prefix names the override" yes env PRODUCT_ALLOW_OLD_DATES=1 git update-ref refs/heads/main "$old2"
git branch -q other "$tip"
REF=refs/heads/other lands "another branch is not judged" yes git update-ref refs/heads/other "$old"
exit "$fail"
