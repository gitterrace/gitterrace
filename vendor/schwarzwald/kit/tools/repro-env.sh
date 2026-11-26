#!/usr/bin/env bash
# repro-env.sh: print the `export` lines of a reproducible cargo build. Evaluate them in bash:
#
#   eval "$(tools/repro-env.sh [--repo DIR] [--root DIR] [--epoch N] [X.Y.Z | TAG])"
#
# What it exports:
#   SOURCE_DATE_EPOCH  release-date.sh --epoch for the version or tag given, else the committer time
#                      of HEAD (an untagged build). --epoch N takes the value as it is (repro-check
#                      computes it once and hands it to both copies). The clock is never read.
#   RUSTFLAGS          the existing value, then these appended, never replaced:
#                        --remap-path-prefix  $HOME                   -> /home/user
#                                             the rustup toolchains   -> /rustup/toolchains
#                                             $CARGO_HOME/registry    -> /cargo/registry
#                                             $CARGO_HOME/git         -> /cargo/git
#                                             the repo root           -> /build
#                      rustc lets the last matching remap win, so the specific ones come after
#                      $HOME and the root comes last. A flag already present is not added twice,
#                      so nesting this script is safe. -C debuginfo is the product's choice and is
#                      not touched. When CARGO_ENCODED_RUSTFLAGS is set (cargo prefers it over
#                      RUSTFLAGS) the flags are appended to it too, 0x1f separated.
#   CARGO_INCREMENTAL=0, TZ=UTC, LC_ALL=C
#
# The root is --root, else the git toplevel of --repo (default the cwd), else the cwd.
# Exit: 0, 2 on a setup error (a path with whitespace cannot go into RUSTFLAGS) with a remedy line.
set -euo pipefail

die() { # die <what failed> <remedy>
  echo "repro-env: $1" >&2
  echo "repro-env: remedy: $2" >&2
  exit 2
}

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
repo=""
root=""
epoch=""
name=""
while [ $# -gt 0 ]; do
  case $1 in
  --repo) repo=${2:?--repo needs a directory}; shift ;;
  --root) root=${2:?--root needs a directory}; shift ;;
  --epoch) epoch=${2:?--epoch needs a number}; shift ;;
  -h | --help) sed -n '2,26p' "${BASH_SOURCE[0]}"; exit 0 ;;
  -*) die "unknown option $1" "usage: repro-env.sh [--repo DIR] [--root DIR] [--epoch N] [X.Y.Z | TAG]" ;;
  *) name=$1 ;;
  esac
  shift
done

repo=${repo:-$PWD}
[ -d "${repo}" ] || die "no directory ${repo}" "pass an existing --repo"
if [ -z "${root}" ]; then
  root=$(git -C "${repo}" rev-parse --show-toplevel 2>/dev/null || echo "${repo}")
fi
root=$(cd "${root}" && pwd -P)

if [ -z "${epoch}" ]; then
  if [ -n "${name}" ]; then
    epoch=$(bash "${here}/release-date.sh" --repo "${repo}" --epoch "${name}") ||
      die "release-date.sh found no date for ${name}" "fetch the tags, or build an untagged tree without a version"
  else
    epoch=$(git -C "${repo}" log -1 --format=%ct HEAD 2>/dev/null) ||
      die "no commit at HEAD in ${repo}" "commit first, or pass --epoch N"
  fi
fi
case ${epoch} in '' | *[!0-9]*) die "bad epoch '${epoch}'" "an epoch is whole seconds" ;; esac

cargo_home=${CARGO_HOME:-${HOME}/.cargo}
rustup_home=${RUSTUP_HOME:-${HOME}/.rustup}
remaps=(
  "${HOME}=/home/user"
  "${rustup_home}/toolchains=/rustup/toolchains"
  "${cargo_home}/registry=/cargo/registry"
  "${cargo_home}/git=/cargo/git"
  "${root}=/build"
)
flags=()
for r in "${remaps[@]}"; do
  case ${r} in *[[:space:]]*) die "path with whitespace: ${r%%=*}" "build from a checkout and a HOME without spaces" ;; esac
  flags+=("--remap-path-prefix=${r}")
done

append() { # append <existing flags> <separator>: print them, then the flags not yet in them
  local cur=$1 sep=$2 out=$1 f
  for f in "${flags[@]}"; do
    case "${sep}${cur}${sep}" in *"${sep}${f}${sep}"*) ;; *) out=${out:+${out}${sep}}${f} ;; esac
  done
  printf '%s' "${out}"
}

printf 'export SOURCE_DATE_EPOCH=%q\n' "${epoch}"
printf 'export RUSTFLAGS=%q\n' "$(append "${RUSTFLAGS-}" " ")"
if [ -n "${CARGO_ENCODED_RUSTFLAGS-}" ]; then
  printf 'export CARGO_ENCODED_RUSTFLAGS=%q\n' "$(append "${CARGO_ENCODED_RUSTFLAGS}" $'\x1f')"
fi
echo 'export CARGO_INCREMENTAL=0'
echo 'export TZ=UTC'
echo 'export LC_ALL=C'
