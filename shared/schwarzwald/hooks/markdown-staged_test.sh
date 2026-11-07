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
printf '[run]\nmarkdownlint = "9.9.9"\nmarkdown_exclude = ["vendor", "red-cases"]\n' >config/schwarzwald/lint.toml
printf '# a\n' >a.md && printf '# v\n' >vendor/v.md
git add a.md vendor/v.md config/schwarzwald/lint.toml && git commit -qm base
expect "nothing staged passes without running npx" 0 -
printf '# a2\n' >a.md && git add a.md
expect "a staged file is linted with the pinned version and refused on findings" 1 "--yes markdownlint-cli2@9.9.9 a.md"
STUB_RC=0 expect "a clean lint passes" 0 "--yes markdownlint-cli2@9.9.9 a.md"
git reset -q && printf '# v2\n' >vendor/v.md && git add vendor/v.md
expect "an excluded path is not linted" 0 -
printf '[run]\nmarkdownlint = ""\n' >config/schwarzwald/lint.toml
git reset -q && git add a.md
expect "no pinned version turns the hook off" 0 -
exit "${fail}"
