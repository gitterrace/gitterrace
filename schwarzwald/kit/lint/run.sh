#!/usr/bin/env bash
# Every external linter and every shared lint, one PASS or FAIL line each. A missing tool is a
# FAIL with its install line, never a skip. Run from the product's gate (`make lint`).
#
#   bash <this part>/run.sh [--repo DIR]
#
# The linters read their own config at the repository root (_typos.toml, .vale.ini,
# .gitleaks.toml, .markdownlint-cli2.jsonc, ruff.toml), and only run when that config or the
# files they lint exist. Product values come from schwarzwald/config/lint.toml [run]:
# steps (extra "name|tool|install hint|command" checks), shell_exclude, shellcheck_exclude,
# markdownlint (the pinned markdownlint-cli2). The part is found from this
# file's own location, so the copy in a product runs the copy beside it.
set -euo pipefail

part="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
repo=""
while (($#)); do
  case $1 in
  --repo)
    repo=${2:?--repo needs a directory}
    shift 2
    ;;
  *)
    echo "run.sh: unknown argument $1" >&2
    exit 2
    ;;
  esac
done
cd "${repo:-$(git rev-parse --show-toplevel)}"

# The shared lints and drift.py need tomllib: the newest python 3.11 or newer on PATH.
# shellcheck source=lib/pick-python.sh
. "${part}/lint/lib/pick-python.sh"
pick_py || {
  echo "FAIL lint: no python 3.11 or newer on PATH (install python3.13)"
  exit 1
}
py=${PY}

