#!/usr/bin/env bash
# The check to run before landing a change: the same gates as `make check`, in the same order,
# but over the crates the change touches. A path it cannot map (the Makefile, scripts/, a root
# manifest, an unknown directory) widens the selection to the whole workspace and says so, so it
# never narrows on ignorance. Order: fmt, clippy -D warnings, tests, lines, names, conventions,
# lint, docs-check, then the subject of every commit. When web or index is in scope the
# equivalence oracle (equivalence_pages) runs again as its own named step. The crates have no
# cargo features, so one clippy pass covers both feature shapes.
#
#   scripts/land-check.sh [--base REV] [--full] [--explain] [--paths P...] [--messages BASE]
#                         [--self-test]
#
#   --base REV    compare against REV (default main, else HEAD~1) plus uncommitted changes
#   --full        run `make check` instead, the whole gate
#   --explain     print the selection and exit
#   --paths P...  use these paths instead of asking git
#   --messages BASE  run only the message step (subjects and the shared lints) over BASE..HEAD
set -euo pipefail
# Config passed in through the environment (GIT_CONFIG_COUNT, as a lander sets core.hooksPath)
# would override what the self-test and hook-test fixture repos configure themselves.
unset GIT_CONFIG_COUNT
cd "$(dirname "$0")/.."
VALE_INI=$PWD/.vale.ini

# The shared lints read tomllib: the first python3 that is 3.11 or newer.
py=""
for c in python3 python3.15 python3.14 python3.13 python3.12 python3.11; do
  if command -v "$c" >/dev/null && "$c" -c 'import sys; sys.exit(sys.version_info < (3, 11))'; then
    py=$c
    break
  fi
done
[[ -n $py ]] || {
  echo "land-check: needs python 3.11 or newer" >&2
  exit 1
}

# One value of schwarzwald/config/land.toml: `land_cfg oracle.crate`. A list prints one item
# per line.
land_cfg() {
  "$py" - "$1" <<'PY'
import sys, tomllib
section, key = sys.argv[1].split(".")
with open("schwarzwald/config/land.toml", "rb") as f:
    v = tomllib.load(f)[section][key]
print("\n".join(v) if isinstance(v, list) else v)
PY
}

