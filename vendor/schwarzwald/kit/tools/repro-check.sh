#!/usr/bin/env bash
# repro-check.sh: prove a Linux release build is reproducible. Build twice, in two fresh copies of
# the tree at DIFFERENT absolute paths, with repro-env.sh applied, then compare the sha256 of the
# named outputs.
#
#   repro-check.sh [--version X.Y.Z|TAG] <product root> [-- build command...]
#
# The two paths are why this finds more than a second run does: it catches host paths in a binary
# (the class behind the manifest.json that once leaked a home directory into a release), a clock
# or a pid in the output, and an unpinned file order. A build that is only stable in one directory
# passes a rebuild in place and fails here.
#
# Config: <root>/schwarzwald/config/repro.toml
#   build   = "cargo build --release --locked --target {target}"   run with bash -c in each copy
#   outputs = ["target/{target}/release/tool"]                     globs, relative to the copy
#   targets = ["x86_64-unknown-linux-gnu"]                         {target} fills build and outputs
# A command after `--` replaces `build`. Without targets the build runs once with {target} empty.
# The copies take the whole tree, .git included, but not target/. Every other thing the build reads
# must be in the tree or in the toolchain. SOURCE_DATE_EPOCH is the committer time of HEAD, or the
# release date of --version, computed once for both copies. CARGO_TARGET_DIR is unset.
#
# Targets: Linux only. A darwin target is refused: codesign and ld64 stamp their own data, and
# macOS stays out until that is handled.
#
# On a difference it names the output and, when diffoscope is installed, prints its summary.
# Without it: the first differing offset, and the host-path strings found in either copy's file
# (/Users/, /home/ but not the /home/user remap target, and the copy's own path).
# Exit: 0 identical, 1 differing, 2 a setup error with a remedy line.
set -uo pipefail

die() { # die <what failed> <remedy>
  echo "repro-check: $1" >&2
  echo "repro-check: remedy: $2" >&2
  exit 2
}

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
version=""
root=""
cmd=()
while [ $# -gt 0 ]; do
  case $1 in
  --version) version=${2:?--version needs a value}; shift ;;
  -h | --help) sed -n '2,29p' "${BASH_SOURCE[0]}"; exit 0 ;;
  --) shift; cmd=("$@"); break ;;
  -*) die "unknown option $1" "usage: repro-check.sh [--version V] <product root> [-- build command]" ;;
  *) [ -z "${root}" ] || die "two roots given" "pass one product root"; root=$1 ;;
  esac
  shift
done
[ -n "${root}" ] || die "no product root" "usage: repro-check.sh [--version V] <product root> [-- build command]"
[ -d "${root}" ] || die "no directory ${root}" "pass the product root"
root=$(cd "${root}" && pwd -P)

PY=""
for cand in python3.15 python3.14 python3.13 python3.12 python3.11 python3; do
  if command -v "${cand}" >/dev/null 2>&1 && "${cand}" -c 'import tomllib' 2>/dev/null; then PY=${cand}; break; fi
done
[ -n "${PY}" ] || die "no python with tomllib (3.11 or newer)" "install python 3.11 or newer"
if command -v sha256sum >/dev/null 2>&1; then sha() { sha256sum "$1" | cut -d' ' -f1; }; else sha() { shasum -a 256 "$1" | cut -d' ' -f1; }; fi

cfg=${root}/schwarzwald/config/repro.toml
[ -f "${cfg}" ] || die "no ${cfg}" "write it: build, outputs, targets (see this script's header)"
build=""
outputs=()
targets=()
while IFS=$'\t' read -r kind val; do
  case ${kind} in
  error) die "cannot read ${cfg}: ${val}" "fix the TOML syntax" ;;
  build) build=${val} ;;
  output) outputs+=("${val}") ;;
  target) targets+=("${val}") ;;
  esac
