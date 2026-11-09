#!/usr/bin/env bash
# The author guard refuses a commit whose author is not the configured identity, and lets a
# normal commit, an unconfigured checkout and the explicit override through. It runs in the
# throwaway repo from tools/test-sandbox.sh, with a lint.toml that sets the override prefix.
#
#   bash hooks/author-guard_test.sh
#   GUARD=/path/to/always-pass.sh bash hooks/author-guard_test.sh   # must FAIL
set -u
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GUARD="${GUARD:-${here}/author-guard.sh}"
. "${here}/../tools/test-sandbox.sh"
sandbox_make author-guard-test
sandbox_enter

mkdir -p .git/hooks schwarzwald/config
printf '[commit]\nallow_prefix = "DEMO_"\n' >schwarzwald/config/lint.toml
cp "${GUARD}" "${SANDBOX}/guard.sh"
chmod +x "${SANDBOX}/guard.sh"
printf '#!/bin/sh\nexec "%s"\n' "${SANDBOX}/guard.sh" >.git/hooks/pre-commit
chmod +x .git/hooks/pre-commit
git config core.hooksPath .git/hooks
git config user.email me@example.org
git config user.name me

fail=0
# expect <label> <ok|refused> <rc>
expect() {
  local label=$1 want=$2 rc=$3 got=ok
  [[ ${rc} == 0 ]] || got=refused
  if [[ ${got} == "${want}" ]]; then printf 'ok   %s\n' "${label}"; else
    printf 'FAIL %s (want %s, rc=%s)\n' "${label}" "${want}" "${rc}"
    fail=1
  fi
}
n=0
# commit <command...>: stage a fresh file and run the command
commit() {
  n=$((n + 1))
  echo "${n}" >"f${n}"
  git add "f${n}"
  "$@" >/dev/null 2>&1
}

rc=0
commit git commit -qm configured || rc=$?
expect "a commit by the configured identity passes" ok "${rc}"

rc=0
commit git -c user.email=t@t -c user.name=t commit -qm forged || rc=$?
expect "a commit forged through git -c is refused" refused "${rc}"
git reset -q --hard HEAD

rc=0
commit env GIT_AUTHOR_EMAIL=t@t GIT_AUTHOR_NAME=t git commit -qm forged-env || rc=$?
expect "a commit forged through GIT_AUTHOR_EMAIL is refused" refused "${rc}"
git reset -q --hard HEAD

rc=0
commit env DEMO_ALLOW_FOREIGN_AUTHOR=1 GIT_AUTHOR_EMAIL=other@example.org GIT_AUTHOR_NAME=other \
  git commit -qm foreign || rc=$?
expect "the override named by allow_prefix lets a foreign author through" ok "${rc}"

rc=0
commit env SCHWARZWALD_ALLOW_FOREIGN_AUTHOR=1 GIT_AUTHOR_EMAIL=other@example.org GIT_AUTHOR_NAME=other \
  git commit -qm wrong-prefix || rc=$?
expect "an override with another prefix does not count" refused "${rc}"
git reset -q --hard HEAD

# A prefix that is not a name falls back to the default: bash would run code in a subscript.
printf '[commit]\nallow_prefix = "a[$(touch %s/pwned)]b"\n' "${SANDBOX}" >schwarzwald/config/lint.toml
rc=0
commit git commit -qm hostile-prefix || rc=$?
[[ -e ${SANDBOX}/pwned ]] && rc=pwned
expect "a prefix holding a command substitution runs nothing" ok "${rc}"
printf '[commit]\nallow_prefix = "DEMO_"\n' >schwarzwald/config/lint.toml

# A lint.toml at the old config/schwarzwald/ path is not read: its prefix names no override.
mv schwarzwald/config/lint.toml "${SANDBOX}/lint.toml"
mkdir -p config/schwarzwald
printf '[commit]\nallow_prefix = "DEMO_"\n' >config/schwarzwald/lint.toml
rc=0
commit env DEMO_ALLOW_FOREIGN_AUTHOR=1 GIT_AUTHOR_EMAIL=other@example.org GIT_AUTHOR_NAME=other \
  git commit -qm old-path || rc=$?
expect "a lint.toml at the old path is not honoured" refused "${rc}"
rm -r config
mv "${SANDBOX}/lint.toml" schwarzwald/config/lint.toml

git config --unset user.email
git config --global --unset user.email
rc=0
commit env GIT_AUTHOR_EMAIL=anyone@example.org GIT_AUTHOR_NAME=anyone GIT_COMMITTER_EMAIL=anyone@example.org \
  GIT_COMMITTER_NAME=anyone git commit -qm unconfigured || rc=$?
expect "a checkout with no configured email has nothing to compare and passes" ok "${rc}"

exit "${fail}"