# Refuses when core.hooksPath of repo $1 does not resolve to the hooks directory $2: a fresh
# clone would otherwise skip the commit-msg hook without a word.
hooks_wired() {
  local set top want have
  set=$(git -C "$1" config core.hooksPath || true)
  if [[ -z $set ]]; then
    echo "land-check: core.hooksPath is unset, so the commit-msg hook never runs." >&2
    echo "land-check: fix: git config core.hooksPath scripts/hooks" >&2
    return 1
  fi
  top=$(git -C "$1" rev-parse --show-toplevel)
  [[ $set == /* ]] || set="$top/$set"
  have=$(cd "$set" 2>/dev/null && pwd -P || true)
  want=$(cd "$2" 2>/dev/null && pwd -P || true)
  if [[ -z $have || $have != "$want" ]]; then
    echo "land-check: core.hooksPath is $set, not the repo's scripts/hooks." >&2
    echo "land-check: fix: git config core.hooksPath scripts/hooks" >&2
    return 1
  fi
}

# type(scope): subject, the form the history already uses.
SUBJECT='^(feat|fix|docs|style|refactor|perf|test|build|ci|chore|revert)(\([a-z0-9_-]+\))?!?: .+'

# Prints "all" or the touched crates plus everything that depends on them, one per line.
# The paths come one per line on stdin, never as arguments: a long list overflows argv.
select_crates() {
  python3 -c "$(
    cat <<'PY'
import json, subprocess, sys

paths = [p for p in sys.stdin.read().split("\n") if p]
meta = json.loads(subprocess.check_output(["cargo", "metadata", "--format-version", "1", "--no-deps"]))
root = meta["workspace_root"] + "/"
pkgs = {p["name"]: p for p in meta["packages"]}
dirs = {p["manifest_path"][len(root):].rsplit("/", 1)[0]: n for n, p in pkgs.items()}
touched, widen = set(), False
for path in paths:
    top = "/".join(path.split("/")[:2]) if path.startswith("crates/") else path.split("/")[0]
    if top in dirs:
        touched.add(dirs[top])
    elif path.endswith((".md", ".json")) and not path.startswith(("crates/", "xtask/")):
        continue
    elif path.startswith(("scripts/", "docs/", "tests/bench/", ".github/")):
        continue
    else:
        widen = True
if widen:
    print("all")
    sys.exit(0)
deps = {n: {d["name"] for d in p["dependencies"]} for n, p in pkgs.items()}
out = set(touched)
while True:
    more = {n for n, d in deps.items() if d & out} - out
    if not more:
        break
    out |= more
print("\n".join(sorted(out)))
PY
  )"
}

changed_paths() {
  {
    git diff --name-only "$1"...HEAD
    git diff --name-only HEAD
  } | sort -u
}

# Fails on any commit in BASE..HEAD whose subject is not type(scope): subject, or whose subject
# breaks the spelling and word rules CHANGELOG.md is held to: git-cliff copies subjects there,
# so a British spelling or a banned word in one fails vale on the generated file.
check_subjects() {
  local bad=0 line d
  while IFS= read -r line; do
    if [[ ! $line =~ $SUBJECT ]]; then
      echo "subject does not read type(scope): subject: $line" >&2
      bad=1
    fi
  done < <(git log --format=%s "$1"..HEAD)
  d=$(mktemp -d)
  git log --format='- %s' "$1"..HEAD >"$d/CHANGELOG.md"
  if [[ -s $d/CHANGELOG.md ]] &&
    ! (cd "$d" && vale --config="$VALE_INI" --output=line \
      --filter='.Name in ["Schwarzwald.American","Schwarzwald.NoFallback"]' CHANGELOG.md) >&2; then
    echo "a subject breaks the CHANGELOG spelling rules (vale, above)" >&2
    bad=1
  fi
  rm -r "$d"
  return "$bad"
}

# The commands `make lines names` runs, each of which this script must also run: through a
# `step make ...` line, or as a literal `step` line. A check added to those targets then reaches
# the gate by itself, and a copied command that falls behind the Makefile is caught here.
# Optional $1 is the script text to inspect (the self-test default is this file).
gate_commands_covered() {
  local script=${1:-$0} want have missing=0 line targets
  want=$(make -n lines names 2>/dev/null | grep -v '^echo ' || true)
  [[ -n $want ]] || return 1
  have=$(grep -E '^step ' "$script" | sed 's/^step //')
  targets=$(grep -E '^step make ' "$script" | sed 's/^step make //' | tr '\n' ' ')
  # shellcheck disable=SC2086
  [[ -z $targets ]] || have="$have"$'\n'$(make -n $targets 2>/dev/null)
  while IFS= read -r line; do
    grep -qxF -- "$line" <<<"$have" || missing=1
  done <<<"$want"
  ((missing == 0))
}

LINTS="$PWD/vendor/schwarzwald/kit/lint"

# The message step: the shared lints over subject and body of every commit in $1..HEAD. Each
# lint runs, so one log names every refusal; any refusal fails the step.
message_lints() {
  local rc=0 rule
  for rule in internal-refs personal-data own-shas; do
    echo "== $py vendor/schwarzwald/kit/lint/$rule.py --messages $1"
    "$py" "$LINTS/$rule.py" --messages "$1" || rc=1
  done
  return "$rc"
}

self_test() {
  local got dep dep_path dependent leaf_path
  dep=$(land_cfg selftest.dependency)
  dep_path=$(land_cfg selftest.dependency_path)
  dependent=$(land_cfg selftest.dependent)
  leaf_path=$(land_cfg selftest.leaf_path)
  got=$(printf '%s\n' "$dep_path" | select_crates | tr '\n' ' ')
  [[ $got == *"$dep"* && $got == *"$dependent"* ]] ||
    {
      echo "self-test: a git edit must reach its dependents: $got" >&2
      exit 1
    }
  got=$(printf '%s\n' "$leaf_path" | select_crates | tr '\n' ' ')
  [[ $got != *"$dep"* ]] ||
    {
      echo "self-test: a cards edit must not select git: $got" >&2
      exit 1
    }
  got=$(printf '%s\n' Makefile | select_crates)
  [[ $got == all ]] || {
    echo "self-test: an unmapped path must widen: $got" >&2
    exit 1
  }
  got=$(printf '%s\n' scripts/land-check.sh docs/api.md | select_crates | tr '\n' ' ')
  [[ -z ${got// /} ]] || {
    echo "self-test: scripts and docs select no crate: $got" >&2
    exit 1
  }
  local d
  d=$(mktemp -d)
  trap 'rm -rf "$d"' RETURN
  git -C "$d" init -q -b main
  # The hooks guard: unset is red, a path to another directory is red, the real one is green.
  mkdir "$d/hooks" "$d/other"
  if hooks_wired "$d" "$d/hooks" 2>/dev/null; then
    echo "self-test: an unset core.hooksPath passed" >&2
    exit 1
  fi
  git -C "$d" config core.hooksPath other
  if hooks_wired "$d" "$d/hooks" 2>/dev/null; then
    echo "self-test: a core.hooksPath at another directory passed" >&2
    exit 1
  fi
  git -C "$d" config core.hooksPath hooks
  hooks_wired "$d" "$d/hooks" 2>/dev/null ||
    {
      echo "self-test: a core.hooksPath at the hooks directory failed" >&2
      exit 1
    }
  git -C "$d" -c user.name=t -c user.email=t@example.invalid commit -q --allow-empty -m "feat(x): ok"
  git -C "$d" -c user.name=t -c user.email=t@example.invalid commit -q --allow-empty -m "wip stuff"
  if (cd "$d" && check_subjects HEAD~1) 2>/dev/null; then
    echo "self-test: a subject without a type passed" >&2
    exit 1
  fi
  git -C "$d" reset -q --hard HEAD~1
  git -C "$d" -c user.name=t -c user.email=t@example.invalid commit -q --allow-empty \
    -m "feat(web): graph columns take cycling theme colours"
  if (cd "$d" && check_subjects HEAD~1) 2>/dev/null; then
    echo "self-test: a subject with a British spelling passed" >&2
    exit 1
  fi
  git -C "$d" reset -q --hard HEAD~1
  git -C "$d" -c user.name=t -c user.email=t@example.invalid commit -q --allow-empty -m "wip stuff"
  # The message step, from this checkout over the scratch history: a body the shared deny list
  # refuses fails it, and the commit before it passes.
  git -C "$d" -c user.name=t -c user.email=t@example.invalid commit -q --allow-empty \
    -m "fix(x): one rule" -m "The page no longer lists ties."
  if (cd "$d" && message_lints HEAD~1) >/dev/null 2>&1; then
    echo "self-test: the message step passed a body the shared deny list refuses" >&2
    exit 1
  fi
  git -C "$d" reset -q --hard HEAD~1
  (cd "$d" && message_lints HEAD~1) >/dev/null 2>&1 ||
    {
      echo "self-test: the message step refused a plain body" >&2
      exit 1
    }
  (cd "$d" && git reset -q --hard HEAD~1 && check_subjects HEAD) ||
    {
      echo "self-test: a well formed subject failed" >&2
      exit 1
    }
  # Today's script, before it called the targets: it copied three commands and lost two.
  printf '%s\n' 'step python3 scripts/lint/width-check.py' \
    'step python3 scripts/lint/status-words-check.py' \
    'step python3 scripts/lint/names-check.py' >"$d/copied.sh"
  if gate_commands_covered "$d/copied.sh"; then
    echo "self-test: a script that copies only three commands passed" >&2
    exit 1
  fi
  gate_commands_covered ||
    {
      echo "self-test: a command of \`make lines names\` never runs in land-check" >&2
      exit 1
    }
  echo "land-check self-test: ok"
}

base=
full=0
explain=0
given=()
while (($#)); do
  case $1 in
  --base)
    base=$2
    shift 2
    ;;
  --full)
    full=1
    shift
    ;;
  --explain)
    explain=1
    shift
    ;;
  --paths)
    shift
    while (($#)) && [[ $1 != --* ]]; do
      given+=("$1")
      shift
    done
    ;;
  --self-test)
    self_test
    exit 0
    ;;
  --messages)
    hooks_wired . scripts/hooks && check_subjects "$2" && message_lints "$2"
    exit
    ;;
  *)
    echo "unknown option: $1" >&2
    exit 2
    ;;
  esac
done

hooks_wired . scripts/hooks || exit 1
if ((full)); then
  exec make check
fi

if [[ -z $base ]]; then
  base=$(git rev-parse --verify -q main || git rev-parse HEAD~1)
fi
if ((${#given[@]})); then
  paths=("${given[@]}")
else
  paths=()
  while IFS= read -r line; do paths+=("$line"); done < <(changed_paths "$base")
fi
((${#paths[@]})) || {
  echo "land-check: nothing changed against $base"
  exit 0
}

crates=$(printf '%s\n' "${paths[@]}" | select_crates)
if [[ $crates == all ]]; then
  echo "land-check: a path is not mapped to a crate, widening to the workspace" >&2
  scope=(--workspace)
elif [[ -n $crates ]]; then
  scope=()
  for c in $crates; do scope+=(-p "$c"); done
else
  scope=()
fi
echo "land-check: ${scope[*]:-no crate touched}"
((explain)) && exit 0

step() {
  echo "== $*"
  "$@"
}

step cargo fmt --all -- --check
if ((${#scope[@]})); then
  step cargo clippy "${scope[@]}" --all-targets -- -D warnings
  # The Windows target of `make clippy-cross`, scoped: test-only code that is dead there fails.
  if rustup target list --installed 2>/dev/null | grep -qx x86_64-pc-windows-gnu &&
    command -v x86_64-w64-mingw32-gcc >/dev/null; then
    step cargo clippy "${scope[@]}" --all-targets --target x86_64-pc-windows-gnu -- -D warnings
  fi
  step cargo test --no-fail-fast "${scope[@]}"
  # Rustdoc as `make doc` runs it: a link to an item that only exists under cfg(test) passes
  # clippy and the tests and fails here alone.
  step env RUSTDOCFLAGS="-D warnings" cargo doc --no-deps "${scope[@]}"
  # The output oracle: named on its own so a log shows it ran whenever the pages could move.
  run_oracle=0
  [[ " ${scope[*]} " == *" --workspace "* ]] && run_oracle=1
  while IFS= read -r t; do
    [[ " ${scope[*]} " == *" $t "* ]] && run_oracle=1
  done < <(land_cfg oracle.triggers)
  if ((run_oracle)); then
    step cargo test -p "$(land_cfg oracle.crate)" --test "$(land_cfg oracle.test)"
  fi
fi
step make lines names
step "$py" vendor/schwarzwald/kit/tools/ratchet-check.py --self-test
step "$py" vendor/schwarzwald/kit/tools/ratchet-check.py --base "$base"
step cargo test -p "$(land_cfg conventions.crate)" --test "$(land_cfg conventions.test)"
step bash vendor/schwarzwald/kit/lint/run.sh
step python3 vendor/schwarzwald/kit/lint/root-entries.py
step make docs-check
check_subjects "$base"
# Subject and body of every commit landed: the shared lints the commit-msg hook runs.
message_lints "$base"
echo "land-check: ok (a full gate is \`scripts/land-check.sh --full\`)"
