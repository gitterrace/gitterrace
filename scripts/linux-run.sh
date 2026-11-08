#!/usr/bin/env bash
# Build git-terrace for Linux in Docker and run scripts/smoke-check.sh against it, or the whole
# `make check` gate. Needs docker. Usage: scripts/linux-run.sh [--full | --self-test].
# Default: release build plus the smoke test. --full: make check, the ubuntu CI job's gate, then
# the release memory_budget test. --self-test: argument parsing and the toolchain pin, no docker.
# The image is native arch (arm64 on Apple silicon). The source is mounted read-only and copied
# into the container, since some linters write beside the files they read. Cargo's registry and
# target/ live in named volumes, so repeat runs are incremental.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
dockerfile=$root/scripts/docker/linux.Dockerfile

# Prints the mode for the arguments, or fails on an unknown one.
parse_args() {
  if [ "$#" -gt 1 ]; then
    echo "usage: scripts/linux-run.sh [--full | --self-test]" >&2
    return 2
  fi
  case "${1:-}" in
  "") echo smoke ;;
  --full) echo full ;;
  --self-test) echo self-test ;;
  *)
    echo "usage: scripts/linux-run.sh [--full | --self-test]" >&2
    return 2
    ;;
  esac
}

# The channel in a file: rust-toolchain.toml form, or the Dockerfile's ARG form.
toolchain_pin() {
  sed -n -e 's/^channel *= *"\(.*\)"/\1/p' -e 's/^ARG RUST_TOOLCHAIN=\(.*\)/\1/p' "$1"
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
  local want got
  want=$(toolchain_pin "$root/rust-toolchain.toml")
  got=$(toolchain_pin "$dockerfile")
  if [ -z "$want" ] || [ "$want" != "$got" ]; then
    fail "scripts/docker/linux.Dockerfile pins '$got', rust-toolchain.toml pins '$want'"
  fi
  echo "linux-run self-test: ok (toolchain $want)"
}

mode=$(parse_args "$@") || exit 2
if [ "$mode" = self-test ]; then
  self_test
  exit 0
fi

command -v docker >/dev/null 2>&1 || {
  echo "linux-run: docker is not installed or not on PATH. Install Docker Desktop or Colima first." >&2
  exit 1
}
docker info >/dev/null 2>&1 || {
  echo "linux-run: the docker daemon is not running." >&2
  exit 1
}

docker build -q -t gitterrace-linux -f "$dockerfile" "$root/scripts/docker" >/dev/null
run=(docker run --rm
  -v "$root:/src:ro"
  -v gitterrace-linux-cargo:/usr/local/cargo/registry
  -v gitterrace-linux-target:/work/target
  -e CARGO_BUILD_JOBS=3 -e CARGO_TERM_COLOR=never
  gitterrace-linux)
# A writable copy of the tree, without the host target/ (the volume holds the Linux one).
prep='tar -C /src --exclude=./target -cf - . | tar -C /work -xf - && '
if [ "$mode" = full ]; then
  "${run[@]}" bash -c "$prep"'make check && cargo test --locked --release -p gitterrace-web --test memory_budget'
else
  "${run[@]}" bash -c "$prep"'cargo build --locked --release -p gitterrace && bash scripts/smoke-check.sh target/release/git-terrace 4261'
fi
