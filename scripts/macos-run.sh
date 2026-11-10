#!/usr/bin/env bash
# Install git-terrace on this Mac and run scripts/verify/smoke-check.sh against it in a clean
# environment:
# HOME is an empty temp dir and PATH is /usr/bin:/bin plus the install dir, so a dependency that
# only exists on the dev machine fails here. Usage: scripts/macos-run.sh [--full | --self-test].
# Default: release install plus the smoke test. --full: make check under the same clean HOME, with
# the cargo and rustup homes and the toolchain on PATH, since the build cannot work without them.
# --self-test: argument parsing and the clean environment, no build.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)

# Prints the mode for the arguments, or fails on an unknown one.
parse_args() {
  if [ "$#" -gt 1 ]; then
    echo "usage: scripts/macos-run.sh [--full | --self-test]" >&2
    return 2
  fi
  case "${1:-}" in
  "") echo smoke ;;
  --full) echo full ;;
  --self-test) echo self-test ;;
  *)
    echo "usage: scripts/macos-run.sh [--full | --self-test]" >&2
    return 2
    ;;
  esac
}

# Runs a command with nothing inherited but HOME, PATH and the given extras.
clean_env() {
  local home=$1 search=$2
  shift 2
  env -i HOME="$home" PATH="$search" TMPDIR="${TMPDIR:-/tmp}" "$@"
}

fail() {
  echo "self-test: $*" >&2
  exit 1
}

self_test() {
  [ "$(parse_args)" = smoke ] || fail "no argument is not smoke"
  [ "$(parse_args --full)" = full ] || fail "--full is not full"
  [ "$(parse_args --self-test)" = self-test ] || fail "--self-test not parsed"
  ! parse_args --bogus >/dev/null 2>&1 || fail "an unknown argument was accepted"
  ! parse_args --full --self-test >/dev/null 2>&1 || fail "two arguments were accepted"
  local home out
  home=$(mktemp -d)
  # shellcheck disable=SC2016  # the child shell expands these, not this one
  out=$(DEV_ONLY_VALUE=1 clean_env "$home" /usr/bin:/bin sh -c 'printf %s "$HOME|$PATH|${DEV_ONLY_VALUE:-}"')
  [ "$out" = "$home|/usr/bin:/bin|" ] || fail "the clean environment leaks or drops values: $out"
  [ -z "$(ls -A "$home")" ] || fail "the clean HOME is not empty"
  rmdir "$home"
  echo "macos-run self-test: ok"
}

mode=$(parse_args "$@") || exit 2
if [ "$mode" = self-test ]; then
  self_test
  exit 0
fi

[ "$(uname -s)" = Darwin ] || {
  echo "macos-run: this runs on macOS only, use scripts/linux-run.sh on Linux." >&2
  exit 1
}

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
mkdir "$work/home" "$work/install"
cargo_home=${CARGO_HOME:-$HOME/.cargo}
rustup_home=${RUSTUP_HOME:-$HOME/.rustup}
tool=$(dirname "$(command -v cargo)")

# Only the build sees the toolchain. The smoke test gets the minimal PATH.
CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$root/target} cargo install --locked --path "$root/crates/gitterrace" \
  --root "$work/install" --quiet
bin=$work/install/bin/git-terrace
[ -x "$bin" ] || {
  echo "macos-run: missing $bin" >&2
  exit 1
}

if [ "$mode" = full ]; then
  (cd "$root" && clean_env "$work/home" "$tool:$work/install/bin:/usr/bin:/bin" \
    CARGO_HOME="$cargo_home" RUSTUP_HOME="$rustup_home" make check)
fi
clean_env "$work/home" "$work/install/bin:/usr/bin:/bin" bash "$root/scripts/verify/smoke-check.sh" "$bin" 4261
