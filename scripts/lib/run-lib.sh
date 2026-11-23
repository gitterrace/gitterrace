#!/usr/bin/env bash
# Shared by the dev run scripts and the docker check. Sourced, never run.
# Set `fail_prefix` before the first fail call (default: self-test).

# The channel in a file: rust-toolchain.toml form, or the Dockerfile's ARG form.
toolchain_pin() {
  sed -n -e 's/^channel *= *"\(.*\)"/\1/p' -e 's/^ARG RUST_TOOLCHAIN=\(.*\)/\1/p' "$1"
}

fail() {
  echo "${fail_prefix:-self-test}: $*" >&2
  exit 1
}

# parse_args NAME ARGS...: prints the mode (smoke, full or self-test) for the dev script NAME's
# arguments, or fails on an unknown one.
parse_args() {
  local name=$1
  shift
  if [ "$#" -le 1 ]; then
    case "${1:-}" in
    "") echo smoke && return ;;
    --full) echo full && return ;;
    --self-test) echo self-test && return ;;
    *) ;;
    esac
  fi
  echo "usage: scripts/dev/$name.sh [--full | --self-test]" >&2
  return 2
}

# parse_args_self_test NAME: the argument rules every run script keeps.
parse_args_self_test() {
  local name=$1
  [ "$(parse_args "$name")" = smoke ] || fail "no argument is not smoke"
  [ "$(parse_args "$name" --full)" = full ] || fail "--full is not full"
  [ "$(parse_args "$name" --self-test)" = self-test ] || fail "--self-test not parsed"
  ! parse_args "$name" --bogus >/dev/null 2>&1 || fail "an unknown argument was accepted"
  ! parse_args "$name" --full --self-test >/dev/null 2>&1 || fail "two arguments were accepted"
}