# [run] settings as lines "key<TAB>value" (lists joined by the unit separator), through tomllib.
cfg=$(
  "${py}" - <<'P'
import sys
from pathlib import Path
try:
    import tomllib
except ImportError:
    sys.exit("run.sh: python 3.11 or newer is needed for tomllib")
run = {}
p = Path("schwarzwald/config/lint.toml")
if p.exists():
    run = tomllib.loads(p.read_text(encoding="utf-8")).get("run", {})
d = {"steps": [], "shell_exclude": [], "shellcheck_exclude": ["SC2250", "SC2292", "SC2310", "SC2312"],
     "markdownlint": ""}
d.update(run)
for k, v in d.items():
    print(k + "\t" + ("\x1f".join(map(str, v)) if isinstance(v, list) else str(v)))
P
)
get() { printf '%s\n' "${cfg}" | awk -F'\t' -v k="$1" '$1 == k { print $2 }'; }
IFS=$'\x1f' read -r -a steps <<<"$(get steps)"
IFS=$'\x1f' read -r -a shell_exclude <<<"$(get shell_exclude)"
sc_exclude=$(get shellcheck_exclude)
sc_exclude=${sc_exclude//$'\x1f'/,}
indent=2
mdl=$(get markdownlint)
# npx --yes installs any npm spec (a URL, npm:other) with no prompt: only a version runs.
if [[ -n ${mdl} && ! ${mdl} =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "FAIL markdownlint: [run] markdownlint must be a version like 0.18.1, not ${mdl}"
  exit 1
fi

fails=0
# run NAME TOOL INSTALL CMD...: run CMD when TOOL exists, report the verdict.
run() {
  local name=$1 tool=$2 install=$3 out
  shift 3
  if ! command -v "${tool}" >/dev/null 2>&1; then
    echo "FAIL ${name}: ${tool} is not installed (${install})"
    fails=$((fails + 1))
    return
  fi
  if out=$("$@" 2>&1); then
    echo "PASS ${name}"
  else
    printf '%s\n' "${out}"
    echo "FAIL ${name}"
    fails=$((fails + 1))
  fi
}

# Tracked shell scripts, minus the [run] shell_exclude pathspecs (the synced copy carries its
# own checks, so a product lists it there).
sh_files() {
  local spec=()
  local e
  for e in ${shell_exclude[@]+"${shell_exclude[@]}"}; do [[ -n ${e} ]] && spec+=(":!:${e}"); done
  git ls-files -z -- '*.sh' ${spec[@]+"${spec[@]}"} | xargs -0 "$@"
}
# Vale needs its packages in styles/ (git-ignored), fetched once.
vale_run() {
  [[ -d styles ]] && [[ -z $(find styles -mindepth 1 -maxdepth 1 -type d ! -name config 2>/dev/null | head -1) ]] && { vale sync >/dev/null || return 1; }
  git ls-files -z -- '*.md' | xargs -0 vale --minAlertLevel=error
}

# A check that cannot run says so. A missing config is a FAIL where every product needs the
# check (typos, gitleaks, ruff on python), else a SKIP, which CI=true turns into a FAIL.
noconf() {
  echo "FAIL $1: no config ($2), so it would not run"
  fails=$((fails + 1))
}
skip() {
  if [[ ${CI:-} == true ]]; then
    echo "FAIL $1: $2 (a skip fails under CI)"
    fails=$((fails + 1))
  else echo "SKIP $1: $2"; fi
}
has_md=$(git ls-files -- '*.md' | head -1)
if [[ -f _typos.toml || -f .typos.toml ]]; then run typos typos "brew install typos-cli" typos; else noconf typos _typos.toml; fi
if [[ -f .vale.ini ]]; then
  run vale vale "brew install vale" vale_run
elif [[ -n ${has_md} ]]; then skip vale "markdown is tracked but .vale.ini is missing"; fi
if [[ -f .gitleaks.toml ]]; then
  run gitleaks gitleaks "brew install gitleaks" gitleaks detect --no-banner -c .gitleaks.toml
else noconf gitleaks .gitleaks.toml; fi
md_files() { git ls-files -z -- '*.md' | xargs -0 -I{} printf './%s\0' {} | xargs -0 "$@"; }
if [[ -n ${mdl} && -n ${has_md} ]]; then
  run markdownlint npx "install node, then npx fetches markdownlint-cli2" md_files npx --yes "markdownlint-cli2@${mdl}" --
elif [[ -n ${has_md} ]]; then skip markdownlint "markdown is tracked but [run] markdownlint pins no version"; fi
if [[ -n $(git ls-files -- '*.sh' | head -1) ]]; then
  run shellcheck shellcheck "brew install shellcheck" sh_files shellcheck -o all -e "${sc_exclude}"
  run shfmt shfmt "brew install shfmt" sh_files shfmt -d -i "${indent}"
fi
if [[ -n $(git ls-files -- '*.py' | head -1) ]]; then
  if [[ -f ruff.toml || -f .ruff.toml || -f pyproject.toml ]]; then
    run ruff-check ruff "brew install ruff" ruff check .
    run ruff-format ruff "brew install ruff" ruff format --check .
  else noconf ruff ruff.toml; fi
fi
[[ -d .github/workflows ]] && run actionlint actionlint "brew install actionlint" actionlint

# Drift: the tool configs keep to the shared bases.
[[ -f ${part}/configs/drift.py ]] && run drift "${py}" "install python 3.11" "${py}" "${part}/configs/drift.py" --repo .

# The shared lints, each proved on its red case by red.py.
lint="${part}/lint"
for rule in internal-refs personal-data denylist-parked own-shas actions-pin dash-standins make-targets file-length script-orphans docs-a11y changelog commit-dates; do
  [[ -f ${lint}/${rule}.py ]] && run "${rule}" "${py}" "install python 3.11" "${py}" "${lint}/${rule}.py" --repo .
done
[[ -f ${lint}/red.py ]] && run lint-red "${py}" "install python 3.11" "${py}" "${lint}/red.py" --repo .
[[ -f ${lint}/run_test.sh ]] && run run-drift-step bash "install bash" bash "${lint}/run_test.sh"
[[ -f ${part}/hooks/commit-msg_test.sh ]] && run commit-msg-hook bash "install bash" bash "${part}/hooks/commit-msg_test.sh"
for t in author-guard stale-index markdown-staged commit-dates; do
  [[ -f ${part}/hooks/${t}_test.sh ]] && run "${t}-hook" bash "install bash" bash "${part}/hooks/${t}_test.sh"
done
[[ -f ${part}/guards/tests/suite.sh ]] && run guards bash "install bash" bash "${part}/guards/tests/suite.sh"

# Product steps: "name|tool|install hint|command". A step's command sees PY, the python 3.11 or
# newer this runner picked, so `$PY scripts/x.py` never falls back to an older plain python3.
export PY=${py}
for s in ${steps[@]+"${steps[@]}"}; do
  [[ -n ${s} ]] || continue
  IFS='|' read -r name tool hint cmd <<<"${s}"
  run "${name}" "${tool}" "${hint}" bash -c "${cmd}"
done

[[ ${fails} -eq 0 ]] || {
  echo "lint: ${fails} failed"
  exit 1
}
echo "lint: all passed"
