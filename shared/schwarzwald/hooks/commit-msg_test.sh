#!/usr/bin/env bash
# The commit-msg hook refuses a tracker code, a planning label, a process word, a home path, an
# own sha and a subject not in `type(scope): subject`, and lets a plain message, a long hex run
# and a stripped comment line through. It runs in a throwaway repository with no config, so the
# default types apply.
#
#   bash hooks/commit-msg_test.sh
#   HOOK=/path/to/always-pass.sh bash hooks/commit-msg_test.sh   # must FAIL
set -euo pipefail
# Run from a hook, GIT_DIR and GIT_INDEX_FILE are set: the scratch repo would be the real one.
unset GIT_DIR GIT_INDEX_FILE GIT_WORK_TREE GIT_PREFIX GIT_COMMON_DIR
HOOK="${HOOK:-$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/commit-msg}"
HOOK="$(cd "$(dirname "${HOOK}")" && pwd)/$(basename "${HOOK}")"  # absolute: the cases cd away
. "$(dirname "${BASH_SOURCE[0]}")/../tools/test-sandbox.sh"
sandbox_make commit-msg-test
sandbox_enter --multi
d=$SANDBOX_REPO
git -C "${d}" init -q
git -C "${d}" -c user.name=t -c user.email=t@example.invalid commit -q --allow-empty -m "init"
own=$(git -C "${d}" rev-parse --short=9 HEAD)
export COMMIT_MSG_REPO="${d}"

fail=0
# expect <label> <ok|refused> <message>
expect() {
  local label=$1 want=$2 msg=$3 got=ok rc
  printf '%s\n' "${msg}" >"${d}/msg"
  bash "${HOOK}" "${d}/msg" >/dev/null 2>&1 && rc=0 || rc=$?
  [[ ${rc} == 0 ]] || got=refused
  if [[ ${got} == "${want}" ]]; then printf 'ok   %s\n' "${label}"; else
    printf 'FAIL %s (want %s, rc=%s)\n' "${label}" "${want}" "${rc}"
    fail=1
  fi
}

expect "a plain message passes" ok "fix(web): count code age through one rule"
expect "a code in the subject is refused" refused "fix(web): one strip (#cfdd68)"
expect "a code in the body is refused" refused $'fix: x\n\nSee #a1b2c3 for the ruling.'
expect "a code at the end of a line is refused" refused $'fix: x\n\nfollows #abcdef'
expect "a planning label is refused" refused $'fix(web): hint\n\nMoves G12.'
expect "a process word is refused" refused $'fix(web): hint\n\nAs the coordinator asked.'
expect "a story against an earlier state is refused" refused $'fix(web): hint\n\nThe page no longer lists ties.'
expect "a longer list passes" ok $'fix(web): hint\n\nThe page shows a longer list.'
home="/Users"
expect "a home path is refused" refused "$(printf 'fix: x\n\nbuilt in %s/jdoe/src' "${home}")"
expect "an own sha is refused" refused "$(printf 'fix: x\n\nfollows %s' "${own}")"
expect "a longer hex run is not a code" ok "chore: bump to deadbeefcafe"
expect "a stripped comment line is skipped" ok $'fix: x\n# Please enter the message #abcdef'
expect "a scopeless subject passes" ok "docs: state the commit form"
expect "a breaking-change marker passes" ok "feat(web)!: new route"
expect "a free-form subject is refused" refused "Empty states: the copy"
expect "an unknown type is refused" refused "wip: scrolling"
expect "a merge subject passes" ok "Merge branch 'x' into main"
expect "a leading comment line is skipped" ok $'# note\nfix: x'

exit "${fail}"
