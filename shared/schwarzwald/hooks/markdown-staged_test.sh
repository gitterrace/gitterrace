#!/usr/bin/env bash
# markdown-staged.sh lints only staged markdown that is not excluded, with the pinned version,
# and stays quiet when nothing is staged, nothing is pinned or the linter passes. A stub npx on
# PATH records its arguments and fails, so no network or node is needed.
#
#   bash hooks/markdown-staged_test.sh
#   HOOK=/usr/bin/true bash hooks/markdown-staged_test.sh   # must FAIL
set -u
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
hook="${HOOK:-${here}/markdown-staged.sh}"
. "${here}/../tools/test-sandbox.sh"
sandbox_make markdown-staged-test
sandbox_enter
mkdir -p config/schwarzwald vendor
cat >"${SANDBOX}/bin/npx" <<'S'
#!/bin/sh
echo "$*" >>"${STUB_LOG}"
echo "Linting: ${STUB_FILES:-1} file(s)"
echo "a.md:1 MD013 line too long"
exit "${STUB_RC:-1}"
S
chmod +x "${SANDBOX}/bin/npx"
export STUB_LOG="${SANDBOX}/npx.log"
export PATH="${SANDBOX}/bin:${PATH}"

fail=0
# expect <label> <rc wanted> <args wanted in the stub log, or ->
expect() {
  local rc=0
  : >"${STUB_LOG}"
  bash "${hook}" >/dev/null 2>&1 || rc=1
  if [[ ${rc} != "$2" ]]; then
    echo "  FAIL $1 (rc ${rc})"
    fail=1
  elif [[ $3 != - ]] && ! grep -qx -- "$3" "${STUB_LOG}"; then
    echo "  FAIL $1 (npx called with: $(cat "${STUB_LOG}"))"
    fail=1
  elif [[ $3 == - && -s ${STUB_LOG} ]]; then
    echo "  FAIL $1 (npx ran: $(cat "${STUB_LOG}"))"
    fail=1
  else
    echo "  ok   $1"
  fi
}
printf '[run]\nmarkdownlint = "9.9.9"\n' >config/schwarzwald/lint.toml
printf '# a\n' >a.md && printf '# v\n' >vendor/v.md
git add a.md vendor/v.md config/schwarzwald/lint.toml && git commit -qm base
expect "nothing staged passes without running npx" 0 -
printf '# a2\n' >a.md && git add a.md
expect "a staged file is linted with the pinned version and refused on findings" 1 "--yes markdownlint-cli2@9.9.9 -- ./a.md"
STUB_RC=0 expect "a clean lint passes" 0 "--yes markdownlint-cli2@9.9.9 -- ./a.md"
STUB_RC=0 STUB_FILES=0 expect "a tool that lints 0 files for a staged list is refused" 1 "--yes markdownlint-cli2@9.9.9 -- ./a.md"
printf '# h\n' >'#h.md' && git add '#h.md'
STUB_RC=0 expect "a name the tool would read as a glob goes after -- as ./" 0 "--yes markdownlint-cli2@9.9.9 -- ./#h.md ./a.md"
git rm -q --cached '#h.md'
nonode=$(mktemp -d "${SANDBOX}/nonode.XXXXXX")
for b in bash git awk mktemp cat grep head; do ln -s "$(command -v "${b}")" "${nonode}/${b}"; done
PATH="${nonode}" CI=true expect "no npx under CI is refused" 1 -
PATH="${nonode}" CI="" expect "no npx on a laptop is a skip" 0 -
printf '[run]\nmarkdownlint = "npm:other-pkg"\n' >config/schwarzwald/lint.toml
expect "a pin that is not x.y.z is refused without running npx" 1 -
printf '[run]\nmarkdownlint = ""\n' >config/schwarzwald/lint.toml
git reset -q && git add a.md
expect "no pinned version turns the hook off" 0 -
exit "${fail}"