done < <("${PY}" - "${cfg}" <<'PY'
import sys, tomllib
try:
    with open(sys.argv[1], "rb") as fh:
        c = tomllib.load(fh)
except (OSError, tomllib.TOMLDecodeError) as e:
    print(f"error\t{e}")
    sys.exit(0)
print("build\t" + str(c.get("build", "")))
for o in c.get("outputs", []):
    print("output\t" + o)
for t in c.get("targets", []):
    print("target\t" + t)
PY
)
[ ${#cmd[@]} -eq 0 ] || build="${cmd[*]}"
[ -n "${build}" ] || die "no build command" "set build in ${cfg}, or pass one after --"
[ ${#outputs[@]} -gt 0 ] || die "no outputs named" "set outputs (globs) in ${cfg}"
for t in ${targets[@]+"${targets[@]}"}; do
  case ${t} in
  *darwin*) die "target ${t} is a darwin target, this checks Linux targets only" "macOS stays out until codesign and ld64 are handled, take it out of targets" ;;
  *linux*) ;;
  *) die "target ${t} is not a Linux target" "this checks Linux targets only, take it out of targets" ;;
  esac
done
[ ${#targets[@]} -gt 0 ] || targets=("")

command -v git >/dev/null 2>&1 || die "git is missing" "install git"
if [ -n "${version}" ]; then
  epoch=$(bash "${here}/release-date.sh" --repo "${root}" --epoch "${version}") ||
    die "no release date for ${version}" "fetch the tags, or leave --version out"
else
  epoch=$(git -C "${root}" log -1 --format=%ct HEAD 2>/dev/null) ||
    die "no commit at HEAD in ${root}" "commit the tree first"
fi

tmp=$(mktemp -d "${TMPDIR:-/tmp}/repro-check.XXXXXX") || die "mktemp failed" "free space in ${TMPDIR:-/tmp}"
tmp=$(cd "${tmp}" && pwd -P)
cleanup() { "${PY}" -c 'import shutil, sys; shutil.rmtree(sys.argv[1], ignore_errors=True)' "${tmp}"; }
trap cleanup EXIT
copies=("${tmp}/one/tree-a" "${tmp}/second-longer-name/checkout-b")

expand() { # expand <dir> <glob>: the files the glob names below dir, sorted
  (cd "$1" && "${PY}" -c 'import glob, sys; print("\n".join(sorted(glob.glob(sys.argv[1], recursive=True))))' "$2")
}

leaks() { # leaks <file> <copy path>: the host paths inside a binary
  grep -a -o -E "(/Users/|/home/)[[:graph:]]{0,60}" "$1" 2>/dev/null | grep -v '^/home/user' | sort -u | sed -n '1,8p'
  grep -a -o -F "$2" "$1" 2>/dev/null | sort -u | sed -n '1,1p'
}

copy_tree() { # copy_tree <dest>: the product tree, .git included, without target/
  "${PY}" - "${root}" "$1" <<'PY'
import os, shutil, sys
src, dst = sys.argv[1:3]
shutil.copytree(src, dst, symlinks=True, dirs_exist_ok=True,
                ignore=lambda d, names: ["target"] if os.path.samefile(d, src) and "target" in names else [])
PY
}

build_in() { # build_in <copy> <command line>: one build, repro-env applied, in a subshell
  (
    cd "$1" || exit 1
    unset CARGO_TARGET_DIR
    eval "$(bash "${here}/repro-env.sh" --repo "$1" --root "$1" --epoch "${epoch}")" || exit 1
    bash -c "$2"
  )
}

status=0
for target in "${targets[@]}"; do
  label=${target:-"(no target)"}
  line=${build//\{target\}/${target}}
  for c in "${copies[@]}"; do
    mkdir -p "$(dirname "${c}")" || die "cannot make ${c}" "free space in ${TMPDIR:-/tmp}"
    copy_tree "${c}" || die "cannot copy the tree to ${c}" "check disk space and that ${root} is readable"
    echo "repro-check: building ${label} in ${c}" >&2
    build_in "${c}" "${line}" ||
      die "the build failed in ${c} (target ${label})" "run the build command by hand in a copy of the tree and fix it"
  done
  for o in "${outputs[@]}"; do
    pat=${o//\{target\}/${target}}
    la=$(expand "${copies[0]}" "${pat}")
    lb=$(expand "${copies[1]}" "${pat}")
    [ -n "${la}" ] || die "no output matches ${pat} after the build (target ${label})" "fix outputs in ${cfg} or the build command"
    if [ "${la}" != "${lb}" ]; then
      echo "DIFFER ${pat}: the two builds produced different files:"
      echo "  one: $(echo "${la}" | tr '\n' ' ')"
      echo "  two: $(echo "${lb}" | tr '\n' ' ')"
      status=1
      continue
    fi
    while IFS= read -r f; do
      [ -n "${f}" ] || continue
      a=${copies[0]}/${f}
      b=${copies[1]}/${f}
      if [ "$(sha "${a}")" = "$(sha "${b}")" ]; then
        echo "same   ${f}  $(sha "${a}")  (${label})"
        continue
      fi
      status=1
      echo "DIFFER ${f} (${label})"
      echo "  one: $(sha "${a}")"
      echo "  two: $(sha "${b}")"
      if command -v diffoscope >/dev/null 2>&1; then
        diffoscope --text - "${a}" "${b}" 2>&1 | sed -n '1,60p'
      else
        echo "  first difference: $(cmp "${a}" "${b}" 2>&1 | sed -e "s#${copies[0]}#<one>#g" -e "s#${copies[1]}#<two>#g")"
        for pair in "${a}:${copies[0]}" "${b}:${copies[1]}"; do
          found=$(leaks "${pair%%:*}" "${pair#*:}")
          [ -z "${found}" ] || { echo "  host paths in ${pair%%:*}:"; echo "${found}" | sed 's/^/    /'; }
        done
      fi
    done <<<"${la}"
  done
done
if [ "${status}" = 0 ]; then echo "repro-check: reproducible"; else echo "repro-check: NOT reproducible"; fi
exit "${status}"
