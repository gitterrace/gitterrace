#!/usr/bin/env bash
# The check to run before landing a change: the same gates as `make check`, in the same order,
# but over the crates the change touches. A path it cannot map (the Makefile, scripts/, a root
# manifest, an unknown directory) widens the selection to the whole workspace and says so, so it
# never narrows on ignorance. Order: fmt, clippy -D warnings, tests, lines, names, conventions,
# lint, docs-check, then the subject of every commit. When web or index is in scope the
# equivalence oracle (equivalence_pages) runs again as its own named step. The crates have no
# cargo features, so one clippy pass covers both feature shapes.
#
#   scripts/land-check.sh [--base REV] [--full] [--explain] [--paths P...] [--self-test]
#
#   --base REV    compare against REV (default main, else HEAD~1) plus uncommitted changes
#   --full        run `make check` instead, the whole gate
#   --explain     print the selection and exit
#   --paths P...  use these paths instead of asking git
set -euo pipefail
cd "$(dirname "$0")/.."

# type(scope): subject, the form the history already uses.
SUBJECT='^(feat|fix|docs|style|refactor|perf|test|build|ci|chore|revert)(\([a-z0-9_-]+\))?!?: .+'

# Prints "all" or the touched crates plus everything that depends on them, one per line.
select_crates() {
  python3 - "$@" <<'PY'
import json, subprocess, sys

paths = sys.argv[1:]
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
    elif path.startswith(("scripts/", "docs/", "site/", "bench/", "ci/", ".github/")):
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
}

changed_paths() {
  {
    git diff --name-only "$1"...HEAD
    git diff --name-only HEAD
    git ls-files --others --exclude-standard
  } | sort -u
}

# Fails on any commit in BASE..HEAD whose subject is not type(scope): subject.
check_subjects() {
  local bad=0 line
  while IFS= read -r line; do
    if [[ ! $line =~ $SUBJECT ]]; then
      echo "subject does not read type(scope): subject: $line" >&2
      bad=1
    fi
  done < <(git log --format=%s "$1"..HEAD)
  return "$bad"
}

self_test() {
  local got
  got=$(select_crates crates/gitterrace-git/src/repo.rs | tr '\n' ' ')
  [[ $got == *gitterrace-git* && $got == *gitterrace-web* ]] ||
    {
      echo "self-test: a git edit must reach its dependents: $got" >&2
      exit 1
    }
  got=$(select_crates crates/gitterrace-cards/src/lib.rs | tr '\n' ' ')
  [[ $got != *gitterrace-git* ]] ||
    {
      echo "self-test: a cards edit must not select git: $got" >&2
      exit 1
    }
  got=$(select_crates Makefile)
  [[ $got == all ]] || {
    echo "self-test: an unmapped path must widen: $got" >&2
    exit 1
  }
  got=$(select_crates scripts/land-check.sh docs/api.md | tr '\n' ' ')
  [[ -z ${got// /} ]] || {
    echo "self-test: scripts and docs select no crate: $got" >&2
    exit 1
  }
  local d
  d=$(mktemp -d)
  trap 'rm -rf "$d"' RETURN
  git -C "$d" init -q -b main
  git -C "$d" -c user.name=t -c user.email=t@example.invalid commit -q --allow-empty -m "feat(x): ok"
  git -C "$d" -c user.name=t -c user.email=t@example.invalid commit -q --allow-empty -m "wip stuff"
  if (cd "$d" && check_subjects HEAD~1) 2>/dev/null; then
    echo "self-test: a subject without a type passed" >&2
    exit 1
  fi
  (cd "$d" && git reset -q --hard HEAD~1 && check_subjects HEAD) ||
    {
      echo "self-test: a well formed subject failed" >&2
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
  *)
    echo "unknown option: $1" >&2
    exit 2
    ;;
  esac
done

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

crates=$(select_crates "${paths[@]}")
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
  step cargo test "${scope[@]}"
  # The output oracle: named on its own so a log shows it ran whenever the pages could move.
  if [[ " ${scope[*]} " == *" --workspace "* || " ${scope[*]} " == *" gitterrace-web "* ||
    " ${scope[*]} " == *" gitterrace-index "* ]]; then
    step cargo test -p gitterrace-web --test equivalence_pages
  fi
fi
step python3 scripts/lines-check.py
step python3 scripts/status-words-check.py
step python3 scripts/names-check.py
step cargo test -p gitterrace-testkit --test conventions
step sh scripts/lint-run.sh
step make docs-check
check_subjects "$base"
# Subject and body of every commit landed, the rules the commit-msg hook runs plus own shas.
step python3 scripts/lint/internal-refs.py --messages "$base"
step python3 scripts/lint/personal-data.py --messages "$base"
step python3 scripts/lint/own-shas.py --messages "$base"
echo "land-check: ok (a full gate is \`scripts/land-check.sh --full\`)"
