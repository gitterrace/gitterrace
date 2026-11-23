#!/usr/bin/env bash
# stale-index.sh refuses a staged tree from an old index that undoes work HEAD has since, and
# passes honest work on a fresh index and unrelated work on an old one.
#
#   bash hooks/stale-index_test.sh
#   HOOK=/usr/bin/true bash hooks/stale-index_test.sh   # must FAIL
set -u
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
hook="${HOOK:-${here}/stale-index.sh}"
. "${here}/../tools/test-sandbox.sh"
sandbox_make stale-index-test
sandbox_enter
mkdir -p schwarzwald/config
printf '[commit]\nallow_prefix = "DEMO_"\n' >schwarzwald/config/lint.toml
fail=0
# expect <label> <rc wanted> [message file], with the environment the caller set
expect() {
  local rc=0
  bash "${hook}" ${3:+"$3"} >/dev/null 2>&1 || rc=1
  if [[ ${rc} == "$2" ]]; then
    echo "  ok   $1"
  else
    echo "  FAIL $1 (rc ${rc})"
    fail=1
  fi
}
printf 'one\n' >a && printf 'keep\n' >b && printf 'x\n' >u && git add a b u schwarzwald && git commit -qm base
printf 'l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\n' >land.sh && git add land.sh && git commit -qm land
stale=$(git write-tree)
printf 'two\n' >a && git commit -qam change
git rm -q b && git commit -qm drop
printf 'l1\nL2 fixed\nl3\nl4\nl5\nl6\nl7\nl8\n' >land.sh && git commit -qam fix-land
printf 'fresh\n' >added && printf 'too\n' >added2 && git add added added2 && git commit -qm add

git read-tree "${stale}"
expect "refuses a stale index that reverts a change and re-adds a deletion" 1
printf 'fix: x\n\nrestore: b on purpose\n' >"${SANDBOX}/msg"
expect "passes the same tree with a restore: line in the message" 0 "${SANDBOX}/msg"
printf 'fix: x\n\nno restore here\n' >"${SANDBOX}/msg2"
expect "a message without restore: still refuses" 1 "${SANDBOX}/msg2"
DEMO_ALLOW_RESTORE=1 expect "passes the same tree with the restore override" 0
SCHWARZWALD_ALLOW_RESTORE=1 expect "an override with another prefix does not count" 1
mv schwarzwald/config/lint.toml "${SANDBOX}/lint.toml"
mkdir -p config/schwarzwald
printf '[commit]\nallow_prefix = "DEMO_"\n' >config/schwarzwald/lint.toml
DEMO_ALLOW_RESTORE=1 expect "a lint.toml at the old config/schwarzwald/ path is not honoured" 1
rm -r config
mv "${SANDBOX}/lint.toml" schwarzwald/config/lint.toml
printf '[commit]\nallow_prefix = "a[$(touch %s/pwned)]b"\n' "${SANDBOX}" >schwarzwald/config/lint.toml
expect "a prefix holding a command substitution still refuses" 1
[[ -e ${SANDBOX}/pwned ]] && { echo "  FAIL the prefix ran a command"; fail=1; }
printf '[commit]\nallow_prefix = "DEMO_"\n' >schwarzwald/config/lint.toml

# Case 1 alone: the old index re-adds b, which HEAD cut, and carries HEAD's other paths.
git read-tree HEAD && git update-index --add --cacheinfo "100644,$(git rev-parse "${stale}:b"),b"
expect "refuses re-adding a file deleted since the index's base" 1
# Case 2: an old index edits land.sh on top of the base, losing HEAD's line 2 fix.
git read-tree "${stale}"
blob=$(printf 'l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8 lane edit\n' | git hash-object -w --stdin)
git update-index --cacheinfo "100644,${blob},land.sh"
expect "refuses an edit that reverts lines changed since the base" 1
# The second case it catches: an old index silently drops the files added since. One dropped
# path alone reads as an honest delete, so the drop needs a base that explains two or more.
git read-tree HEAD && git rm -q --cached added
expect "passes deleting a file on a fresh index" 0
git read-tree "${stale}" && git update-index --cacheinfo "100644,$(git rev-parse HEAD:a),a" &&
  git update-index --cacheinfo "100644,$(git rev-parse HEAD:land.sh),land.sh" && git rm -q --cached b
expect "refuses an old index that drops a file added since" 1

# Allowed: an old index touching only a path HEAD never changed since its base.
git read-tree "${stale}" && printf 'y\n' >u && git add u
expect "refuses nothing it should not: old index, unrelated path, still carries old paths" 1
git read-tree "${stale}"
for p in a land.sh added added2; do git update-index --add --cacheinfo "100644,$(git rev-parse "HEAD:${p}"),${p}"; done
git rm -q --cached b
printf 'y\n' >u && git add u
expect "passes an old index whose only change from HEAD is an unrelated path" 0
# The second defect it catches: one path alone put back to a blob a recent commit replaced, on an
# index that is otherwise fresh. No base explains it, so only the history of that path shows it.
git read-tree HEAD && git update-index --cacheinfo "100644,$(git rev-parse "${stale}:land.sh"),land.sh"
expect "refuses one path staged back to a blob a recent commit replaced" 1
DEMO_ALLOW_RESTORE=1 expect "passes that revert with the restore override" 0
# A path whose older blob equals HEAD's is not a revert, and neither is a new edit beside it.
printf 'v1\n' >r && git add r && git commit -qm r1
printf 'v2\n' >r && git commit -qam r2
printf 'v1\n' >r && git commit -qam r3
git read-tree HEAD && printf 'v3\n' >r && printf 'four\n' >a && git add r a
expect "passes a genuine new edit on paths with older blobs" 0
git read-tree HEAD && printf 'four\n' >a && git add a
expect "passes a path whose old blob equals HEAD's, left as it is" 0
git read-tree HEAD && printf 'three\n' >a && git add a
expect "passes an ordinary change on a fresh index" 0
git read-tree HEAD && git rm -q --cached a
expect "passes an ordinary deletion on a fresh index" 0
git read-tree HEAD && printf 'l1\nL2 fixed\nl3\nl4\nl5\nl6\nl7\nl8 more\n' >land.sh && git add land.sh
expect "passes an edit on top of HEAD's land.sh" 0
exit "${fail}"
