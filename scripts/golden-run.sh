#!/usr/bin/env bash
# Runs every test target that compares against a saved answer in tests/golden, so a landing
# checks the saved pages on the exact tree it lands. The list is found, not written down: any
# test file that reads a golden/ file is a target, so a new golden test is covered without a
# change here.
#
#   bash scripts/golden-run.sh            run them
#   bash scripts/golden-run.sh --list     print the cargo arguments only
#   bash scripts/golden-run.sh --self-test
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

# targets <root>: "-p <crate> --test <name>" for every crates/*/tests/*.rs that names golden/.
targets() {
  local root=$1 f crate
  while IFS= read -r f; do
    crate=$(basename "$(dirname "$(dirname "$f")")")
    printf -- '-p %s --test %s\n' "$crate" "$(basename "$f" .rs)"
  done < <(cd "$root" && grep -l 'golden/' crates/*/tests/*.rs 2>/dev/null | sort)
}

case ${1:-} in
--list)
  targets .
  ;;
--self-test)
  t=$(mktemp -d)
  trap 'rm -rf "$t"' EXIT
  mkdir -p "$t/crates/a/tests" "$t/crates/b/tests"
  printf 'const G: &str = "tests/golden/x.txt";\n' >"$t/crates/a/tests/pages.rs"
  printf 'fn plain() {}\n' >"$t/crates/a/tests/other.rs"
  got=$(targets "$t")
  [ "$got" = "-p a --test pages" ] || {
    echo "golden-run self-test FAILED: one target, got: $got"
    exit 1
  }
  printf 'let p = "golden/y.txt";\n' >"$t/crates/b/tests/new_golden.rs"
  got=$(targets "$t" | wc -l | tr -d ' ')
  [ "$got" = 2 ] || {
    echo "golden-run self-test FAILED: a new golden test is not found"
    exit 1
  }
  echo "golden-run self-test: ok"
  ;;
"")
  args=()
  while IFS= read -r line; do
    # shellcheck disable=SC2206
    args+=($line)
  done < <(targets .)
  [ ${#args[@]} -gt 0 ] || {
    echo "golden-run: no golden test found, refusing"
    exit 1
  }
  # land.sh stamps its own author and committer into the environment. The tests build histories
  # with their own identity, so the answers they save only match when those variables are gone.
  env -u GIT_AUTHOR_NAME -u GIT_AUTHOR_EMAIL -u GIT_AUTHOR_DATE \
    -u GIT_COMMITTER_NAME -u GIT_COMMITTER_EMAIL -u GIT_COMMITTER_DATE \
    cargo test -q "${args[@]}"
  ;;
*)
  echo "usage: golden-run.sh [--list|--self-test]" >&2
  exit 2
  ;;
esac
