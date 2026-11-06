#!/bin/sh
# Every external linter, one PASS or FAIL line each. A missing tool is a FAIL with its install
# line, never a skip. Run by `make lint`. Config lives at the root: _typos.toml, .vale.ini,
# .gitleaks.toml, .markdownlint-cli2.jsonc, ruff.toml, pyrightconfig.json, jsconfig.json.
#
#   sh scripts/lint-run.sh
set -eu
# pipefail is POSIX since 2024 but older dash lacks it, so it is set only where supported.
# shellcheck disable=SC3040
(set -o pipefail) 2>/dev/null && set -o pipefail
cd "$(dirname "$0")/.."

fails=0

# run NAME TOOL INSTALL CMD...: run CMD when TOOL exists, report the verdict.
run() {
  name=$1 tool=$2 install=$3
  shift 3
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "FAIL $name: $tool is not installed ($install)"
    fails=$((fails + 1))
    return
  fi
  if out=$("$@" 2>&1); then
    echo "PASS $name"
  else
    printf '%s\n' "$out"
    echo "FAIL $name"
    fails=$((fails + 1))
  fi
}

# Vale needs its proselint package in styles/ (git-ignored), fetched once.
vale_run() {
  [ -d styles/proselint ] || vale sync >/dev/null || return 1
  # Tracked files only: `.` would walk .wt/ worktrees and target/.
  git ls-files -z -- '*.md' '*.rs' ':!:vendor/**' ':!:shared/schwarzwald/**' |
    xargs -0 vale --minAlertLevel=error
}

# tsc and the node types, pinned, installed once into target/js-types (git-ignored). The
# scripts load playwright at run time by path, so its API is untyped here on purpose.
js_types() {
  t=target/js-types/node_modules
  [ -x "$t/.bin/tsc" ] && [ -d "$t/@types/node" ] ||
    npm install --silent --no-save --no-package-lock --prefix target/js-types \
      typescript@5.9.3 @types/node@24.10.1 >/dev/null || return 1
  "$t/.bin/tsc" -p jsconfig.json
}

# Every tracked shell script, the docs/actions examples included. -o all minus: SC2250 (braces
# on every $var) and SC2292 ([[ ]] in bash) are style, SC2310 and SC2312 flag the deliberate
# `check || fail` pattern the test scripts are built on.
sh_files() {
  # The synced copy carries its own checks.
  git ls-files -z -- '*.sh' ':!:shared/**' | xargs -0 "$@"
}

run typos typos "brew install typos-cli" typos
run vale vale "brew install vale" vale_run
run gitleaks gitleaks "brew install gitleaks" gitleaks detect --no-banner -c .gitleaks.toml
run markdownlint npx "install node, then npx fetches markdownlint-cli2" npx --yes markdownlint-cli2@0.24.0
run shellcheck shellcheck "brew install shellcheck" sh_files shellcheck -o all -e SC2250,SC2292,SC2310,SC2312
run shfmt shfmt "brew install shfmt" sh_files shfmt -d -i 2
run ruff-check ruff "brew install ruff" ruff check scripts
run ruff-format ruff "brew install ruff" ruff format --check scripts
run pyright npx "install node, then npx fetches pyright" npx --yes pyright@1.1.407
run tsc npm "install node, then npm fetches typescript" js_types
run actionlint actionlint "brew install actionlint" actionlint
run actions-pin python3 "install python3" python3 scripts/actions-pin-check.py
run actions-pin-test python3 "install python3" python3 scripts/actions-pin-check.py --self-test
run ast-grep ast-grep "brew install ast-grep" ast-grep scan --error
run ast-grep-test ast-grep "brew install ast-grep" ast-grep test --skip-snapshot-tests
run ast-grep-macro python3 "install python3" python3 scripts/ast-grep-macro-check.py
run ast-grep-macro-test python3 "install python3" python3 scripts/ast-grep-macro-check.py --self-test
run super-ratchet python3 "install python3" python3 scripts/super-check.py
run super-ratchet-test python3 "install python3" python3 scripts/super-check.py --self-test
run tests-cfg python3 "install python3" python3 scripts/tests-check.py
run css-scope python3 "install python3" python3 scripts/css-scope-check.py
run site-csp python3 "install python3" python3 scripts/site-check.py csp
run machete cargo-machete "cargo install cargo-machete" cargo-machete
# The tree carries no task ids, labels, process words, personal data or own commits: the shared
# lints (shared/schwarzwald/lint) with config/schwarzwald/lint.toml, each proved on its red case.
# They need python 3.11 or newer for tomllib.
py=python3
for c in python3 python3.15 python3.14 python3.13 python3.12 python3.11; do
  if command -v "$c" >/dev/null 2>&1 && "$c" -c 'import sys; sys.exit(sys.version_info < (3, 11))'; then
    py=$c
    break
  fi
done
sw=shared/schwarzwald
run schwarzwald-copy "$py" "install python 3.11" "$py" "$sw"/check.py
run internal-refs "$py" "install python 3.11" "$py" "$sw"/lint/internal-refs.py
run personal-data "$py" "install python 3.11" "$py" "$sw"/lint/personal-data.py
run own-shas "$py" "install python 3.11" "$py" "$sw"/lint/own-shas.py
run lint-red "$py" "install python 3.11" "$py" "$sw"/lint/red.py --repo .
run commit-msg-hook bash "install bash" bash "$sw"/hooks/commit-msg_test.sh
run commit-msg-shim bash "install bash" env HOOK=scripts/hooks/commit-msg bash "$sw"/hooks/commit-msg_test.sh
run guards bash "install bash" bash "$sw"/guards/tests/suite.sh

[ "$fails" -eq 0 ] || {
  echo "lint: $fails failed"
  exit 1
}
echo "lint: all passed"
