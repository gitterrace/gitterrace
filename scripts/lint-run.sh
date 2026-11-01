#!/bin/sh
# Every external linter, one PASS or FAIL line each. A missing tool is a FAIL with its install
# line, never a skip. Run by `make lint`. Config lives at the root: _typos.toml, .vale.ini,
# .gitleaks.toml, .markdownlint-cli2.jsonc, ruff.toml.
cd "$(dirname "$0")/.." || exit 1

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
  git ls-files -z -- '*.md' '*.rs' ':!:vendor/**' ':!:theme/schwarzwald/**' |
    xargs -0 vale --minAlertLevel=error
}

run typos typos "brew install typos-cli" typos
run vale vale "brew install vale" vale_run
run gitleaks gitleaks "brew install gitleaks" gitleaks detect --no-banner -c .gitleaks.toml
run markdownlint npx "install node, then npx fetches markdownlint-cli2" npx --yes markdownlint-cli2@0.24.0
run shellcheck shellcheck "brew install shellcheck" shellcheck scripts/*.sh
run shfmt shfmt "brew install shfmt" shfmt -d -i 2 scripts/*.sh
run ruff-check ruff "brew install ruff" ruff check scripts
run ruff-format ruff "brew install ruff" ruff format --check scripts
run machete cargo-machete "cargo install cargo-machete" cargo-machete

[ "$fails" -eq 0 ] || {
  echo "lint: $fails failed"
  exit 1
}
echo "lint: all passed"
